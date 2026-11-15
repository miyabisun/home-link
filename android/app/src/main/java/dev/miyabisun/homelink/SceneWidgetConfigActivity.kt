package dev.miyabisun.homelink

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.content.Intent
import android.os.Bundle
import android.view.View
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.Spinner

/**
 * Picks the routine a placed routine widget runs, from those home-link has written. The launcher opens it on placing
 * the widget and from its settings.
 */
class SceneWidgetConfigActivity : Activity() {
    private var widget = AppWidgetManager.INVALID_APPWIDGET_ID
    private lateinit var api: HomeLinkApi
    private lateinit var reads: Reads
    private var scenes: List<Scene>? = null
    private var error: ApiError? = null
    private var picked: String? = null

    private lateinit var spinner: Spinner
    private lateinit var adapter: ArrayAdapter<String>
    private lateinit var resultRow: ResultRow
    private lateinit var reloadButton: Button
    private lateinit var saveButton: Button

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        widget = intent.getIntExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, AppWidgetManager.INVALID_APPWIDGET_ID)
        // Leaving without saving takes a widget being placed back off the home screen.
        setResult(RESULT_CANCELED, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, widget))
        // Only this app's own widgets are set up here.
        if (AppWidgetManager.getInstance(this).getAppWidgetInfo(widget)?.provider?.packageName != packageName) return finish()
        api = homeLinkApi(this)
        reads = Reads(this)

        val content = page()
        label(content, "ウィジェット", 28, bold = true)
        val panel = panel(content)
        label(panel, "操作", 18, bold = true)
        adapter = ArrayAdapter<String>(this, R.layout.select_item)
        spinner = select(panel, adapter, "操作") { position ->
            picked = scenes?.getOrNull(position)?.name ?: picked
            render()
        }
        resultRow = ResultRow(this, content, dp(16))
        reloadButton = button(content, "再接続") { load() }
        saveButton = button(content, "保存", primary = true) { save() }

        picked = savedInstanceState?.getString("picked") ?: SceneWidget.name(this, widget)
        load()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        picked?.let { outState.putString("picked", it) }
        super.onSaveInstanceState(outState)
    }

    override fun onDestroy() {
        if (::reads.isInitialized) reads.close()
        super.onDestroy()
    }

    private fun load() {
        scenes = null
        error = null
        render()
        reads.run({ api.scenes() }) { result ->
            when (result) {
                is ApiResult.Ok -> {
                    scenes = result.value
                    // The routine picked already, else the first written.
                    picked = (result.value.firstOrNull { it.name == picked } ?: result.value.firstOrNull())?.name
                }
                is ApiResult.Failed -> error = result.error
            }
            render()
        }
    }

    private fun save() {
        val name = picked?.takeIf { name -> scenes?.any { it.name == name } == true } ?: return
        SceneWidget.save(this, widget, name)
        SceneWidget.showAll(this)
        setResult(RESULT_OK, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, widget))
        finish()
    }

    private fun render() {
        val read = scenes.orEmpty()
        fill(adapter, read.map { it.name })
        spinner.visibility = if (read.isEmpty()) View.GONE else View.VISIBLE
        val index = read.indexOfFirst { it.name == picked }
        if (index >= 0 && spinner.selectedItemPosition != index) spinner.setSelection(index)
        when {
            error != null -> resultRow.show(failure(error!!, homeLinkHost(this)).text, failed = true)
            scenes?.isEmpty() == true -> resultRow.show("home-linkに操作が書かれていません", failed = true)
            else -> resultRow.show(null, failed = false)
        }
        reloadButton.visibility = if (error != null) View.VISIBLE else View.GONE
        saveButton.isEnabled = index >= 0
    }
}
