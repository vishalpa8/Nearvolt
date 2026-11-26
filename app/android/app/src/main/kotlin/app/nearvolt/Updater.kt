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
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * Updates straight from GitHub Releases. The release workflow publishes signed
 * APKs and update.json; the APK must match its SHA-256, and Android itself
 * refuses any APK not signed with the same key as the installed app.
 */
class Updater(private val context: Context, private val emit: (PlatformEvent) -> Unit) {
    suspend fun check(): UpdateInfo? = withContext(Dispatchers.IO) {
        val manifest = JSONObject(open(MANIFEST_URL).inputStream.bufferedReader().use { it.readText() })
        val version = manifest.getString("version")
        if (!isNewer(version, currentVersion())) return@withContext null
        val apks = manifest.getJSONObject("apks")
        val abi = Build.SUPPORTED_ABIS.firstOrNull(apks::has) ?: return@withContext null
        val apk = apks.getJSONObject(abi)
        UpdateInfo(version, manifest.optString("notes"), apk.getString("url"), apk.getString("sha256"))
    }

    suspend fun install(update: UpdateInfo) {
        if (!context.packageManager.canRequestPackageInstalls()) {
            context.startActivity(
                Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}"))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
            throw FlutterError("permission", "Allow Nearvolt to install updates, then tap Update again.")
        }
        val apk = withContext(Dispatchers.IO) { download(update) }
        withContext(Dispatchers.IO) { commit(apk) }
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

    private fun currentVersion(): String =
        context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "0"

    private fun open(url: String): HttpURLConnection =
        (URL(url).openConnection() as HttpURLConnection).apply {
            connectTimeout = 15_000
            readTimeout = 30_000
            instanceFollowRedirects = true
        }

    companion object {
        const val MANIFEST_URL = "https://github.com/vishalpa8/Nearvolt/releases/latest/download/update.json"

        /** Compares dotted version numbers, ignoring anything after `+`. */
        fun isNewer(candidate: String, current: String): Boolean {
            fun parts(v: String) = v.substringBefore('+').split('.').map { it.toIntOrNull() ?: 0 }
            val (a, b) = parts(candidate) to parts(current)
            for (i in 0 until maxOf(a.size, b.size)) {
                val x = a.getOrElse(i) { 0 }
                val y = b.getOrElse(i) { 0 }
                if (x != y) return x > y
            }
            return false
        }
    }
}

/** Shows Android's install confirmation when the session needs the user. */
class UpdateReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, -1) == PackageInstaller.STATUS_PENDING_USER_ACTION) {
            val confirm = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
            } else {
                @Suppress("DEPRECATION")
                intent.getParcelableExtra(Intent.EXTRA_INTENT)
            }
            confirm?.let { context.startActivity(it.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
        }
    }
}
