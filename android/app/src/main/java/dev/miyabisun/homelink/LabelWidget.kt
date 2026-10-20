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
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Future
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException
import java.util.concurrent.atomic.AtomicInteger

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
        private const val PRESS = "dev.miyabisun.homelink.LABEL_PRESS"
        /**
         * Reads the labels again after a press and takes the pressed button back from a short result, in a
         * broadcast of its own: a receiver may run 10 s, and a slow answer to the press must not use that up.
         */
        private const val REREAD = "dev.miyabisun.homelink.LABEL_REREAD"
        /** Waits on for a press's answer that has not come within one receiver's time. */
        private const val AWAIT = "dev.miyabisun.homelink.LABEL_AWAIT"
        private const val OP = "op"
        /** How long a button shows a press's result before it reads as a button again. */
        private const val LABEL_MS = 5_000L
        /** How long one receiver waits for a press's answer, within the 10 s a pressed button's receiver may run. */
        private const val HOLD_MS = 8_000L
        private val CELLS = listOf(R.id.label_button to R.id.label_button_text, R.id.label_button2 to R.id.label_button2_text)
        private val presses = AtomicInteger()
        /** The answers presses still wait for, by press. */
        private val answers = ConcurrentHashMap<Int, Future<*>>()

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

        /** Saves each label's state a read found; a failed read forgets them, so no toggle reads as on. */
        fun saveStates(context: Context, read: ApiResult<List<Label>>) {
            val edit = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putStringSet("on", emptySet())
            if (read is ApiResult.Ok) edit.putStringSet("on", read.value.filter { it.on == true }.map { it.id.toString() }.toSet())
            edit.commit()
        }

        private fun saveState(context: Context, label: Long, on: Boolean?) {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            val ids = prefs.getStringSet("on", emptySet()).orEmpty() - label.toString()
            prefs.edit().putStringSet("on", if (on == true) ids + label.toString() else ids).commit()
        }

        private fun isOn(context: Context, label: Long) =
            label.toString() in context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getStringSet("on", emptySet()).orEmpty()

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
                views.setOnClickPendingIntent(button, PendingIntent.getBroadcast(context, id * LabelOp.entries.size + op.ordinal,
                    Intent(context, kind.provider()).setAction(PRESS).putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id)
                        .putExtra(OP, op.ordinal), PendingIntent.FLAG_IMMUTABLE))
                val pressed = press?.takeIf { it.op == op }
                val face = labelFace(op, config.name, isOn(context, config.labelId), pressed?.label, pressed?.sending == true)
                views.showFace(button, words, face)
                // A press that shows the new state is read as that state, not as the result.
                views.setContentDescription(button, pressed?.message?.takeIf { pressed.label != null }?.let { "${face.text}：$it" } ?: face.text)
            }
            AppWidgetManager.getInstance(context).updateAppWidget(id, views)
        }

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

    /** Draws the widgets as last known, then reads the labels again: on placement and every update period. */
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        for (id in ids) show(context, kind, id)
        async {
            saveStates(context, homeLinkApi(context).labels())
            showAll(context)
        }
    }

    override fun onDeleted(context: Context, ids: IntArray) {
        val edit = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
        for (id in ids) edit.remove("$id.label").remove("$id.name")
        edit.apply()
    }

    /**
     * Waits for press `press`'s answer, handing the wait to a broadcast of its own before the receiver's time runs
     * out: the label's server may take longer than 10 s, and a receiver over its time takes the process down.
     */
    private fun BroadcastReceiver.await(context: Context, press: Int) {
        // None after the process was started again: the answer is lost, so the buttons read as buttons again.
        val answer = answers[press] ?: return showAll(context)
        async {
            try {
                answer.get(HOLD_MS, TimeUnit.MILLISECONDS)
                answers.remove(press)
            } catch (_: TimeoutException) {
                context.sendBroadcast(Intent(context, kind.provider()).setAction(AWAIT).putExtra("press", press))
            }
        }
    }

    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getIntExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, AppWidgetManager.INVALID_APPWIDGET_ID)
        when (intent.action) {
            REREAD -> {
                val press = intent.getIntExtra("press", 0)
                val shown = intent.getLongExtra("shown", 0)
                val restore = intent.getBooleanExtra("restore", false)
                async {
                    saveStates(context, homeLinkApi(context).labels())
                    // The receiver stays alive for the pause: a cached process may be frozen before a later callback.
                    if (restore) Thread.sleep((shown + LABEL_MS - SystemClock.elapsedRealtime()).coerceAtLeast(0))
                    if (presses.get() == press) showAll(context)
                }
            }
            PRESS -> {
                val op = LabelOp.entries.getOrNull(intent.getIntExtra(OP, -1)) ?: return
                val config = config(context, id) ?: return
                val press = presses.incrementAndGet()
                show(context, kind, id, Press(op, LightsMessage("送信中…", failed = false), sending = true))
                val answer = FutureTask {
                    val result = homeLinkApi(context).switchLabel(config.labelId, op)
                    if (result is ApiResult.Ok) saveState(context, config.labelId, result.value.on)
                    val short = pressLabel(op, result)
                    val message = labelMessage(config.name, result, homeLinkHost(context))
                    show(context, kind, id, Press(op, short, message = message?.text))
                    // A press can change other labels too (off ends their overrides), so the server's word is read again.
                    context.sendBroadcast(Intent(context, kind.provider()).setAction(REREAD).putExtra("press", press)
                        .putExtra("shown", SystemClock.elapsedRealtime()).putExtra("restore", short != null))
                }
                answers[press] = answer
                Thread(answer).start()
                await(context, press)
            }
            AWAIT -> await(context, intent.getIntExtra("press", 0))
            else -> super.onReceive(context, intent)
        }
    }
}

class LabelSwitchWidget : LabelWidget(Kind.SWITCH)

class LabelOnWidget : LabelWidget(Kind.ON)

class LabelOffWidget : LabelWidget(Kind.OFF)

class LabelToggleWidget : LabelWidget(Kind.TOGGLE)
