package app.nearvolt

import android.app.Application
import android.content.Context
import android.os.Build
import android.os.Environment
import android.os.Looper
import android.util.Log
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/** Starts the log before anything else in the app runs, so crashes anywhere are recorded. */
class NearvoltApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        AppLog.start(this)
    }
}

/**
 * One plain-text log, Download/Nearvolt/nearvolt.log, that users can open and
 * share when something goes wrong: crashes, connections, update steps and
 * errors from every layer. It holds no file contents, no file names, no codes
 * and no full IP addresses, and keeps only the last week, under 1 MB. Writing
 * happens on a background thread, so logging never slows the screen.
 */
object AppLog {
    private const val MAX_BYTES = 1_000_000L
    private const val KEEP_BYTES = 500_000
    private val DAY = TimeUnit.DAYS.toMillis(1)
    private val KEEP_FOR = 7 * DAY
    private val time = SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US)
    private val entryStart = Regex("(?m)^(?=\\d{4}-\\d{2}-\\d{2} )")
    private var file: File? = null
    private val writer = Executors.newSingleThreadExecutor { r -> Thread(r, "nearvolt-log").apply { isDaemon = true } }
    private val ipv4 = Regex("""\b(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})\b""")
    private val path = Regex("""(?<![\w.])(?:/[\w.\-@ ]+){2,}|[A-Za-z]:\\[^\s,;)]+""")
    private val media = Regex(
        """[^\s/\\'"()]+\.(?:jpe?g|png|gif|webp|heic|mp4|mov|mkv|avi|mp3|m4a|wav|pdf|docx?|xlsx?|pptx?|txt|zip|apk)\b""",
        RegexOption.IGNORE_CASE,
    )

    /** Leaves out what could identify a person or their files: full IP addresses, paths and file names. */
    fun redact(message: String): String =
        message
            .replace(ipv4) { m ->
                // A Wi-Fi Direct group address is not personal and says how the phones were linked.
                if (m.groupValues[1] == "192" && m.groupValues[2] == "168" && m.groupValues[3] == "49") {
                    "192.168.49.x"
                } else {
                    "${m.groupValues[1]}.${m.groupValues[2]}.x.x"
                }
            }
            .replace(path, "<path>")
            .replace(media, "<file>")

    fun start(context: Context) {
        file = runCatching { open(File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "Nearvolt")) }
            .getOrNull()
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, error ->
            write("crash", "on ${thread.name}: ${Log.getStackTraceString(error)}")
            // A background thread just ends. Only the main thread can't go on, so
            // only an error there closes the app.
            if (thread == Looper.getMainLooper().thread) previous?.uncaughtException(thread, error)
        }
        val app = context.packageManager.getPackageInfo(context.packageName, 0)
        write(
            "start",
            "Nearvolt ${app.versionName} (${app.longVersionCode}) on ${Build.MANUFACTURER} ${Build.MODEL}, " +
                "Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT}), ${Build.SUPPORTED_ABIS.joinToString()}",
        )
    }

    fun write(tag: String, message: String) {
        val clean = redact(message)
        Log.i("Nearvolt", "[$tag] $clean")
        val stamp = Date()
        // A crash on the main thread ends the app right after, so it can't wait in a queue.
        if (tag == "crash") append(tag, clean, stamp) else writer.execute { append(tag, clean, stamp) }
    }

    @Synchronized
    private fun append(tag: String, message: String, stamp: Date) {
        val log = file ?: return
        runCatching {
            log.appendText("${time.format(stamp)} [$tag] $message\n")
            if (log.length() > MAX_BYTES) trim(log)
        }
    }

    fun error(tag: String, error: Throwable) = write(tag, Log.getStackTraceString(error))

    /**
     * Android ties a file in Downloads to the install that created it, so a log
     * left by an uninstalled copy of the app can be neither written nor deleted.
     * This install keeps to the first name that's free or already its own:
     * nearvolt.log, then nearvolt-2.log and so on. Logs from older versions of
     * the app that it can delete are removed.
     */
    private fun open(dir: File): File {
        dir.mkdirs()
        dir.listFiles { f -> f.name.startsWith("nearvolt-log") && f.name.endsWith(".txt") }
            ?.forEach { if (it.canWrite()) it.delete() }
        val log = generateSequence(1) { it + 1 }
            .map { File(dir, if (it == 1) "nearvolt.log" else "nearvolt-$it.log") }
            .first { !it.exists() || it.canWrite() }
        // A day's slack, so the log isn't rewritten at every start once it's a week old.
        if (log.exists() && (log.length() > MAX_BYTES || (oldest(log) ?: "") < cutoff(KEEP_FOR + DAY))) trim(log)
        return log
    }

    private fun cutoff(age: Long = KEEP_FOR): String = time.format(Date(System.currentTimeMillis() - age))

    private fun oldest(log: File): String? = log.bufferedReader().use { it.readLine() }?.take(23)

    /** Keeps entries from the last week, and only the newest half megabyte of them. */
    private fun trim(log: File) {
        val cutoff = cutoff()
        var size = 0
        val kept = log.readText().split(entryStart)
            .filter { it.take(23) >= cutoff }
            .asReversed()
            .takeWhile { size += it.length; size <= KEEP_BYTES }
            .asReversed()
        log.writeText(kept.joinToString(""))
    }
}
