package dev.miyabisun.homelink

import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.widget.RemoteViews

private const val AIRCON = "エアコン"

/**
 * How the air conditioner's button reads: as a label's toggle, "エアコン中" with its own icon
 * while home-link says its control is `on`, unknown while null, and a press's short result instead.
 */
fun airconFace(on: Boolean?, press: LightsMessage?, sending: Boolean = false): ButtonFace =
    buttonFace(AIRCON, R.drawable.ic_power, Toggle.of(on), press, sending, activeIcon = R.drawable.ic_aircon_on)

/** What home-link does with a switch the air conditioner did not take. */
private const val RESENT = "入・切は保存済みで、10分ごとにエアコンへ送り直します"

/**
 * What a press shows on the button: none when the air conditioner took the switch, as the button then shows the
 * new state; "届かず" when home-link kept it but the air conditioner did not take it; else the request's failure.
 */
fun airconPress(result: ApiResult<AirconToggle>): LightsMessage? = when (result) {
    is ApiResult.Failed -> failureLabel(result.error)
    is ApiResult.Ok -> if (result.value.delivered) null else LightsMessage("届かず", failed = true)
}

/** What is read aloud for a press's result: home-link's reason a switch it kept did not reach the air conditioner. */
fun airconMessage(result: ApiResult<AirconToggle>, host: String): String? = when (result) {
    is ApiResult.Failed -> failure(result.error, host).text
    is ApiResult.Ok -> if (result.value.delivered) null
        else "${result.value.reason?.trimEnd('。') ?: "エアコンへ届いていません"}。$RESENT"
}

/**
 * The home-screen toggle for the air conditioner's control: off holds it off, on lets it follow the room again.
 * The state is home-link's, read when placed, after a press and every update period.
 */
class AirconWidget : AppWidgetProvider() {
    companion object {
        private const val PREFS = "aircon_widget"
        private val flow = PressFlow()

        /** The state last known; none (never read, or a press may have changed it unseen) is unknown. */
        private fun isOn(context: Context): Boolean? = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .takeIf { it.contains("on") }?.getBoolean("on", false)

        private fun save(context: Context, on: Boolean?) {
            val edit = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            if (on != null) edit.putBoolean("on", on) else edit.remove("on")
            edit.commit()
        }

        /** Reads the state again within `limits`, a failed read keeping what was known; whether it was read. */
        fun read(context: Context, limits: Limits = Limits.SCREEN): Boolean {
            val read = homeLinkApi(context, limits).aircon()
            save(context, readState(isOn(context), read))
            return read is ApiResult.Ok
        }

        /** Redraws the placed widgets: the state, or a press's short result `press` with `message` read aloud. */
        fun showAll(context: Context, press: LightsMessage? = null, sending: Boolean = false, message: String? = null) {
            val manager = AppWidgetManager.getInstance(context)
            val ids = manager.getAppWidgetIds(ComponentName(context, AirconWidget::class.java))
            val blocked = ids.isNotEmpty() && Widgets.lanBlocked(context)
            for (id in ids) {
                val views = RemoteViews(context.packageName, R.layout.label_widget)
                views.setOnClickPendingIntent(R.id.label_button,
                    if (blocked) Widgets.askLan(context) else PressService.pending(context, "aircon", "エアコンに送っています", R.drawable.ic_power))
                val face = if (blocked) Widgets.LAN_FACE else airconFace(isOn(context), press, sending)
                views.showFace(R.id.label_button, R.id.label_button_text, face)
                views.setContentDescription(R.id.label_button,
                    if (blocked) "$AIRCON：${face.description}" else message?.let { "${face.text}：$it" } ?: face.description)
                manager.updateAppWidget(id, views)
            }
        }

        /**
         * Switches the control the other way, showing the press's progress and, for a while, a failure on the button:
         * the request's, or the air conditioner not taking it. The button then shows the new state.
         */
        fun press(context: Context) = flow.run(context, sending = { showAll(context, SENDING, sending = true) }, send = {
            val result = homeLinkApi(context).toggleAircon()
            save(context, pressedState(isOn(context), when (result) {
                is ApiResult.Ok -> ApiResult.Ok(result.value.on)
                is ApiResult.Failed -> result
            }))
            val short = airconPress(result)
            showAll(context, short, message = airconMessage(result, homeLinkHost(context)))
            short != null
        }, reread = { if (!read(context)) Widgets.refreshWhenOnline(context) }, restore = { showAll(context) })
    }

    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        showAll(context)
        async {
            if (!read(context, Limits.RECEIVER)) Widgets.refreshWhenOnline(context)
            showAll(context)
        }
    }
}
