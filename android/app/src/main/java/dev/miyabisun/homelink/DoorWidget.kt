package dev.miyabisun.homelink

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.widget.RemoteViews
import java.util.concurrent.atomic.AtomicInteger

/**
 * A one-cell home-screen button that unlocks the front door, kept apart from the lights so a
 * slip of the finger there cannot open the door. A press shows its progress and result on the
 * button for a while, then the button reads as a button again.
 */
class DoorWidget : AppWidgetProvider() {
    companion object {
        private const val UNLOCK = "dev.miyabisun.homelink.UNLOCK_DOOR"
        /** How long the button shows a press's result before it reads as a button again. */
        private const val LABEL_MS = 5_000L
        private val presses = AtomicInteger()

        /**
         * Redraws every placed unlock widget: the button, or `label` with `message` read aloud.
         * A `sending` label keeps the button's icon, as it is no result yet.
         */
        private fun show(context: Context, label: LightsMessage? = null, message: String? = null, sending: Boolean = false) {
            val manager = AppWidgetManager.getInstance(context)
            val ids = manager.getAppWidgetIds(ComponentName(context, DoorWidget::class.java))
            if (ids.isEmpty()) return
            val views = RemoteViews(context.packageName, R.layout.door_widget)
            val id = R.id.door_unlock
            val name = context.getString(R.string.door_unlock)
            views.setOnClickPendingIntent(id, PendingIntent.getBroadcast(
                context, 0, Intent(context, DoorWidget::class.java).setAction(UNLOCK), PendingIntent.FLAG_IMMUTABLE,
            ))
            views.setTextViewText(id, label?.text ?: name)
            // A resource, not a value, so a widget drawn before the OS switches dark or light follows it.
            views.setColorStateList(id, "setTextColor", if (label?.failed == true) R.color.danger else R.color.text)
            val icon = when {
                label == null || sending -> R.drawable.ic_unlock
                label.failed -> R.drawable.ic_error
                else -> R.drawable.ic_success
            }
            views.setTextViewCompoundDrawablesRelative(id, 0, icon, 0, 0)
            views.setContentDescription(id, if (message == null) name else "$name：$message")
            manager.updateAppWidget(ids, views)
        }
    }

    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) = show(context)

    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != UNLOCK) return super.onReceive(context, intent)
        val press = presses.incrementAndGet()
        show(context, LightsMessage("送信中…", failed = false), "解錠を送っています", sending = true)
        val pending = goAsync()
        Thread {
            try {
                val api = MainActivity.apiFactory?.invoke() ?: HttpHomeLinkApi(BuildConfig.HOME_LINK_URL)
                val host = Uri.parse(BuildConfig.HOME_LINK_URL).authority.orEmpty()
                val result = api.unlockDoor()
                show(context, doorLabel(result), doorMessage(result, host).text)
                // The receiver stays alive for the pause: a cached process may be frozen before a later callback.
                Thread.sleep(LABEL_MS)
                if (presses.get() == press) show(context)
            } finally {
                pending.finish()
            }
        }.start()
    }
}
