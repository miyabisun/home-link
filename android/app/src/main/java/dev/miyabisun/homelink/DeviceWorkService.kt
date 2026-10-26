package dev.miyabisun.homelink

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import android.util.Log

/**
 * Keeps the app's process in the foreground while [DeviceWork] runs a job, so leaving the screen or turning it off
 * does not freeze the process or cut its network before home-link answers: a registration holds the BLE proxy open
 * and waits up to six minutes, as does a Wi-Fi move. The job itself runs in [DeviceWork].
 */
class DeviceWorkService : Service() {
    companion object {
        private const val CHANNEL = "device_work"
        private const val MOVE = "move"
        /** The service while it runs, and its latest start; both on the main thread. */
        private var running: DeviceWorkService? = null
        private var lastStart = 0

        /**
         * Starts the service for `job`, or stops it with none; a refused start is logged and the job runs on. Called on
         * the main thread. A service still starting stops itself once in the foreground: Android ends the app whose
         * service is stopped before it calls startForeground.
         */
        fun keep(context: Context, job: DeviceJob?) {
            if (job == null) {
                running?.stopSelf(lastStart)
                return
            }
            try {
                context.startForegroundService(Intent(context, DeviceWorkService::class.java).putExtra(MOVE, job is DeviceJob.MoveWifi))
            } catch (e: IllegalStateException) {
                Log.w(LOG, "device job $job: no foreground service: $e")
            }
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        running = this
    }

    override fun onDestroy() {
        if (running === this) running = null
        super.onDestroy()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        lastStart = startId
        val move = intent?.getBooleanExtra(MOVE, false) == true
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.device_work_channel), NotificationManager.IMPORTANCE_LOW))
        val notification = Notification.Builder(this, CHANNEL)
            .setSmallIcon(if (move) R.drawable.ic_wifi else R.drawable.ic_bluetooth)
            .setContentTitle(if (move) "機器のWi-Fiを移しています" else "機器を登録しています")
            .setContentIntent(PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE))
            .setOngoing(true).build()
        try {
            startForeground(2, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        } catch (e: RuntimeException) {
            // Refused (ForegroundServiceStartNotAllowedException, or a type without its permission): the job runs on
            // with the process as it is.
            Log.w(LOG, "device job: not in the foreground: $e")
        }
        // The job may have ended before the service came up.
        if (DeviceWork.of(this).job == null) stopSelf(startId)
        return START_NOT_STICKY
    }
}
