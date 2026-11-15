package dev.miyabisun.homelink

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.widget.RemoteViews

/**
 * The home-screen button for one of home-link's routines, such as "おやすみ" or "帰宅", picked when placed. A press runs
 * it; a routine that waits for the phone to come near home shows that until it does, and a second press takes it back.
 */
class SceneWidget : AppWidgetProvider() {
    companion object {
        private const val PREFS = "scene_widgets"

        /** The routine a placed widget runs; none until its setup was saved. */
        fun name(context: Context, id: Int): String? =
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString("$id.name", null)

        fun save(context: Context, id: Int, name: String) {
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString("$id.name", name).apply()
        }

        private fun placed(context: Context): List<Int> =
            AppWidgetManager.getInstance(context).getAppWidgetIds(ComponentName(context, SceneWidget::class.java)).toList()

        /** Redraws every placed routine widget: its name, or that it waits. */
        fun showAll(context: Context) = placed(context).forEach { show(context, it) }

        /** Draws widget `id`: a press's short `press` label with `message` read aloud, else the button. */
        fun show(context: Context, id: Int, press: LightsMessage? = null, sending: Boolean = false, message: String? = null) {
            val views = RemoteViews(context.packageName, R.layout.label_widget)
            val name = name(context, id)
            when {
                name == null -> {
                    // Placed without a routine (the setup was left): the button opens the setup again.
                    views.setOnClickPendingIntent(R.id.label_button, PendingIntent.getActivity(context, id,
                        Intent(context, SceneWidgetConfigActivity::class.java).putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id),
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
                    views.setOnClickPendingIntent(R.id.label_button, SceneService.pending(context, id))
                    val face = sceneFace(name, press, sending, waiting = press == null && SceneService.waiting(id))
                    views.showFace(R.id.label_button, R.id.label_button_text, face)
                    views.setContentDescription(R.id.label_button, message?.let { "$name：$it" } ?: face.description)
                }
            }
            AppWidgetManager.getInstance(context).updateAppWidget(id, views)
        }
    }

    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        for (id in ids) show(context, id)
    }

    override fun onDeleted(context: Context, ids: IntArray) {
        val edit = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
        for (id in ids) edit.remove("$id.name")
        edit.apply()
    }
}
