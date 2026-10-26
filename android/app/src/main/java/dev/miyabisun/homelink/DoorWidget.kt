package dev.miyabisun.homelink

import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.widget.RemoteViews

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
    if (Widgets.lanBlocked(context)) {
        views.setOnClickPendingIntent(R.id.door_button, Widgets.askLan(context))
        views.showFace(R.id.door_button, R.id.door_button_text, Widgets.LAN_FACE)
        views.setContentDescription(R.id.door_button, "${command.verb}：${Widgets.LAN_FACE.description}")
        return manager.updateAppWidget(ids, views)
    }
    views.setOnClickPendingIntent(R.id.door_button, PressService.pending(context, "door:${command.name}",
        "玄関に${command.verb}を送っています", command.icon) { putExtra(COMMAND, command.name) })
    views.showFace(R.id.door_button, R.id.door_button_text, buttonFace(command.verb, command.icon, null, label, sending))
    views.setContentDescription(R.id.door_button, if (message == null) command.verb else "${command.verb}：$message")
    manager.updateAppWidget(ids, views)
}

private const val COMMAND = "command"
private val flows = DoorCommand.entries.associateWith { PressFlow() }

/** Draws every placed door widget as a button. */
internal fun showDoors(context: Context) = DoorCommand.entries.forEach { showDoor(context, it) }

/** Sends a pressed door command, showing its progress and then its result on the button for a while. */
internal fun pressDoor(context: Context, intent: Intent) {
    val command = intent.getStringExtra(COMMAND)?.let { name -> DoorCommand.entries.firstOrNull { it.name == name } } ?: return
    flows.getValue(command).run(context, sending = { showDoor(context, command, SENDING, "${command.verb}を送っています", sending = true) }, send = {
        val result = homeLinkApi(context).door(command)
        showDoor(context, command, doorLabel(result), doorMessage(command, result, homeLinkHost(context)).text)
        true
    }, reread = {}, restore = { showDoor(context, command) })
}
