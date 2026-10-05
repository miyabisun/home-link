package dev.miyabisun.homelink

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.view.View
import android.widget.RemoteViews

/** Home-screen buttons that switch every light on or off and show the last result. */
class LightsWidget : AppWidgetProvider() {
    companion object {
        private const val ACTION_ON = "dev.miyabisun.homelink.LIGHTS_ON"
        private const val ACTION_OFF = "dev.miyabisun.homelink.LIGHTS_OFF"
        private const val PREFS = "lights_widget"

        /**
         * Redraws every placed widget with `message`, or the last saved result.
         * A `progress` line is neither saved nor marked as a result.
         */
        fun show(context: Context, message: LightsMessage? = null, progress: Boolean = false) {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            if (message != null && !progress) {
                prefs.edit().putString("text", message.text).putBoolean("failed", message.failed).apply()
            }
            val shown = message ?: prefs.getString("text", null)?.let { LightsMessage(it, prefs.getBoolean("failed", false)) }
            val manager = AppWidgetManager.getInstance(context)
            val ids = manager.getAppWidgetIds(ComponentName(context, LightsWidget::class.java))
            if (ids.isNotEmpty()) manager.updateAppWidget(ids, views(context, shown, progress))
        }

        private fun views(context: Context, message: LightsMessage?, progress: Boolean) =
            RemoteViews(context.packageName, R.layout.lights_widget).apply {
                setOnClickPendingIntent(R.id.light_on, action(context, ACTION_ON))
                setOnClickPendingIntent(R.id.light_off, action(context, ACTION_OFF))
                setViewVisibility(R.id.status, if (message == null) View.GONE else View.VISIBLE)
                if (message != null) {
                    setTextViewText(R.id.status, message.text)
                    setTextColor(R.id.status, context.getColor(if (message.failed) R.color.danger else R.color.text))
                    val icon = when {
                        progress -> 0
                        message.failed -> R.drawable.ic_error
                        else -> R.drawable.ic_success
                    }
                    setTextViewCompoundDrawablesRelative(R.id.status, icon, 0, 0, 0)
                }
            }

        private fun action(context: Context, action: String) = PendingIntent.getBroadcast(
            context, 0, Intent(context, LightsWidget::class.java).setAction(action),
            PendingIntent.FLAG_IMMUTABLE,
        )
    }

    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) = show(context)

    override fun onReceive(context: Context, intent: Intent) {
        val on = when (intent.action) {
            ACTION_ON -> true
            ACTION_OFF -> false
            else -> return super.onReceive(context, intent)
        }
        show(context, LightsMessage(if (on) "照明をオンにしています…" else "照明をオフにしています…", failed = false), progress = true)
        val pending = goAsync()
        Thread {
            try {
                val api = MainActivity.apiFactory?.invoke() ?: HttpHomeLinkApi(BuildConfig.HOME_LINK_URL)
                val host = Uri.parse(BuildConfig.HOME_LINK_URL).authority.orEmpty()
                show(context, lightsMessage(on, api.switchLights(on), host, names = false))
            } finally {
                pending.finish()
            }
        }.start()
    }
}
