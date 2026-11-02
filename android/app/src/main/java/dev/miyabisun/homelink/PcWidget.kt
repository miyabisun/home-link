package dev.miyabisun.homelink

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.widget.RemoteViews

/**
 * The home-screen toggle for one PC connected to home-link, named when placed: a press puts it to sleep, or wakes it
 * while it sleeps. The state is the PC's, read when placed, after a press and every update period.
 */
class PcWidget : AppWidgetProvider() {
    companion object {
        private const val PREFS = "pc_widgets"
        private val flow = PressFlow()

        /** The PC a placed widget sends to; none until its setup was saved. */
        fun name(context: Context, id: Int): String? =
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString("$id.name", null)

        fun save(context: Context, id: Int, name: String) {
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString("$id.name", name).commit()
        }

        /** Each PC's state as last known by name; a PC missing is unknown. */
        private fun states(context: Context): Map<String, Boolean> {
            val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            fun names(key: String) = prefs.getStringSet(key, emptySet()).orEmpty()
            return names("awake").associateWith { false } + names("sleeping").associateWith { true }
        }

        private fun save(context: Context, states: Map<String, Boolean>) {
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
                .putStringSet("sleeping", states.filterValues { it }.keys)
                .putStringSet("awake", states.filterValues { !it }.keys).commit()
        }

        private fun save(context: Context, name: String, sleeping: Boolean?) {
            val known = states(context)
            save(context, if (sleeping == null) known - name else known + (name to sleeping))
        }

        private fun placed(context: Context): List<Int> =
            AppWidgetManager.getInstance(context).getAppWidgetIds(ComponentName(context, PcWidget::class.java)).toList()

        /** Keeps what a read of the PCs found for each placed widget's PC; a failed read keeps what was known. */
        fun saveStates(context: Context, read: ApiResult<List<Pc>>) {
            val known = states(context)
            val names = placed(context).mapNotNull { name(context, it) }.toSet()
            save(context, names.mapNotNull { name -> pcState(name, known[name], read)?.let { name to it } }.toMap())
        }

        /** Reads the PCs again within `limits`; whether they were read. Nothing placed reads nothing. */
        fun read(context: Context, limits: Limits = Limits.SCREEN): Boolean {
            if (placed(context).isEmpty()) return true
            val read = homeLinkApi(context, limits).pcs()
            saveStates(context, read)
            return read is ApiResult.Ok
        }

        /** Redraws every placed PC widget as last known. */
        fun showAll(context: Context) = placed(context).forEach { show(context, it) }

        private fun show(context: Context, id: Int, press: LightsMessage? = null, sending: Boolean = false, message: String? = null) {
            val views = RemoteViews(context.packageName, R.layout.label_widget)
            val name = name(context, id)
            when {
                name == null -> {
                    // Placed without a PC (the setup was left): the button opens the setup again.
                    views.setOnClickPendingIntent(R.id.label_button, PendingIntent.getActivity(context, id,
                        Intent(context, PcWidgetConfigActivity::class.java).putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id),
                        PendingIntent.FLAG_IMMUTABLE))
                    views.showFace(R.id.label_button, R.id.label_button_text, ButtonFace("", R.drawable.ic_settings, Tone.PLAIN))
                    views.setContentDescription(R.id.label_button, "設定")
                }
                Widgets.lanBlocked(context) -> {
                    views.setOnClickPendingIntent(R.id.label_button, Widgets.askLan(context))
                    views.showFace(R.id.label_button, R.id.label_button_text, Widgets.LAN_FACE)
                    views.setContentDescription(R.id.label_button, "$name：${Widgets.LAN_FACE.description}")
                }
                else -> {
                    views.setOnClickPendingIntent(R.id.label_button, PressService.pending(context, "pc:$id",
                        "${name}に送っています", R.drawable.ic_power) { putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id) })
                    val face = pcFace(name, states(context)[name], press, sending)
                    views.showFace(R.id.label_button, R.id.label_button_text, face)
                    views.setContentDescription(R.id.label_button, message?.let { "${face.text}：$it" } ?: face.description)
                }
            }
            AppWidgetManager.getInstance(context).updateAppWidget(id, views)
        }

        /**
         * Sends the pressed PC sleep, or wake while it sleeps, showing the press's progress and, for a while, a result
         * the button does not show as the new state; then reads the PCs again and takes the button back.
         */
        fun press(context: Context, intent: Intent) {
            val id = intent.getIntExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, AppWidgetManager.INVALID_APPWIDGET_ID)
            val name = name(context, id) ?: return
            flow.run(context, sending = { show(context, id, SENDING, sending = true) }, send = {
                val action = pcAction(states(context)[name])
                val result = homeLinkApi(context).switchPc(name, action)
                save(context, name, pressedState(states(context)[name], result))
                val short = pcPress(result)
                show(context, id, short, message = pcMessage(name, action, result, homeLinkHost(context)))
                short != null
            }, reread = { if (!read(context)) Widgets.refreshWhenOnline(context) }, restore = { showAll(context) })
        }
    }

    /** Draws the widgets as last known, then reads the PCs again: on placement and every update period. */
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        for (id in ids) show(context, id)
        async {
            if (!read(context, Limits.RECEIVER)) Widgets.refreshWhenOnline(context)
            showAll(context)
        }
    }

    override fun onDeleted(context: Context, ids: IntArray) {
        val edit = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
        for (id in ids) edit.remove("$id.name")
        edit.apply()
    }
}
