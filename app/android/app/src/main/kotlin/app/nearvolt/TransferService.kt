package app.nearvolt

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager

/**
 * Keeps a transfer running with the screen off or the app in the background:
 * a connected-device foreground service holding a wake lock and a low-latency
 * Wi-Fi lock, with progress in its notification.
 */
class TransferService : Service() {
    private var wakeLock: PowerManager.WakeLock? = null
    private var wifiLock: WifiManager.WifiLock? = null
    private val handler = Handler(Looper.getMainLooper())

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val title = intent?.getStringExtra(EXTRA_TITLE) ?: getString(R.string.app_name)
        // Android crashes the app if a service started this way stops before
        // calling startForeground, so it always does, even when already told to stop.
        try {
            startForeground(NOTIFICATION_ID, notification(this, title, null, -1), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        } catch (e: Exception) {
            // Refused by this Android version or device; the transfer runs without the service.
            AppLog.error("service", e)
            stopSelf()
            return START_NOT_STICKY
        }
        started = true
        if (!active) {
            stopSelf()
            return START_NOT_STICKY
        }
        instance = this
        return START_NOT_STICKY
    }

    /**
     * Keeps the CPU awake and Wi-Fi at full speed while data is moving, and lets
     * both go 15 seconds after it stops. An open but idle connection then costs
     * only the small foreground notification, not a held wake lock.
     */
    private fun hold() {
        if (wakeLock == null) {
            wakeLock = getSystemService(PowerManager::class.java).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "nearvolt:transfer")
            wifiLock = getSystemService(WifiManager::class.java)
                .createWifiLock(WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "nearvolt:transfer")
        }
        wakeLock?.acquire(IDLE_RELEASE_MS * 2)
        if (wifiLock?.isHeld == false) wifiLock?.acquire()
        handler.removeCallbacks(release)
        handler.postDelayed(release, IDLE_RELEASE_MS)
    }

    private val release = Runnable { releaseLocks() }

    private fun releaseLocks() {
        wakeLock?.takeIf { it.isHeld }?.release()
        wifiLock?.takeIf { it.isHeld }?.release()
    }

    override fun onDestroy() {
        started = false
        instance = null
        handler.removeCallbacks(release)
        releaseLocks()
        super.onDestroy()
    }

    companion object {
        private const val CHANNEL = "transfers"
        private const val NOTIFICATION_ID = 1
        private const val EXTRA_TITLE = "title"
        private const val NO_BAR = -2

        /** Locks are let go this long after the last progress update. */
        private const val IDLE_RELEASE_MS = 15_000L

        @Volatile
        private var title = ""

        @Volatile
        private var instance: TransferService? = null

        /** True while a transfer runs; updates wait until it ends. */
        @Volatile
        var active = false
            private set

        /** The service has reached the foreground; until then it stops itself instead. */
        @Volatile
        private var started = false

        fun start(context: Context, title: String) {
            this.title = title
            active = true
            val manager = context.getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL, context.getString(R.string.transfer_channel), NotificationManager.IMPORTANCE_LOW),
            )
            try {
                context.startForegroundService(Intent(context, TransferService::class.java).putExtra(EXTRA_TITLE, title))
            } catch (e: IllegalStateException) {
                // Not allowed from the background; the transfer runs without the service.
                AppLog.error("service", e)
            }
        }

        /**
         * Refreshes the notification with what the app says is going on. A percent of 0 to 100
         * is a bar and means data is moving, so the locks are held; -1 is a bar that waits
         * (reconnecting); -2 is no bar, for a connection with nothing moving.
         */
        fun update(context: Context, title: String, text: String, percent: Int) {
            if (!active) return
            this.title = title
            context.getSystemService(NotificationManager::class.java)
                .notify(NOTIFICATION_ID, notification(context, title, text, percent))
            if (percent >= 0) instance?.hold()
        }

        fun stop(context: Context) {
            active = false
            if (started) context.stopService(Intent(context, TransferService::class.java))
        }

        private fun notification(context: Context, title: String, text: String?, percent: Int): Notification {
            val open = PendingIntent.getActivity(
                context,
                0,
                Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE,
            )
            return Notification.Builder(context, CHANNEL)
                .setSmallIcon(android.R.drawable.stat_sys_upload)
                .setContentTitle(title)
                .setContentText(text)
                .apply { if (percent != NO_BAR) setProgress(100, percent.coerceAtLeast(0), percent < 0) }
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setContentIntent(open)
                .build()
        }
    }
}
