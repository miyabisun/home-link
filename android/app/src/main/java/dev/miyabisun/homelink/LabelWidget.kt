package dev.miyabisun.homelink

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.widget.RemoteViews

/** Where a placed label widget points: its label and the name it shows. */
data class WidgetConfig(val labelId: Long, val name: String)

/**
 * The four home-screen buttons for one label, each chosen with its name when placed:
 * on and off side by side, on, off, and a toggle that reads "<name>中" while the server says the label is on.
 * A press shows its progress and short result on the pressed button for a while.
 */
abstract class LabelWidget(private val kind: Kind) : AppWidgetProvider() {
    /** What each kind of widget sends from its buttons, left to right. */
    enum class Kind(val ops: List<LabelOp>, val layout: Int) {
        SWITCH(listOf(LabelOp.ON, LabelOp.OFF), R.layout.label_switch_widget),
        ON(listOf(LabelOp.ON), R.layout.label_widget),
        OFF(listOf(LabelOp.OFF), R.layout.label_widget),
        TOGGLE(listOf(LabelOp.TOGGLE), R.layout.label_widget),
    }

    /** The button a press went through, its short result (`sending` until the answer) and what to read aloud. */
    private class Press(val op: LabelOp, val label: LightsMessage?, val sending: Boolean = false, val message: String? = null)

    companion object {
        private const val PREFS = "label_widgets"
        private const val OP = "op"
        private const val KIND = "kind"
        private val CELLS = listOf(R.id.label_button to R.id.label_button_text, R.id.label_button2 to R.id.label_button2_text)
        private val flow = PressFlow()

        private fun Kind.provider() = when (this) {
            Kind.SWITCH -> LabelSwitchWidget::class.java
            Kind.ON -> LabelOnWidget::class.java
            Kind.OFF -> LabelOffWidget::class.java
            Kind.TOGGLE -> LabelToggleWidget::class.java
        }

        fun config(context: Context, id: Int): WidgetConfig? {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val label = prefs.getLong("$id.label", -1).takeIf { it >= 0 } ?: return null
            return WidgetConfig(label, prefs.getString("$id.name", null).orEmpty())
        }

        fun save(context: Context, id: Int, config: WidgetConfig) {
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
                .putLong("$id.label", config.labelId).putString("$id.name", config.name).commit()
        }

        /** Each label's state as last known; a label missing is unknown. */
        private fun states(context: Context): Map<Long, Boolean> {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            fun ids(key: String) = prefs.getStringSet(key, emptySet()).orEmpty().mapNotNull(String::toLongOrNull)
            return ids("off").associateWith { false } + ids("on").associateWith { true }
        }

        private fun save(context: Context, states: Map<Long, Boolean>) {
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
                .putStringSet("on", states.filterValues { it }.keys.map(Long::toString).toSet())
                .putStringSet("off", states.filterValues { !it }.keys.map(Long::toString).toSet()).commit()
        }

        /** Keeps what a read of the labels found; a failed read keeps what was known. */
        fun saveStates(context: Context, read: ApiResult<List<Label>>) = save(context, readStates(states(context), read))

        /** Reads the labels' states again; whether they were read. */
        fun read(context: Context): Boolean {
            val read = homeLinkApi(context).labels()
            saveStates(context, read)
            return read is ApiResult.Ok
        }

        /** Redraws every placed label widget as last known. */
        fun showAll(context: Context) {
            val manager = AppWidgetManager.getInstance(context)
            for (kind in Kind.entries) {
                for (id in manager.getAppWidgetIds(ComponentName(context, kind.provider()))) show(context, kind, id)
            }
        }

        private fun show(context: Context, kind: Kind, id: Int, press: Press? = null) {
            val views = RemoteViews(context.packageName, kind.layout)
            val config = config(context, id)
            for ((op, cell) in kind.ops.zip(CELLS)) {
                val (button, words) = cell
                if (config == null) {
                    // Placed without a label (the setup was left): the button opens the setup again.
                    views.setOnClickPendingIntent(button, PendingIntent.getActivity(context, id,
                        Intent(context, WidgetConfigActivity::class.java).putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id),
                        PendingIntent.FLAG_IMMUTABLE))
                    views.showFace(button, words, ButtonFace("", R.drawable.ic_settings, Tone.PLAIN))
                    views.setContentDescription(button, "設定")
                    continue
                }
                views.setOnClickPendingIntent(button, PressService.pending(context, "label:$id:${op.name}",
                    "「${config.name}」に送っています", R.drawable.ic_light_on) {
                    putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id).putExtra(OP, op.ordinal).putExtra(KIND, kind.ordinal)
                })
                val pressed = press?.takeIf { it.op == op }
                val face = labelFace(op, config.name, states(context)[config.labelId], pressed?.label, pressed?.sending == true)
                views.showFace(button, words, face)
                // A press that shows the new state is read as that state, not as the result.
                views.setContentDescription(button, pressed?.message?.takeIf { pressed.label != null }?.let { "${face.text}：$it" } ?: face.description)
            }
            AppWidgetManager.getInstance(context).updateAppWidget(id, views)
        }

        /**
         * Sends a pressed button's op and shows its progress and short result on that button, then reads the labels
         * again (a press can change other labels too: off ends their overrides) and takes the button back.
         */
        fun press(context: Context, intent: Intent) {
            val id = intent.getIntExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, AppWidgetManager.INVALID_APPWIDGET_ID)
            val kind = Kind.entries.getOrNull(intent.getIntExtra(KIND, -1)) ?: return
            val op = LabelOp.entries.getOrNull(intent.getIntExtra(OP, -1)) ?: return
            val config = config(context, id) ?: return
            flow.run(context, sending = { show(context, kind, id, Press(op, SENDING, sending = true)) }, send = {
                val result = homeLinkApi(context).switchLabel(config.labelId, op)
                val known = states(context)
                val after = pressedState(known[config.labelId], when (result) {
                    is ApiResult.Ok -> ApiResult.Ok(result.value.on)
                    is ApiResult.Failed -> result
                })
                save(context, if (after == null) known - config.labelId else known + (config.labelId to after))
                val short = pressLabel(op, result)
                show(context, kind, id, Press(op, short, message = labelMessage(config.name, result, homeLinkHost(context))?.text))
                short != null
            }, reread = { if (!read(context)) Widgets.refreshWhenOnline(context) }, restore = { showAll(context) })
        }
    }

    /** Draws the widgets as last known, then reads the labels again: on placement and every update period. */
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        for (id in ids) show(context, kind, id)
        async {
            if (!read(context)) Widgets.refreshWhenOnline(context)
            showAll(context)
        }
    }

    override fun onDeleted(context: Context, ids: IntArray) {
        val edit = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
        for (id in ids) edit.remove("$id.label").remove("$id.name")
        edit.apply()
    }
}

class LabelSwitchWidget : LabelWidget(Kind.SWITCH)

class LabelOnWidget : LabelWidget(Kind.ON)

class LabelOffWidget : LabelWidget(Kind.OFF)

class LabelToggleWidget : LabelWidget(Kind.TOGGLE)
