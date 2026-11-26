package app.nearvolt

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONException
import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * Updates straight from GitHub Releases, only when the user asks. The release
 * workflow publishes signed APKs and update.json; the APK must match its
 * SHA-256, and Android itself refuses any APK not signed with the same key as
 * the installed app. On Android 12 and later the update installs without a
 * prompt once the user has allowed Nearvolt to install apps.
 */
class Updater(private val activity: MainActivity, private val emit: (PlatformEvent) -> Unit) {
    private val context: Context = activity.applicationContext

    suspend fun check(): UpdateInfo? = withContext(Dispatchers.IO) {
        val manifest = try {
            JSONObject(open(MANIFEST_URL).inputStream.bufferedReader().use { it.readText() })
        } catch (_: IOException) {
            throw FlutterError("offline", "Couldn't reach GitHub. Check your internet connection.")
        } catch (_: JSONException) {
            throw FlutterError("damaged", "The update information is damaged. Try again later.")
        }
        if (manifest.getLong("code") <= installedCode()) return@withContext null
        val apks = manifest.getJSONObject("apks")
        val abi = Build.SUPPORTED_ABIS.firstOrNull(apks::has) ?: return@withContext null
        val apk = apks.getJSONObject(abi)
        UpdateInfo(manifest.getString("version"), manifest.optString("notes"), apk.getString("url"), apk.getString("sha256"))
    }

    suspend fun install(update: UpdateInfo) {
        requireIdle()
        if (!context.packageManager.canRequestPackageInstalls()) {
            activity.settings.launch(
                Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}")),
            )
            if (!context.packageManager.canRequestPackageInstalls()) {
                throw FlutterError("permission", "Nearvolt needs permission to install its updates.")
            }
        }
        val apk = try {
            withContext(Dispatchers.IO) { download(update) }
        } catch (_: IOException) {
            throw FlutterError("offline", "The download stopped. Check your internet connection and try again.")
        }
        requireIdle()
        withContext(Dispatchers.IO) { commit(apk) }
    }

    /** Installing restarts the app, so it never happens during a transfer. */
    private fun requireIdle() {
        if (TransferService.active) throw FlutterError("busy", "Finish the transfer first, then check again.")
    }

    private fun download(update: UpdateInfo): File {
        val file = File(context.cacheDir, "update.apk")
        val digest = MessageDigest.getInstance("SHA-256")
        val connection = open(update.url)
        val total = connection.contentLengthLong.takeIf { it > 0 }
        connection.inputStream.use { input ->
            file.outputStream().use { output ->
                val buffer = ByteArray(1 shl 16)
                var done = 0L
                var reported = -1
                while (true) {
                    val n = input.read(buffer)
                    if (n < 0) break
                    output.write(buffer, 0, n)
                    digest.update(buffer, 0, n)
                    done += n
                    val percent = total?.let { (done * 100 / it).toInt() } ?: 0
                    if (percent != reported) {
                        reported = percent
                        emit(PlatformEvent(PlatformEventKind.UPDATE_PROGRESS, progress = percent / 100.0))
                    }
                }
            }
        }
        val actual = digest.digest().joinToString("") { "%02x".format(it) }
        if (!actual.equals(update.sha256, ignoreCase = true)) {
            file.delete()
            throw FlutterError("checksum", "The download was damaged. Try again.")
        }
        return file
    }

    private fun commit(apk: File) {
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(context.packageName)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
            }
        }
        val id = installer.createSession(params)
        installer.openSession(id).use { session ->
            session.openWrite("nearvolt.apk", 0, apk.length()).use { out ->
                apk.inputStream().use { it.copyTo(out) }
                session.fsync(out)
            }
            val result = PendingIntent.getBroadcast(
                context,
                id,
                Intent(context, UpdateReceiver::class.java),
                PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            session.commit(result.intentSender)
        }
    }

    private fun installedCode(): Long =
        context.packageManager.getPackageInfo(context.packageName, 0).longVersionCode

    private fun open(url: String): HttpURLConnection =
        (URL(url).openConnection() as HttpURLConnection).apply {
            connectTimeout = 15_000
            readTimeout = 30_000
            instanceFollowRedirects = true
        }

    companion object {
        const val MANIFEST_URL = "https://github.com/vishalpa8/Nearvolt/releases/latest/download/update.json"
    }
}

/** Shows Android's install confirmation when the session needs the user. */
class UpdateReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, -1) == PackageInstaller.STATUS_PENDING_USER_ACTION) {
            intent.parcelable<Intent>(Intent.EXTRA_INTENT)
                ?.let { context.startActivity(it.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
        }
    }
}
