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
import android.os.IBinder
import android.os.PowerManager

/**
 * Keeps a transfer running with the screen off or the app in the background:
 * a connected-device foreground service holding a wake lock and a low-latency
 * Wi-Fi lock, with progress in its notification.
 */
class TransferService : Service() {
    private var wakeLock: PowerManager.WakeLock? = null
    private var wifiLock: WifiManager.WifiLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val title = intent?.getStringExtra(EXTRA_TITLE) ?: getString(R.string.app_name)
        startForeground(NOTIFICATION_ID, notification(this, title, null, 0), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        if (wakeLock == null) {
            wakeLock = getSystemService(PowerManager::class.java)
                .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "nearvolt:transfer")
                .apply { acquire(MAX_HOLD_MS) }
            wifiLock = getSystemService(WifiManager::class.java)
                .createWifiLock(WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "nearvolt:transfer")
                .apply { acquire() }
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        wakeLock?.takeIf { it.isHeld }?.release()
        wifiLock?.takeIf { it.isHeld }?.release()
        super.onDestroy()
    }

    companion object {
        private const val CHANNEL = "transfers"
        private const val NOTIFICATION_ID = 1
        private const val EXTRA_TITLE = "title"

        /** Upper bound so a lock can never outlive a stuck transfer by days. */
        private const val MAX_HOLD_MS = 24L * 60 * 60 * 1000

        @Volatile
        private var title = ""

        fun start(context: Context, title: String) {
            this.title = title
            val manager = context.getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL, context.getString(R.string.transfer_channel), NotificationManager.IMPORTANCE_LOW),
            )
            context.startForegroundService(Intent(context, TransferService::class.java).putExtra(EXTRA_TITLE, title))
        }

        fun update(context: Context, text: String, percent: Int) {
            context.getSystemService(NotificationManager::class.java)
                .notify(NOTIFICATION_ID, notification(context, title, text, percent))
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, TransferService::class.java))
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
                .setProgress(100, percent, text == null)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setContentIntent(open)
                .build()
        }
    }
}
