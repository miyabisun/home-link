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
 * Names the PC a placed PC widget sends to: picked from those connected now, or typed, as a PC not connected yet
 * may be named before it first connects. The launcher opens it on placing the widget and from its settings.
 */
class PcWidgetConfigActivity : Activity() {
    private var widget = AppWidgetManager.INVALID_APPWIDGET_ID
    private lateinit var api: HomeLinkApi
    private lateinit var reads: Reads
    private var pcs: List<Pc>? = null
    private var error: ApiError? = null
    /** The PC last drawn as picked; the select box reports it back without the user choosing. */
    private var picked: String? = null

    private lateinit var pcSpinner: Spinner
    private lateinit var pcAdapter: ArrayAdapter<String>
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
        val panel = panel(content)
        label(panel, "PC", 18, bold = true)
        pcAdapter = ArrayAdapter<String>(this, R.layout.select_item)
        pcSpinner = select(panel, pcAdapter, "PC") { pick(it) }
        nameField = field(panel, "名前", EditorInfo.IME_ACTION_DONE) { save() }
        nameField.addTextChangedListener(changed { render() })
        resultRow = ResultRow(this, content, dp(16))
        reloadButton = button(content, "再接続") { load() }
        saveButton = button(content, "保存", primary = true) { save() }

        val name = savedInstanceState?.getString("name") ?: PcWidget.name(this, widget).orEmpty()
        nameField.setText(name)
        picked = savedInstanceState?.getString("picked") ?: name.ifEmpty { null }
        load()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        if (::nameField.isInitialized) outState.putString("name", nameField.text.toString())
        picked?.let { outState.putString("picked", it) }
        super.onSaveInstanceState(outState)
    }

    override fun onDestroy() {
        if (::reads.isInitialized) reads.close()
        super.onDestroy()
    }

    private fun load() {
        pcs = null
        error = null
        render()
        reads.run({ api.pcs() }) { result ->
            when (result) {
                is ApiResult.Ok -> {
                    pcs = result.value
                    // The PC named already, else the first connected one.
                    val pc = result.value.firstOrNull { it.name == picked } ?: result.value.firstOrNull()
                    pc?.let { show(it.name) }
                }
                is ApiResult.Failed -> error = result.error
            }
            render()
        }
    }

    /** Draws the PC `name` as picked, naming the widget after it while no other name is typed. */
    private fun show(name: String) {
        nameField.setText(pcNameAfterPick(nameField.text.toString(), picked, name))
        picked = name
    }

    private fun pick(position: Int) {
        val pc = pcs?.getOrNull(position) ?: return
        if (pc.name == picked) return
        show(pc.name)
        render()
    }

    private fun save() {
        val name = nameField.text.toString().trim().takeIf { it.isNotEmpty() } ?: return
        PcWidget.save(this, widget, name)
        pcs?.let { PcWidget.saveStates(this, ApiResult.Ok(it)) }
        PcWidget.showAll(this)
        setResult(RESULT_OK, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, widget))
        finish()
    }

    private fun render() {
        val read = pcs.orEmpty()
        fill(pcAdapter, read.map { it.name })
        pcSpinner.visibility = if (read.isEmpty()) View.GONE else View.VISIBLE
        val index = read.indexOfFirst { it.name == picked }
        if (index >= 0 && pcSpinner.selectedItemPosition != index) pcSpinner.setSelection(index)
        when {
            error != null -> resultRow.show(failure(error!!, homeLinkHost(this)).text, failed = true)
            pcs?.isEmpty() == true -> resultRow.show("つながっているPCはありません", failed = false, progress = R.drawable.ic_power)
            else -> resultRow.show(null, failed = false)
        }
        reloadButton.visibility = if (error != null) View.VISIBLE else View.GONE
        saveButton.isEnabled = nameField.text.isNotBlank()
    }
}
