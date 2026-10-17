package dev.miyabisun.homelink

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import android.widget.RemoteViews
import java.util.concurrent.atomic.AtomicInteger

/**
 * One-cell home-screen buttons that unlock or lock the front door, kept apart from the lights so a
 * slip of the finger there cannot open the door. A press shows its progress and result on the
 * button for a while, then the button reads as a button again.
 */
abstract class DoorButton(private val command: DoorCommand) : AppWidgetProvider() {
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) = showDoor(context, command)
}

class DoorWidget : DoorButton(DoorCommand.UNLOCK)

class DoorLockWidget : DoorButton(DoorCommand.LOCK)

private val DoorCommand.widget get() = if (this == DoorCommand.UNLOCK) DoorWidget::class.java else DoorLockWidget::class.java
private val DoorCommand.layout get() = if (this == DoorCommand.UNLOCK) R.layout.door_widget else R.layout.door_lock_widget
private val DoorCommand.icon get() = if (this == DoorCommand.UNLOCK) R.drawable.ic_unlock else R.drawable.ic_lock

/**
 * Redraws every placed widget of `command`: the button, or `label` with `message` read aloud.
 * A `sending` label keeps the button's icon, as it is no result yet.
 */
private fun showDoor(context: Context, command: DoorCommand, label: LightsMessage? = null, message: String? = null,
                     sending: Boolean = false) {
    val manager = AppWidgetManager.getInstance(context)
    val ids = manager.getAppWidgetIds(ComponentName(context, command.widget))
    if (ids.isEmpty()) return
    val views = RemoteViews(context.packageName, command.layout)
    views.setOnClickPendingIntent(R.id.door_button, PendingIntent.getForegroundService(
        context, command.ordinal, DoorService.intent(context, command), PendingIntent.FLAG_IMMUTABLE,
    ))
    views.showFace(R.id.door_button, R.id.door_button_text, buttonFace(command.verb, command.icon, null, label, sending))
    views.setContentDescription(R.id.door_button, if (message == null) command.verb else "${command.verb}：$message")
    manager.updateAppWidget(ids, views)
}

/**
 * Sends a pressed door command as a short foreground service: Android lets a background app use metered
 * networks with Data Saver on only from the foreground, and a widget's broadcast receiver is background.
 */
class DoorService : Service() {
    companion object {
        private const val COMMAND = "command"
        private const val CHANNEL = "door"
        /** How long the button shows a press's result before it reads as a button again. */
        private const val LABEL_MS = 5_000L
        private val presses = DoorCommand.entries.associateWith { AtomicInteger() }

        fun intent(context: Context, command: DoorCommand): Intent =
            Intent(context, DoorService::class.java).setAction(command.name).putExtra(COMMAND, command.name)
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val command = intent?.getStringExtra(COMMAND)?.let(DoorCommand::valueOf) ?: run {
            stopSelf(startId)
            return START_NOT_STICKY
        }
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.door_channel), NotificationManager.IMPORTANCE_LOW))
        val notification = Notification.Builder(this, CHANNEL).setSmallIcon(command.icon)
            .setContentTitle("玄関に${command.verb}を送っています").build()
        try {
            startForeground(1, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE)
        } catch (_: IllegalStateException) {
            // Refused (ForegroundServiceStartNotAllowedException): send anyway, as the widget did before.
        }
        val press = presses.getValue(command).incrementAndGet()
        showDoor(this, command, LightsMessage("送信中…", failed = false), "${command.verb}を送っています", sending = true)
        Thread {
            try {
                val result = homeLinkApi(this).door(command)
                val host = homeLinkHost(this)
                showDoor(this, command, doorLabel(result), doorMessage(command, result, host).text)
                Thread.sleep(LABEL_MS)
                if (presses.getValue(command).get() == press) showDoor(this, command)
            } finally {
                stopSelf(startId)
            }
        }.start()
        return START_NOT_STICKY
    }

    // A short service may run 3 minutes; a press takes at most about 40 s.
    override fun onTimeout(startId: Int, fgsType: Int) = stopSelf(startId)
}
