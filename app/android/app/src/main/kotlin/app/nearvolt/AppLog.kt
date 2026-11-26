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

/** Starts the log before anything else in the app runs, so crashes anywhere are recorded. */
class NearvoltApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        AppLog.start(this)
    }
}

/**
 * One plain-text log, Download/Nearvolt/nearvolt-log.txt, that users can open
 * and share when something goes wrong: crashes, update steps and errors from
 * every layer. It holds no file contents and no codes.
 */
object AppLog {
    private const val MAX_BYTES = 1_000_000
    private val time = SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US)
    private var file: File? = null

    fun start(context: Context) {
        file = pick(File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "Nearvolt"))
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

    @Synchronized
    fun write(tag: String, message: String) {
        Log.i("Nearvolt", "[$tag] $message")
        runCatching { file?.appendText("${time.format(Date())} [$tag] $message\n") }
    }

    fun error(tag: String, error: Throwable) = write(tag, Log.getStackTraceString(error))

    /** Starts over past 1 MB. A log left by an earlier install can't be written, so it gets a new name. */
    private fun pick(dir: File): File? = runCatching {
        dir.mkdirs()
        val log = File(dir, "nearvolt-log.txt")
        if (log.exists() && log.canWrite() && log.length() > MAX_BYTES) log.delete()
        if (!log.exists() || log.canWrite()) log else File(dir, "nearvolt-log-${System.currentTimeMillis()}.txt")
    }.getOrNull()
}
