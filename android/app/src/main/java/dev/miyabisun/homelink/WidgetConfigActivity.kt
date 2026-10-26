package dev.miyabisun.homelink

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.content.Intent
import android.os.Bundle
import android.view.View
import android.view.inputmethod.EditorInfo
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.Spinner

/**
 * Picks the label a placed label widget sends to and the name it shows; the launcher opens it on placing
 * the widget and again from the widget's settings.
 */
class WidgetConfigActivity : Activity() {
    private var widget = AppWidgetManager.INVALID_APPWIDGET_ID
    private lateinit var api: HomeLinkApi
    private lateinit var reads: Reads
    private var labels: List<Label>? = null
    private var error: ApiError? = null
    /** The label last drawn as picked; the select box reports it back without the user choosing. */
    private var picked: Label? = null

    private lateinit var labelSpinner: Spinner
    private lateinit var labelAdapter: ArrayAdapter<String>
    private lateinit var nameField: EditText
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
        val labelPanel = panel(content)
        label(labelPanel, "ラベル", 18, bold = true)
        labelAdapter = ArrayAdapter<String>(this, R.layout.select_item)
        labelSpinner = select(labelPanel, labelAdapter, "ラベル") { pick(it) }
        val namePanel = panel(content)
        label(namePanel, "名前", 18, bold = true)
        nameField = field(namePanel, "名前", EditorInfo.IME_ACTION_DONE) { save() }
        resultRow = ResultRow(this, content, dp(16))
        reloadButton = button(content, "再接続") { load() }
        saveButton = button(content, "保存", primary = true) { save() }

        val saved = LabelWidget.config(this, widget)
        nameField.setText(savedInstanceState?.getString("name") ?: saved?.name.orEmpty())
        val pickId = savedInstanceState?.getLong("label", -1)?.takeIf { it >= 0 } ?: saved?.labelId
        load(pickId)
    }

    override fun onSaveInstanceState(outState: Bundle) {
        if (::nameField.isInitialized) outState.putString("name", nameField.text.toString())
        picked?.let { outState.putLong("label", it.id) }
        super.onSaveInstanceState(outState)
    }

    override fun onDestroy() {
        if (::reads.isInitialized) reads.close()
        super.onDestroy()
    }

    private fun load(pickId: Long? = picked?.id) {
        labels = null
        error = null
        render()
        reads.run({ api.labels() }) { result ->
            when (result) {
                is ApiResult.Ok -> {
                    labels = result.value
                    val index = result.value.indexOfFirst { it.id == pickId }.coerceAtLeast(0)
                    result.value.getOrNull(index)?.let { show(it) }
                }
                is ApiResult.Failed -> error = result.error
            }
            render()
        }
    }

    /** Draws `label` as picked, naming the widget after it while no other name is typed. */
    private fun show(label: Label) {
        nameField.setText(nameAfterPick(nameField.text.toString(), picked, label))
        picked = label
    }

    private fun pick(position: Int) {
        val label = labels?.getOrNull(position) ?: return
        if (label == picked) return
        show(label)
        render()
    }

    private fun save() {
        val label = picked ?: return
        LabelWidget.save(this, widget, WidgetConfig(label.id, widgetName(nameField.text.toString(), label)))
        LabelWidget.saveStates(this, ApiResult.Ok(labels.orEmpty()))
        LabelWidget.showAll(this)
        setResult(RESULT_OK, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, widget))
        finish()
    }

    private fun render() {
        val read = labels.orEmpty()
        fill(labelAdapter, read.map { it.name })
        val index = read.indexOf(picked)
        if (index >= 0 && labelSpinner.selectedItemPosition != index) labelSpinner.setSelection(index)
        nameField.hint = picked?.name.orEmpty()
        val host = homeLinkHost(this)
        when {
            error != null -> resultRow.show(failure(error!!, host).text, failed = true)
            labels?.isEmpty() == true -> resultRow.show("ラベルがありません", failed = true)
            else -> resultRow.show(null, failed = false)
        }
        reloadButton.visibility = if (error != null) View.VISIBLE else View.GONE
        saveButton.isEnabled = picked != null
    }
}
