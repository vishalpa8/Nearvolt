package app.nearvolt

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import org.json.JSONException
import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap

/**
 * Updates straight from GitHub Releases, only when the user asks. The release
 * workflow publishes signed APKs and update.json; the APK must match its
 * SHA-256, and Android itself refuses any APK not signed with the same key as
 * the installed app. On Android 12 and later the update installs without a
 * prompt once the user has allowed Nearvolt to install apps.
 */
class Updater(private val activity: MainActivity, private val emit: (PlatformEvent) -> Unit) {
    private val context: Context = activity.applicationContext
    private val downloads = File(context.cacheDir, "updates")

    init {
        cleanUp()
    }

    suspend fun check(): UpdateInfo? = withContext(Dispatchers.IO) {
        val manifest = try {
            JSONObject(open(MANIFEST_URL).inputStream.bufferedReader().use { it.readText() })
        } catch (e: IOException) {
            AppLog.error("update", e)
            throw FlutterError("offline", "Couldn't reach GitHub. Check your internet connection.")
        } catch (e: JSONException) {
            AppLog.error("update", e)
            throw FlutterError("damaged", "The update information is damaged. Try again later.")
        }
        AppLog.write("update", "latest ${manifest.optString("version")} (${manifest.optLong("code")}), installed ${installedCode()}")
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
                AppLog.write("update", "install permission refused")
                throw FlutterError("permission", "Nearvolt needs permission to install its updates.")
            }
        }
        val apk = withContext(Dispatchers.IO) { downloaded(update) } ?: try {
            AppLog.write("update", "downloading ${update.url}")
            withContext(Dispatchers.IO) { download(update) }
        } catch (e: IOException) {
            AppLog.error("update", e)
            throw FlutterError("offline", "The download stopped. Check your internet connection and try again.")
        }
        emit(PlatformEvent(PlatformEventKind.UPDATE_PROGRESS, progress = 1.0))
        requireIdle()
        val answer = withContext(Dispatchers.IO) { commit(apk) }
        // Success ends this process, so only a failure or a lost answer gets past here.
        val (status, message) = withTimeoutOrNull(ANSWER_WAIT) { answer.await() }
            ?: throw FlutterError("install", "The install didn't finish. Check for updates to try again.")
        if (status != PackageInstaller.STATUS_SUCCESS) throw FlutterError("install", explain(status, message))
    }

    /** Installing restarts the app, so it never happens during a transfer. */
    private fun requireIdle() {
        if (TransferService.active) throw FlutterError("busy", "Disconnect first, then check again.")
    }

    private fun explain(status: Int, message: String?): String = when {
        message?.contains("unknown source", ignoreCase = true) == true ->
            if (Build.MANUFACTURER.equals("samsung", ignoreCase = true)) {
                "Samsung's Auto Blocker stopped the update. Turn it off in Settings › Security and privacy › " +
                    "Auto Blocker, then check for updates again. You can turn it back on afterwards."
            } else {
                "This phone only allows updates from its app store. Allow other sources in Settings, then check again."
            }
        status == PackageInstaller.STATUS_FAILURE_ABORTED -> "The update was cancelled."
        status == PackageInstaller.STATUS_FAILURE_STORAGE -> "There isn't enough free space to install the update."
        status == PackageInstaller.STATUS_FAILURE_CONFLICT || status == PackageInstaller.STATUS_FAILURE_INCOMPATIBLE ->
            "This update can't replace the installed app. Uninstall Nearvolt, then install the latest version from GitHub."
        else -> "The update didn't install: ${message ?: "status $status"}"
    }

    /** The APK for `update` if an earlier attempt already downloaded and checked it. */
    private fun downloaded(update: UpdateInfo): File? {
        val file = File(downloads, "nearvolt-${update.version}.apk")
        if (!file.exists()) return null
        if (runCatching { sha256(file) }.getOrNull().equals(update.sha256, ignoreCase = true)) {
            AppLog.write("update", "using the ${update.version} download from earlier")
            return file
        }
        file.delete()
        return null
    }

    /**
     * Keeps only downloads newer than the installed app, so an APK is removed once
     * it's installed. Also removes the single file older versions used.
     */
    private fun cleanUp() = runCatching {
        File(context.cacheDir, "update.apk").delete()
        val installed = installedCode()
        downloads.listFiles()?.forEach { apk ->
            val code = context.packageManager.getPackageArchiveInfo(apk.path, 0)?.longVersionCode
            if (code == null || code <= installed) apk.delete()
        }
    }

    private fun sha256(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buffer = ByteArray(1 shl 16)
            while (true) {
                val n = input.read(buffer)
                if (n < 0) break
                digest.update(buffer, 0, n)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    /** Downloads into a part file, so only a complete, checked APK is ever kept. */
    private fun download(update: UpdateInfo): File {
        downloads.mkdirs()
        downloads.listFiles()?.forEach { it.delete() }
        val target = File(downloads, "nearvolt-${update.version}.apk")
        val file = File(downloads, "${target.name}.part")
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
            AppLog.write("update", "checksum mismatch: got $actual, expected ${update.sha256}")
            file.delete()
            throw FlutterError("checksum", "The download was damaged. Try again.")
        }
        AppLog.write("update", "downloaded ${file.length()} bytes, checksum matches")
        if (!file.renameTo(target)) throw IOException("couldn't keep the download")
        return target
    }

    /** Hands the APK to Android; the result completes with Android's final answer. */
    private fun commit(apk: File): CompletableDeferred<Pair<Int, String?>> {
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(context.packageName)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
            }
        }
        val id = installer.createSession(params)
        val answer = UpdateReceiver.expect(id)
        // Android closes the app to replace it; this tells the app that comes back it
        // was our own update, so it opens again by itself.
        UpdatedReceiver.expectUpdate(context)
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
            AppLog.write("update", "install session $id committed")
        }
        return answer
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

        /** Long enough for the user to answer Android's confirmation, if it shows one. */
        private const val ANSWER_WAIT = 5 * 60_000L
    }
}

/**
 * Android's answer to an install: shows its confirmation when the session needs
 * the user, and hands the final result to the [Updater] waiting for it.
 */
class UpdateReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
        val message = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)
        val id = intent.getIntExtra(PackageInstaller.EXTRA_SESSION_ID, -1)
        AppLog.write("update", "install status $status: $message")
        if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
            intent.parcelable<Intent>(Intent.EXTRA_INTENT)?.let {
                runCatching { context.startActivity(it.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
                    .onFailure { e -> AppLog.error("update", e) }
            }
            return
        }
        // Not installed: the app stays, so there's nothing to reopen.
        if (status != PackageInstaller.STATUS_SUCCESS) UpdatedReceiver.forgetUpdate(context)
        waiting.remove(id)?.complete(status to message)
    }

    companion object {
        private val waiting = ConcurrentHashMap<Int, CompletableDeferred<Pair<Int, String?>>>()

        /** Call before committing session `id`, so even an instant answer is caught. */
        fun expect(id: Int): CompletableDeferred<Pair<Int, String?>> = CompletableDeferred<Pair<Int, String?>>().also {
            waiting[id] = it
        }
    }
}

/**
 * Android closes the app to replace it. When that was our own update, this opens
 * the new version again. Android may refuse to open a screen from the background,
 * so a one-tap notice is posted too; it goes away once the app is open.
 */
class UpdatedReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_MY_PACKAGE_REPLACED) return
        val prefs = context.getSharedPreferences("nearvolt", Context.MODE_PRIVATE)
        val asked = prefs.getLong(UPDATING, 0)
        forgetUpdate(context)
        // Updated some other way (an app store, adb): leave the user alone.
        if (System.currentTimeMillis() - asked > EXPECT_FOR_MS) return
        val version = context.packageManager.getPackageInfo(context.packageName, 0).versionName
        AppLog.write("update", "updated to $version; opening the app again")
        val open = Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        runCatching { context.startActivity(open) }.onFailure { AppLog.error("update", it) }
        runCatching {
            val manager = context.getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(NotificationChannel(CHANNEL, "App updates", NotificationManager.IMPORTANCE_DEFAULT))
            val tap = PendingIntent.getActivity(context, 0, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            manager.notify(
                NOTIFICATION_ID,
                Notification.Builder(context, CHANNEL)
                    .setSmallIcon(android.R.drawable.stat_sys_download_done)
                    .setContentTitle("Nearvolt updated to $version")
                    .setContentText("Tap to open")
                    .setAutoCancel(true)
                    .setContentIntent(tap)
                    .build(),
            )
        }.onFailure { AppLog.error("update", it) }
    }

    companion object {
        private const val UPDATING = "updating_since"
        private const val CHANNEL = "updates"
        private const val NOTIFICATION_ID = 2

        /** An update takes seconds; anything much later wasn't ours. */
        private const val EXPECT_FOR_MS = 15 * 60_000L

        fun expectUpdate(context: Context) {
            context.getSharedPreferences("nearvolt", Context.MODE_PRIVATE).edit()
                .putLong(UPDATING, System.currentTimeMillis()).apply()
        }

        fun forgetUpdate(context: Context) {
            context.getSharedPreferences("nearvolt", Context.MODE_PRIVATE).edit().remove(UPDATING).apply()
        }

        fun dismiss(context: Context) {
            context.getSystemService(NotificationManager::class.java).cancel(NOTIFICATION_ID)
        }
    }
}
