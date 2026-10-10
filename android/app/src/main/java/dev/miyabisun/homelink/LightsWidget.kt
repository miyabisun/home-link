package dev.miyabisun.homelink

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.SizeF
import android.util.TypedValue
import android.view.View
import android.widget.RemoteViews
import java.util.concurrent.atomic.AtomicInteger

/**
 * Home-screen buttons that switch every light on or off, and boost or release the lights of the
 * work desk and work labels. One row shows a press's result on the pressed button for a while;
 * a taller widget keeps the last result below the buttons.
 */
class LightsWidget : AppWidgetProvider() {
    /** A widget button: switching every light `on` or off, or toggling the boost of `label`. */
    private enum class WidgetButton(val id: Int, val action: String, val text: Int, val icon: Int,
                                    val on: Boolean = false, val label: String? = null) {
        ON(R.id.light_on, "dev.miyabisun.homelink.LIGHTS_ON", R.string.lights_on, R.drawable.ic_light_on, on = true),
        OFF(R.id.light_off, "dev.miyabisun.homelink.LIGHTS_OFF", R.string.lights_off, R.drawable.ic_light_off),
        WORK(R.id.light_work, "dev.miyabisun.homelink.BOOST_WORK", R.string.lights_work, R.drawable.ic_work, label = "仕事用デスク"),
        TASK(R.id.light_task, "dev.miyabisun.homelink.BOOST_TASK", R.string.lights_task, R.drawable.ic_task, label = "作業"),
    }

    /** The button a widget press went through and its short result; no `label` while sending. */
    private class Press(val button: WidgetButton, val label: LightsMessage?)

    companion object {
        private const val PREFS = "lights_widget"
        /** Above one row of any launcher grid (about 96dp on a phone, 120dp on a tablet), the result line fits below. */
        private const val TALL_DP = 160f
        /** The width from which a label fits beside its icon (four buttons of about 120dp). */
        private const val WIDE_DP = 520f
        /** How long one row shows a press's result before its button reads as a button again. */
        private const val LABEL_MS = 5_000L
        private val presses = AtomicInteger()

        /**
         * Redraws every placed widget with `message`, or the last saved result.
         * A `progress` line is neither saved nor marked as a result.
         */
        fun show(context: Context, message: LightsMessage? = null, progress: Boolean = false) =
            show(context, message, progress, press = null)

        private fun show(context: Context, message: LightsMessage?, progress: Boolean, press: Press?) {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            if (message != null && !progress) {
                prefs.edit().putString("text", message.text).putBoolean("failed", message.failed).apply()
            }
            val shown = message ?: prefs.getString("text", null)?.let { LightsMessage(it, prefs.getBoolean("failed", false)) }
            val manager = AppWidgetManager.getInstance(context)
            val ids = manager.getAppWidgetIds(ComponentName(context, LightsWidget::class.java))
            if (ids.isNotEmpty()) manager.updateAppWidget(ids, views(context, shown, progress, press))
        }

        /**
         * One row of buttons, or the buttons over the result line once the widget is tall enough;
         * a narrow widget puts each icon above its label.
         */
        private fun views(context: Context, message: LightsMessage?, progress: Boolean, press: Press?): RemoteViews {
            fun row(narrow: Boolean) = RemoteViews(context.packageName, R.layout.lights_widget_row).apply {
                buttons(context, this, narrow, press, message)
            }
            fun tall(narrow: Boolean) = RemoteViews(context.packageName, R.layout.lights_widget).apply {
                buttons(context, this, narrow, press = null, message = null)
                status(context, this, message, progress)
            }
            return RemoteViews(mapOf(
                SizeF(0f, 0f) to row(narrow = true),
                SizeF(WIDE_DP, 0f) to row(narrow = false),
                SizeF(0f, TALL_DP) to tall(narrow = true),
                SizeF(WIDE_DP, TALL_DP) to tall(narrow = false),
            ))
        }

        private fun buttons(context: Context, views: RemoteViews, narrow: Boolean, press: Press?, message: LightsMessage?) {
            val padding = (4 * context.resources.displayMetrics.density).toInt()
            for (button in WidgetButton.entries) {
                val id = button.id
                views.setOnClickPendingIntent(id, action(context, button.action))
                val name = context.getString(button.text)
                val pressed = press?.takeIf { it.button == button }
                val label = pressed?.label
                views.setTextViewText(id, if (pressed == null) name else label?.text ?: "送信中…")
                // A resource, not a value, so a widget drawn before the OS switches dark or light follows it.
                views.setColorStateList(id, "setTextColor", if (label?.failed == true) R.color.danger else R.color.text)
                val icon = when {
                    label == null -> button.icon
                    label.failed -> R.drawable.ic_error
                    else -> R.drawable.ic_success
                }
                if (narrow) {
                    views.setTextViewCompoundDrawablesRelative(id, 0, icon, 0, 0)
                    views.setViewPadding(id, padding, padding, padding, padding)
                    views.setTextViewTextSize(id, TypedValue.COMPLEX_UNIT_SP, 14f)
                } else {
                    views.setTextViewCompoundDrawablesRelative(id, icon, 0, 0, 0)
                }
                views.setContentDescription(id, if (pressed == null || message == null) name else "$name：${message.text}")
            }
        }

        private fun status(context: Context, views: RemoteViews, message: LightsMessage?, progress: Boolean) {
            views.setViewVisibility(R.id.status, if (message == null) View.GONE else View.VISIBLE)
            if (message == null) return
            views.setTextViewText(R.id.status, message.text)
            views.setColorStateList(R.id.status, "setTextColor", if (message.failed) R.color.danger else R.color.text)
            val icon = when {
                progress -> 0
                message.failed -> R.drawable.ic_error
                else -> R.drawable.ic_success
            }
            views.setTextViewCompoundDrawablesRelative(R.id.status, icon, 0, 0, 0)
        }

        private fun action(context: Context, action: String) = PendingIntent.getBroadcast(
            context, 0, Intent(context, LightsWidget::class.java).setAction(action),
            PendingIntent.FLAG_IMMUTABLE,
        )
    }

    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) = show(context)

    override fun onReceive(context: Context, intent: Intent) {
        val button = WidgetButton.entries.find { it.action == intent.action }
            ?: return super.onReceive(context, intent)
        val label = button.label
        val press = presses.incrementAndGet()
        val sending = when {
            label != null -> "${label}の照明を切り替えています…"
            button.on -> "照明をオンにしています…"
            else -> "照明をオフにしています…"
        }
        show(context, LightsMessage(sending, failed = false), progress = true, Press(button, label = null))
        val pending = goAsync()
        Thread {
            try {
                val api = MainActivity.apiFactory?.invoke() ?: HttpHomeLinkApi(BuildConfig.HOME_LINK_URL)
                val host = Uri.parse(BuildConfig.HOME_LINK_URL).authority.orEmpty()
                val (message, short) = if (label != null) {
                    val result = api.toggleBoost(label)
                    boostMessage(label, result, host) to boostLabel(result)
                } else {
                    val result = api.switchLights(button.on)
                    lightsMessage(button.on, result, host, detail = false) to lightsLabel(button.on, result)
                }
                show(context, message, progress = false, Press(button, short))
                // The receiver stays alive for the pause: a cached process may be frozen before a later callback.
                Thread.sleep(LABEL_MS)
                if (presses.get() == press) show(context)
            } finally {
                pending.finish()
            }
        }.start()
    }
}
