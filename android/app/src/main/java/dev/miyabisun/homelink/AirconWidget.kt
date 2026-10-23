package dev.miyabisun.homelink

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.SystemClock
import android.widget.RemoteViews

private const val AIRCON = "エアコン"

/**
 * How the air conditioner's button reads: as a label's toggle, "エアコン中" with its own icon
 * while home-link says its control is `on`, and a press's short result instead.
 */
fun airconFace(on: Boolean?, press: LightsMessage?, sending: Boolean = false): ButtonFace =
    buttonFace(AIRCON, R.drawable.ic_power, on, press, sending, activeIcon = R.drawable.ic_aircon_on)

/**
 * The home-screen toggle for the air conditioner's control: off holds it off, on lets it follow the room again.
 * The state is home-link's, read when placed, after a press and every update period.
 */
class AirconWidget : AppWidgetProvider() {
    private companion object {
        const val PREFS = "aircon_widget"
        const val PRESS = "dev.miyabisun.homelink.AIRCON_PRESS"
        /** Takes the button back from a short result, in a broadcast of its own so the press's receiver ends in time. */
        const val RESTORE = "dev.miyabisun.homelink.AIRCON_RESTORE"
        /** How long a button shows a press's failure before it reads as a button again. */
        const val LABEL_MS = 5_000L

        /** The state last read; none (never read, or the read failed) reads as off. */
        fun isOn(context: Context): Boolean? = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .takeIf { it.contains("on") }?.getBoolean("on", false)

        fun save(context: Context, read: ApiResult<Boolean>) {
            val edit = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            if (read is ApiResult.Ok) edit.putBoolean("on", read.value) else edit.remove("on")
            edit.commit()
        }

        fun showAll(context: Context, press: LightsMessage? = null, sending: Boolean = false) {
            val manager = AppWidgetManager.getInstance(context)
            for (id in manager.getAppWidgetIds(ComponentName(context, AirconWidget::class.java))) {
                val views = RemoteViews(context.packageName, R.layout.label_widget)
                views.setOnClickPendingIntent(R.id.label_button, PendingIntent.getBroadcast(context, 0,
                    Intent(context, AirconWidget::class.java).setAction(PRESS), PendingIntent.FLAG_IMMUTABLE))
                val face = airconFace(isOn(context), press, sending)
                views.showFace(R.id.label_button, R.id.label_button_text, face)
                views.setContentDescription(R.id.label_button, face.text)
                manager.updateAppWidget(id, views)
            }
        }

        /** Runs `work` off the main thread while the receiver stays alive for it. */
        fun BroadcastReceiver.async(work: () -> Unit) {
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

    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        showAll(context)
        async {
            save(context, homeLinkApi(context).aircon())
            showAll(context)
        }
    }

    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            PRESS -> {
                showAll(context, LightsMessage("送信中…", failed = false), sending = true)
                async {
                    val result = homeLinkApi(context).toggleAircon()
                    if (result is ApiResult.Ok) save(context, result)
                    val failed = (result as? ApiResult.Failed)?.let { failureLabel(it.error) }
                    showAll(context, failed)
                    if (failed != null) {
                        context.sendBroadcast(Intent(context, AirconWidget::class.java).setAction(RESTORE)
                            .putExtra("shown", SystemClock.elapsedRealtime()))
                    }
                }
            }
            RESTORE -> {
                val shown = intent.getLongExtra("shown", 0)
                async {
                    // A failed press may mean the state changed anyway; home-link's word is read again.
                    save(context, homeLinkApi(context).aircon())
                    Thread.sleep((shown + LABEL_MS - SystemClock.elapsedRealtime()).coerceAtLeast(0))
                    showAll(context)
                }
            }
            else -> super.onReceive(context, intent)
        }
    }
}
