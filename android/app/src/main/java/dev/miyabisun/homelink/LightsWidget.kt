package dev.miyabisun.homelink

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.SystemClock
import android.util.SizeF
import android.view.View
import android.widget.RemoteViews
import java.util.concurrent.atomic.AtomicInteger

/**
 * Home-screen buttons that switch every light on or off, and boost or release the lights of the
 * work desk and work labels. A label button reads "<name>中" while the server says its label is
 * boosted. One row shows a press's progress and a short result on the pressed button for a while;
 * a taller widget keeps the last result below the buttons.
 */
class LightsWidget : AppWidgetProvider() {
    /** A widget button `id` with its `words`: switching every light `on` or off, or toggling the boost of `label`. */
    private enum class WidgetButton(val id: Int, val words: Int, val action: String, val text: Int, val icon: Int,
                                    val on: Boolean = false, val label: String? = null) {
        ON(R.id.light_on, R.id.light_on_text, "dev.miyabisun.homelink.LIGHTS_ON", R.string.lights_on, R.drawable.ic_light_on, on = true),
        OFF(R.id.light_off, R.id.light_off_text, "dev.miyabisun.homelink.LIGHTS_OFF", R.string.lights_off, R.drawable.ic_light_off),
        WORK(R.id.light_work, R.id.light_work_text, "dev.miyabisun.homelink.BOOST_WORK", R.string.lights_work, R.drawable.ic_work, label = "仕事用デスク"),
        TASK(R.id.light_task, R.id.light_task_text, "dev.miyabisun.homelink.BOOST_TASK", R.string.lights_task, R.drawable.ic_task, label = "作業"),
    }

    /** The button a widget press went through and its short result; `sending` until the answer. */
    private class Press(val button: WidgetButton, val label: LightsMessage?, val sending: Boolean = false)

    companion object {
        private const val PREFS = "lights_widget"
        /**
         * Reads the boosts again after a press and takes the pressed button back from a short result, in a
         * broadcast of its own: a receiver may run 10 s, and a slow answer to the press must not use that up.
         */
        private const val REREAD = "dev.miyabisun.homelink.LIGHTS_REREAD"
        /** Above one row of any launcher grid (about 96dp on a phone, 120dp on a tablet), the result line fits below. */
        private const val TALL_DP = 160f
        /** How long one row shows a press's result before its button reads as a button again. */
        private const val LABEL_MS = 5_000L
        private val presses = AtomicInteger()

        /**
         * Redraws every placed widget with `message`, or the last saved result.
         * A `progress` line is neither saved nor marked as a result.
         */
        fun show(context: Context, message: LightsMessage? = null, progress: Boolean = false) =
            show(context, message, progress, press = null)

        /** Saves which labels a read found boosted; a failed read forgets them, so none reads as boosted. */
        fun saveBoosts(context: Context, read: ApiResult<Set<String>>) =
            storeBoosts(context, (read as? ApiResult.Ok)?.value)

        /** The labels last known boosted, or null when unknown. */
        private fun boosts(context: Context): Set<String>? {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            return if (prefs.getBoolean("boosts_known", false)) prefs.getStringSet("boosted", emptySet()) else null
        }

        private fun storeBoosts(context: Context, boosted: Set<String>?) {
            val edit = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            if (boosted == null) edit.putBoolean("boosts_known", false).remove("boosted")
            else edit.putBoolean("boosts_known", true).putStringSet("boosted", boosted)
            edit.commit()
        }

        private fun show(context: Context, message: LightsMessage?, progress: Boolean, press: Press?) {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            if (message != null && !progress) {
                prefs.edit().putString("text", message.text).putBoolean("failed", message.failed).apply()
            }
            val shown = message ?: prefs.getString("text", null)?.let { LightsMessage(it, prefs.getBoolean("failed", false)) }
            val boosted = boosts(context)
            val manager = AppWidgetManager.getInstance(context)
            val ids = manager.getAppWidgetIds(ComponentName(context, LightsWidget::class.java))
            if (ids.isNotEmpty()) manager.updateAppWidget(ids, views(context, shown, progress, press, boosted))
        }

        /** One row of buttons, or the buttons over the result line once the widget is tall enough. */
        private fun views(context: Context, message: LightsMessage?, progress: Boolean, press: Press?, boosted: Set<String>?): RemoteViews {
            val row = RemoteViews(context.packageName, R.layout.lights_widget_row).apply {
                buttons(context, this, press, message, boosted)
            }
            val tall = RemoteViews(context.packageName, R.layout.lights_widget).apply {
                buttons(context, this, press = null, message = null, boosted)
                status(this, message, progress)
            }
            return RemoteViews(mapOf(SizeF(0f, 0f) to row, SizeF(0f, TALL_DP) to tall))
        }

        private fun buttons(context: Context, views: RemoteViews, press: Press?, message: LightsMessage?, boosted: Set<String>?) {
            for (button in WidgetButton.entries) {
                val id = button.id
                views.setOnClickPendingIntent(id, action(context, button.action))
                val name = context.getString(button.text)
                val pressed = press?.takeIf { it.button == button }
                val face = buttonFace(name, button.icon, button.label?.let { boosted?.contains(it) }, pressed?.label,
                    pressed?.sending == true)
                views.showFace(id, button.words, face)
                // A press that shows the new state is read as that state, not as the result line.
                views.setContentDescription(id, if (pressed?.label == null || message == null) face.text else "$name：${message.text}")
            }
        }

        private fun status(views: RemoteViews, message: LightsMessage?, progress: Boolean) {
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

        private fun api() = MainActivity.apiFactory?.invoke() ?: HttpHomeLinkApi(BuildConfig.HOME_LINK_URL)

        /** Runs `work` off the main thread while the receiver stays alive for it. */
        private fun BroadcastReceiver.async(work: () -> Unit) {
            val pending = goAsync()
            Thread {
                try {
                    work()
                } finally {
                    pending.finish()
                }
            }.start()
        }
    }

    /** Draws the widget as last known, then reads the boosts again: on placement and every update period. */
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        show(context)
        async {
            saveBoosts(context, api().boostedLabels())
            show(context)
        }
    }

    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == REREAD) {
            val press = intent.getIntExtra("press", 0)
            val shown = intent.getLongExtra("shown", 0)
            val restore = intent.getBooleanExtra("restore", false)
            return async {
                saveBoosts(context, api().boostedLabels())
                // The receiver stays alive for the pause: a cached process may be frozen before a later callback.
                if (restore) Thread.sleep((shown + LABEL_MS - SystemClock.elapsedRealtime()).coerceAtLeast(0))
                if (presses.get() == press) show(context)
            }
        }
        val button = WidgetButton.entries.find { it.action == intent.action }
            ?: return super.onReceive(context, intent)
        val label = button.label
        val press = presses.incrementAndGet()
        val sending = when {
            label != null -> "${label}の照明を切り替えています…"
            button.on -> "照明をオンにしています…"
            else -> "照明をオフにしています…"
        }
        show(context, LightsMessage(sending, failed = false), progress = true, Press(button, LightsMessage("送信中…", failed = false), sending = true))
        async {
            val api = api()
            val host = Uri.parse(BuildConfig.HOME_LINK_URL).authority.orEmpty()
            val (message, short) = if (label != null) {
                val result = api.toggleBoost(label)
                storeBoosts(context, boostsAfterToggle(boosts(context), label, result))
                boostMessage(label, result, host) to boostLabel(result)
            } else {
                val result = api.switchLights(button.on)
                storeBoosts(context, boostsAfterSwitch(boosts(context), button.on, result))
                lightsMessage(button.on, result, host, detail = false) to lightsLabel(button.on, result)
            }
            show(context, message, progress = false, Press(button, short))
            // Every press can change a boost, here or elsewhere, so the server's word is read again.
            context.sendBroadcast(Intent(context, LightsWidget::class.java).setAction(REREAD).putExtra("press", press)
                .putExtra("shown", SystemClock.elapsedRealtime()).putExtra("restore", short != null))
        }
    }
}
