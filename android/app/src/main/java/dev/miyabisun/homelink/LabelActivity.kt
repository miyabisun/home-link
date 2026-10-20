package dev.miyabisun.homelink

import android.app.Activity
import android.app.TimePickerDialog
import android.content.Context
import android.content.Intent
import android.content.res.ColorStateList
import android.os.Bundle
import android.text.InputType
import android.view.View
import android.view.inputmethod.EditorInfo
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.Spinner
import android.widget.TextView
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

/**
 * Edits one label: its name, its devices, how it dims (following the schedule, held at fixed values, or neither)
 * and its daily switches. Every change is sent as it is made.
 */
class LabelActivity : Activity() {
    companion object {
        private const val LABEL = "label"
        /** Defaults for fixed values: the day's 80% and 5000 K. */
        private const val FIXED_PERCENT = 80
        private const val FIXED_KELVIN = 5000
        private val DIMMING = listOf("なし", "自動調光", "固定")
        /** Instrumentation picks a time here instead of in the system's dialog. */
        internal var timePicker: ((Activity, Int, (Int) -> Unit) -> Unit)? = null

        fun intent(context: Context, id: Long): Intent = Intent(context, LabelActivity::class.java).putExtra(LABEL, id)
    }

    private var labelId = -1L
    private lateinit var api: HomeLinkApi
    private lateinit var worker: ExecutorService
    private var label: Label? = null
    private var devices: List<Device> = emptyList()
    private var loadError: ApiError? = null
    private var error: ApiError? = null
    private var nameError: String? = null
    private var overrideErrors: Set<OverrideError> = emptySet()
    private var clash = false
    /** The dimming position last drawn; the select box reports it back without the user choosing. */
    private var shownDimming = -1

    private lateinit var nameField: EditText
    private lateinit var nameMessage: TextView
    private lateinit var devicePanel: LinearLayout
    private lateinit var deviceBoxes: LinearLayout
    private lateinit var dimmingSpinner: Spinner
    private lateinit var fixedPanel: LinearLayout
    private lateinit var percentField: EditText
    private lateinit var percentMessage: TextView
    private lateinit var kelvinField: EditText
    private lateinit var kelvinMessage: TextView
    private lateinit var scheduleRows: LinearLayout
    private lateinit var scheduleMessage: TextView
    private lateinit var resultRow: ResultRow
    private lateinit var reloadButton: Button

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        labelId = intent.getLongExtra(LABEL, -1)
        api = homeLinkApi(this)
        worker = Executors.newSingleThreadExecutor()
        build()
        load(savedInstanceState)
    }

    override fun onPause() {
        saveName()
        saveFixed()
        super.onPause()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putString("name", nameField.text.toString())
        outState.putString("percent", percentField.text.toString())
        outState.putString("kelvin", kelvinField.text.toString())
        super.onSaveInstanceState(outState)
    }

    override fun onDestroy() {
        worker.shutdownNow()
        super.onDestroy()
    }

    private fun <T> background(call: () -> T, done: (T) -> Unit) {
        worker.execute {
            val result = call()
            runOnUiThread { if (!isDestroyed) done(result) }
        }
    }

    private fun load(state: Bundle? = null) {
        loadError = null
        background({ api.labels() to api.devices() }) { (labels, ledger) ->
            when {
                labels is ApiResult.Failed -> loadError = labels.error
                ledger is ApiResult.Failed -> loadError = ledger.error
                labels is ApiResult.Ok && ledger is ApiResult.Ok -> {
                    val read = labels.value.firstOrNull { it.id == labelId } ?: return@background finish()
                    devices = ledger.value
                    shown(read)
                    nameField.setText(state?.getString("name") ?: read.name)
                    state?.getString("percent")?.let { percentField.setText(it) }
                    state?.getString("kelvin")?.let { kelvinField.setText(it) }
                }
            }
            render()
        }
    }

    /** Takes `read` as the label, with the fixed values it holds in the fields. */
    private fun shown(read: Label) {
        label = read
        read.override?.let {
            percentField.setText(digits(percentOf(it.level)))
            kelvinField.setText(digits(it.kelvin))
        }
    }

    private fun update(change: LabelUpdate) {
        error = null
        background({ api.updateLabel(labelId, change) }) { result ->
            when (result) {
                is ApiResult.Ok -> label = result.value
                is ApiResult.Failed -> when (result.error) {
                    ApiError.DUPLICATE_LABEL -> nameError = "同じ名前のラベルがあります"
                    ApiError.INVALID_NAME -> nameError = "名前は100文字以内で入力してください"
                    ApiError.LABEL_NOT_FOUND -> return@background finish()
                    else -> error = result.error
                }
            }
            render()
        }
    }

    /** Sends the typed name once it differs from the label's. */
    private fun saveName() {
        val current = label ?: return
        val name = nameField.text.toString().trim()
        if (name == current.name) {
            nameError = null
            return render()
        }
        nameError = if (name.isEmpty()) "名前を入力してください" else null
        render()
        if (name.isNotEmpty()) update(LabelUpdate.Name(name))
    }

    /** Sends typed fixed values once they are valid and differ from the label's. */
    private fun saveFixed() {
        val current = label ?: return
        if (current.override == null) return
        overrideErrors = overrideErrors(percentField.text.toString(), kelvinField.text.toString())
        render()
        val fixed = overrideOf(percentField.text.toString(), kelvinField.text.toString()) ?: return
        if (fixed != current.override) update(LabelUpdate.Dimming(auto = false, override = fixed))
    }

    private fun dim(position: Int) {
        val current = label ?: return
        if (position == shownDimming) return
        shownDimming = position
        overrideErrors = emptySet()
        val change = when (position) {
            1 -> LabelUpdate.Dimming(auto = true, override = null)
            2 -> LabelUpdate.Dimming(auto = false, override = overrideOf(percentField.text.toString(), kelvinField.text.toString())
                ?: Override(levelOf(FIXED_PERCENT), FIXED_KELVIN).also {
                    percentField.setText(digits(FIXED_PERCENT))
                    kelvinField.setText(digits(FIXED_KELVIN))
                })
            else -> LabelUpdate.Dimming(auto = false, override = null)
        }
        if (change != LabelUpdate.Dimming(current.auto, current.override)) update(change)
        render()
    }

    private fun assign(device: Device, on: Boolean) {
        error = null
        val ids = if (on) device.labelIds + labelId else device.labelIds - labelId
        background({ api.updateDevice(device.id, DeviceUpdate.Labels(ids)) }) { result ->
            when (result) {
                is ApiResult.Ok -> devices = devices.map { if (it.id == device.id) result.value else it }
                is ApiResult.Failed -> error = result.error
            }
            render()
        }
    }

    /** Sends the schedule with `entry` at `index`, or added, unless another switch is at its time. */
    private fun schedule(entry: ScheduleEntry, index: Int? = null) {
        val current = label ?: return
        val entries = scheduleWith(current.schedule, entry, index)
        clash = entries == null
        render()
        if (entries != null) update(LabelUpdate.Schedule(entries))
    }

    private fun unschedule(index: Int) {
        val current = label ?: return
        clash = false
        update(LabelUpdate.Schedule(current.schedule.filterIndexed { i, _ -> i != index }))
    }

    private fun pickTime(minute: Int, picked: (Int) -> Unit) {
        timePicker?.let { return it(this, minute, picked) }
        TimePickerDialog(this, { _, hour, min -> picked(hour * 60 + min) }, minute / 60, minute % 60, true).show()
    }

    private fun delete() {
        error = null
        background({ api.deleteLabel(labelId) }) { result ->
            if (result is ApiResult.Failed && result.error != ApiError.LABEL_NOT_FOUND) {
                error = result.error
                render()
            } else finish()
        }
    }

    private fun build() {
        val content = page()
        label(content, "ラベル", 28, bold = true)

        val name = panel(content)
        label(name, "名前", 18, bold = true)
        nameField = field(name, "名前", EditorInfo.IME_ACTION_DONE) { saveName() }
        nameField.setOnFocusChangeListener { _, focused -> if (!focused) saveName() }
        nameMessage = message(name)

        devicePanel = panel(content)
        label(devicePanel, "機器", 18, bold = true)
        deviceBoxes = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        devicePanel.addView(deviceBoxes, LinearLayout.LayoutParams(-1, -2))

        val dimming = panel(content)
        label(dimming, "調光", 18, bold = true)
        dimmingSpinner = select(dimming, ArrayAdapter<String>(this, R.layout.select_item).apply { addAll(DIMMING) }, "調光") { dim(it) }
        fixedPanel = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        dimming.addView(fixedPanel, LinearLayout.LayoutParams(-1, -2))
        label(fixedPanel, "明るさ（%）", 14, muted = true)
        percentField = number(fixedPanel, "明るさ（%）")
        percentMessage = message(fixedPanel)
        label(fixedPanel, "色温度（K）", 14, muted = true)
        kelvinField = number(fixedPanel, "色温度（K）")
        kelvinMessage = message(fixedPanel)

        val schedule = panel(content)
        label(schedule, "スケジュール", 18, bold = true)
        scheduleRows = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        schedule.addView(scheduleRows, LinearLayout.LayoutParams(-1, -2))
        scheduleMessage = message(schedule)
        button(schedule, "追加", icon = R.drawable.ic_add) {
            pickTime(label?.schedule?.lastOrNull()?.minute ?: 0) { schedule(ScheduleEntry(it, on = true)) }
        }

        resultRow = ResultRow(this, content, dp(16))
        reloadButton = button(content, "再接続") { load() }
        button(content, "このラベルを削除") { delete() }.apply {
            (layoutParams as LinearLayout.LayoutParams).topMargin = dp(24)
        }
        render()
    }

    /** Digits as typed in the number fields, whatever the locale. */
    private fun digits(value: Int) = "%d".format(java.util.Locale.ROOT, value)

    private fun number(parent: LinearLayout, description: String): EditText =
        field(parent, description, EditorInfo.IME_ACTION_DONE, maxLength = 5) { saveFixed() }.apply {
            inputType = InputType.TYPE_CLASS_NUMBER
            setOnFocusChangeListener { _, focused -> if (!focused) saveFixed() }
        }

    private fun render() {
        val current = label
        val host = homeLinkHost(this)
        show(nameMessage, nameError)
        renderDevices()
        val dimming = when {
            current == null -> -1
            current.auto -> 1
            current.override != null -> 2
            else -> 0
        }
        if (dimming >= 0 && dimming != shownDimming) {
            shownDimming = dimming
            dimmingSpinner.setSelection(dimming)
        }
        fixedPanel.visibility = if (shownDimming == 2) View.VISIBLE else View.GONE
        show(percentMessage, if (OverrideError.LEVEL in overrideErrors) "1〜100で入力してください" else null)
        show(kelvinMessage, if (OverrideError.KELVIN in overrideErrors) "1000〜10000で入力してください" else null)
        renderSchedule(current?.schedule.orEmpty())
        show(scheduleMessage, if (clash) "同じ時刻があります" else null)
        val shown = loadError ?: error
        resultRow.show(shown?.let { failure(it, host).text }, failed = true)
        reloadButton.visibility = if (loadError != null) View.VISIBLE else View.GONE
    }

    /** One check box per light or plug, and per device already given the label. */
    private fun renderDevices() {
        val listed = devices.filter { it.kind == DeviceKind.LIGHT || it.kind == DeviceKind.PLUG || labelId in it.labelIds }
        devicePanel.visibility = if (listed.isEmpty()) View.GONE else View.VISIBLE
        val titles = listed.map { it.title }
        if (deviceBoxes.childCount != listed.size ||
            titles.withIndex().any { (i, title) -> (deviceBoxes.getChildAt(i) as CheckBox).text != title }) {
            deviceBoxes.removeAllViews()
            for (title in titles) deviceBoxes.addView(CheckBox(this).apply {
                text = title
                textSize = 16f
                minHeight = dp(48)
                setTextColor(getColor(R.color.text))
                buttonTintList = ColorStateList.valueOf(getColor(R.color.accent))
            })
        }
        listed.forEachIndexed { i, device ->
            (deviceBoxes.getChildAt(i) as CheckBox).apply {
                isChecked = labelId in device.labelIds
                // Clicks, not checked changes, so drawing the assignments sends nothing.
                setOnClickListener { assign(device, isChecked) }
            }
        }
    }

    /** A row per daily switch: its time, opening a picker, whether it switches on, and its removal. */
    private fun renderSchedule(entries: List<ScheduleEntry>) {
        if (scheduleRows.childCount != entries.size) {
            scheduleRows.removeAllViews()
            repeat(entries.size) {
                val row = LinearLayout(this).apply {
                    orientation = LinearLayout.HORIZONTAL
                    isBaselineAligned = false
                }
                scheduleRows.addView(row, LinearLayout.LayoutParams(-1, -2))
                button(row, "", weight = true) { }
                button(row, "", weight = true) { }
                button(row, "", icon = R.drawable.ic_delete) { }.apply {
                    layoutParams = LinearLayout.LayoutParams(dp(48), -2).apply { topMargin = dp(8) }
                    compoundDrawablePadding = 0
                    setPadding(dp(12), dp(12), dp(12), dp(12))
                }
            }
        }
        entries.forEachIndexed { i, entry ->
            val row = scheduleRows.getChildAt(i) as LinearLayout
            (row.getChildAt(0) as Button).apply {
                text = clock(entry.minute)
                setOnClickListener { pickTime(entry.minute) { schedule(entry.copy(minute = it), i) } }
            }
            (row.getChildAt(1) as Button).apply {
                paintSwitch(this, if (entry.on) "オン" else "オフ", entry.on)
                contentDescription = "${clock(entry.minute)} ${text}"
                setOnClickListener { schedule(entry.copy(on = !entry.on), i) }
            }
            row.getChildAt(2).apply {
                contentDescription = "${clock(entry.minute)}を削除"
                setOnClickListener { unschedule(i) }
            }
        }
    }
}
