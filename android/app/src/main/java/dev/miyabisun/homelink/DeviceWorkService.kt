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
        /** The service once in the foreground, and whether a start is on its way; both on the main thread. */
        private var running: DeviceWorkService? = null
        private var starting = false

        /**
         * Keeps the service up for `job`, or lets it go with none; a refused start is logged and the job runs on.
         * Called on the main thread. One start at a time, and a stop only once the service is in the foreground:
         * Android ends the app whose service stops before it calls startForeground.
         */
        fun keep(context: Context, job: DeviceJob?) {
            val service = running
            when {
                job == null -> service?.end()
                service != null -> service.show(job)
                !starting -> try {
                    context.startForegroundService(Intent(context, DeviceWorkService::class.java))
                    starting = true
                } catch (e: IllegalStateException) {
                    Log.w(LOG, "device job $job: no foreground service: $e")
                }
            }
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        starting = false
        val job = DeviceWork.of(this).job
        getSystemService(NotificationManager::class.java)
            .createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.device_work_channel), NotificationManager.IMPORTANCE_LOW))
        foreground(job)
        running = this
        // The job may have ended before the service came up.
        if (job == null) end()
        return START_NOT_STICKY
    }

    private fun foreground(job: DeviceJob?) {
        try {
            startForeground(2, notification(job), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        } catch (e: RuntimeException) {
            // Refused (ForegroundServiceStartNotAllowedException, or a type without its permission): the job runs on
            // with the process as it is.
            Log.w(LOG, "device job: not in the foreground: $e")
        }
    }

    private fun notification(job: DeviceJob?): Notification {
        val move = job is DeviceJob.MoveWifi
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(if (move) R.drawable.ic_wifi else R.drawable.ic_bluetooth)
            .setContentTitle(if (move) "機器のWi-Fiを移しています" else "機器を登録しています")
            .setContentIntent(PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE))
            .setOngoing(true).build()
    }

    /** Names the job a running service now keeps up for, through the foreground notification as it has no other. */
    private fun show(job: DeviceJob) = foreground(job)

    /** Lets the service go; a job started after this starts it again. */
    private fun end() {
        if (running === this) running = null
        stopSelf()
    }

    override fun onDestroy() {
        if (running === this) running = null
        super.onDestroy()
    }
}
