package dev.miyabisun.homelink

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.res.ColorStateList
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.LayerDrawable
import android.graphics.drawable.RippleDrawable
import android.graphics.drawable.StateListDrawable
import android.net.Uri
import android.os.Bundle
import android.text.Editable
import android.text.InputFilter
import android.text.InputType
import android.text.TextWatcher
import android.view.Gravity
import android.view.View
import android.view.WindowInsets
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

class MainActivity : Activity() {
    companion object {
        // Instrumentation supplies fakes here; there is no user-facing mock mode.
        internal var apiFactory: (() -> HomeLinkApi)? = null
        internal var scannerFactory: ((Activity) -> QrScanner)? = null
        private const val MAX_NAME = 100
    }

    private val form = RegisterForm()
    private lateinit var api: HomeLinkApi
    private lateinit var scanner: QrScanner
    private lateinit var worker: ExecutorService
    private lateinit var lightsWorker: ExecutorService
    /** The light switch in flight: true for on, false for off. */
    private var switching: Boolean? = null
    private var lightsResult: LightsMessage? = null
    /** Offer to place the widget while the launcher can pin one and none is placed. */
    private var canPinWidget = false
    private var loadingRooms = false
    private var addingRoom = false
    private var enteringCode = false
    private var formattingCode = false

    private lateinit var content: LinearLayout
    private lateinit var lightsOnButton: Button
    private lateinit var lightsOffButton: Button
    private lateinit var lightsRow: ResultRow
    private lateinit var pinButton: Button
    private lateinit var qrState: TextView
    private lateinit var qrMessage: TextView
    private lateinit var scanButton: Button
    private lateinit var manualButton: Button
    private lateinit var manualPanel: LinearLayout
    private lateinit var manualField: EditText
    private lateinit var manualMessage: TextView
    private lateinit var roomSpinner: Spinner
    private lateinit var roomAdapter: ArrayAdapter<String>
    private lateinit var roomState: TextView
    private lateinit var roomMessage: TextView
    private lateinit var addRoomButton: Button
    private lateinit var newRoomPanel: LinearLayout
    private lateinit var newRoomName: EditText
    private lateinit var createRoomButton: Button
    private lateinit var nameField: EditText
    private lateinit var resultRow: ResultRow
    private lateinit var reconnectButton: Button
    private lateinit var registerButton: Button

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        api = apiFactory?.invoke() ?: HttpHomeLinkApi(BuildConfig.HOME_LINK_URL)
        scanner = scannerFactory?.invoke(this) ?: GmsQrScanner(this)
        worker = Executors.newSingleThreadExecutor()
        // Switching lights can wait seconds for every bulb; registration does not queue behind it.
        lightsWorker = Executors.newSingleThreadExecutor()
        savedInstanceState?.let { state ->
            form.payload = state.getString("payload")
            form.roomId = state.getLong("roomId", -1).takeIf { it >= 0 }
            form.name = state.getString("name").orEmpty()
            addingRoom = state.getBoolean("addingRoom")
            enteringCode = state.getBoolean("enteringCode")
            lightsResult = state.getString("lightsText")?.let { LightsMessage(it, state.getBoolean("lightsFailed")) }
        }
        build()
        manualField.setText(savedInstanceState?.getString("manualCode").orEmpty())
        manualField.setSelection(manualField.length())
        newRoomName.setText(savedInstanceState?.getString("newRoom").orEmpty())
        nameField.setText(form.name)
        loadRooms()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putString("payload", form.payload)
        outState.putLong("roomId", form.roomId ?: -1)
        outState.putString("name", form.name)
        outState.putBoolean("addingRoom", addingRoom)
        outState.putString("newRoom", newRoomName.text.toString())
        outState.putBoolean("enteringCode", enteringCode)
        outState.putString("manualCode", manualField.text.toString())
        lightsResult?.let {
            outState.putString("lightsText", it.text)
            outState.putBoolean("lightsFailed", it.failed)
        }
        super.onSaveInstanceState(outState)
    }

    override fun onResume() {
        super.onResume()
        // A widget may have been placed meanwhile.
        val manager = AppWidgetManager.getInstance(this)
        canPinWidget = manager.isRequestPinAppWidgetSupported &&
            manager.getAppWidgetIds(ComponentName(this, LightsWidget::class.java)).isEmpty()
        render()
    }

    override fun onDestroy() {
        worker.shutdownNow()
        lightsWorker.shutdownNow()
        super.onDestroy()
    }

    /** Runs a blocking API call off the main thread and drops results for a destroyed screen. */
    private fun <T> background(call: () -> T, done: (T) -> Unit) = background(worker, call, done)

    private fun <T> background(executor: ExecutorService, call: () -> T, done: (T) -> Unit) {
        executor.execute {
            val result = call()
            runOnUiThread { if (!isDestroyed) done(result) }
        }
    }

    private fun loadRooms() {
        loadingRooms = true
        render()
        background({ api.rooms() }) { result ->
            loadingRooms = false
            form.rooms(result)
            render()
        }
    }

    private fun switchLights(on: Boolean) {
        switching = on
        lightsResult = null
        render()
        val host = Uri.parse(BuildConfig.HOME_LINK_URL).authority.orEmpty()
        background(lightsWorker, { api.switchLights(on) }) { result ->
            switching = null
            lightsResult = lightsMessage(on, result, host, names = true)
            render()
            LightsWidget.show(this, lightsMessage(on, result, host, names = false))
        }
    }

    private fun pinWidget() {
        AppWidgetManager.getInstance(this).requestPinAppWidget(ComponentName(this, LightsWidget::class.java), null, null)
    }

    private fun scan() {
        scanner.scan { result ->
            if (isDestroyed) return@scan
            when (result) {
                is ScanResult.Read -> {
                    form.scanned(result.value)
                    if (form.status == null) closeManual()
                }
                ScanResult.Cancelled -> Unit
                ScanResult.Failed -> form.status = Status.ScanFailed
            }
            render()
        }
    }

    private fun openManual() {
        enteringCode = true
        form.status = null
        render()
        manualField.requestFocus()
        getSystemService(InputMethodManager::class.java).showSoftInput(manualField, 0)
    }

    private fun closeManual() {
        enteringCode = false
        form.manualError = null
        manualField.setText("")
        getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(manualField.windowToken, 0)
    }

    /** Shows the digits grouped 4-3-4 and takes the code as soon as it is complete and valid. */
    private fun manualChanged(text: Editable) {
        if (formattingCode) return
        val shown = ManualCode.format(ManualCode.digits(text.toString()))
        if (text.toString() != shown) {
            val before = ManualCode.digits(text.substring(0, manualField.selectionEnd.coerceIn(0, text.length))).length
            formattingCode = true
            text.replace(0, text.length, shown)
            formattingCode = false
            manualField.setSelection(ManualCode.cursor(shown, before))
        }
        if (form.typed(shown)) closeManual()
        render()
    }

    private fun submitManual() {
        if (form.submitted(manualField.text.toString())) closeManual()
        render()
    }

    private fun createRoom() {
        val name = newRoomName.text.toString().trim()
        if (name.isEmpty()) return
        createRoomButton.isEnabled = false
        background({ api.createRoom(name) }) { result ->
            form.roomCreated(result)
            if (result is ApiResult.Ok) {
                addingRoom = false
                newRoomName.setText("")
            }
            render()
        }
    }

    private fun register() {
        val payload = form.payload ?: return
        val roomId = form.roomId ?: return
        form.busy = true
        form.status = null
        render()
        background({ api.register(roomId, payload, form.name.trim()) }) { result ->
            form.registered(result)
            if (result is ApiResult.Ok) nameField.setText("")
            render()
            if (result == ApiResult.Failed(ApiError.ROOM_NOT_FOUND)) loadRooms()
        }
    }

    private fun build() {
        val scroll = ScrollView(this).apply {
            setBackgroundColor(getColor(R.color.background))
            isFillViewport = true
            setOnApplyWindowInsetsListener { view, insets ->
                val bars = insets.getInsets(WindowInsets.Type.systemBars() or
                    WindowInsets.Type.displayCutout() or WindowInsets.Type.ime())
                view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
                insets
            }
        }
        content = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(16), dp(16), dp(24))
        }
        scroll.addView(content)
        setContentView(scroll)

        label(content, "照明", 28, bold = true)
        val lights = panel()
        val lightActions = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        lights.addView(lightActions, LinearLayout.LayoutParams(-1, -2))
        lightsOnButton = button(lightActions, "照明オン", icon = R.drawable.ic_light_on, weight = true) { switchLights(true) }
        lightsOffButton = button(lightActions, "照明オフ", icon = R.drawable.ic_light_off, weight = true) { switchLights(false) }
        lightsRow = ResultRow(lights, dp(8))
        pinButton = button(lights, "ホーム画面にボタンを置く", icon = R.drawable.ic_widget) { pinWidget() }

        label(content, "機器を登録", 28, bold = true).apply {
            (layoutParams as LinearLayout.LayoutParams).topMargin = dp(24)
        }

        val qr = panel()
        label(qr, "機器のコード", 18, bold = true)
        qrState = label(qr, "", 16)
        qrMessage = message(qr)
        scanButton = button(qr, "", icon = R.drawable.ic_qr) { scan() }
        manualButton = button(qr, "", icon = R.drawable.ic_keypad) { openManual() }
        manualPanel = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        qr.addView(manualPanel, LinearLayout.LayoutParams(-1, -2))
        label(manualPanel, "機器に印字された11桁の数字", 14, muted = true)
        manualField = field(manualPanel, "11桁の数字", EditorInfo.IME_ACTION_DONE) { submitManual() }
        // The phone keypad shows digits and, unlike the number class, keeps the grouping spaces.
        manualField.inputType = InputType.TYPE_CLASS_PHONE
        manualField.importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
        manualField.hint = "0000 000 0000"
        manualField.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}
            override fun afterTextChanged(s: Editable) = manualChanged(s)
        })
        manualMessage = message(manualPanel)
        button(manualPanel, "やめる") {
            closeManual()
            render()
        }

        val room = panel()
        label(room, "部屋", 18, bold = true)
        roomState = label(room, "", 16)
        roomAdapter = ArrayAdapter<String>(this, android.R.layout.simple_spinner_item).apply {
            setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
        }
        roomSpinner = Spinner(this).apply {
            adapter = roomAdapter
            contentDescription = "部屋"
            minimumHeight = dp(48)
            background = LayerDrawable(arrayOf(surface(R.color.background), getDrawable(R.drawable.ic_expand))).apply {
                setLayerGravity(1, Gravity.END or Gravity.CENTER_VERTICAL)
                setLayerInsetEnd(1, dp(12))
            }
            setPaddingRelative(0, 0, dp(40), 0)
            onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
                override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                    form.rooms.getOrNull(position)?.let { form.roomId = it.id }
                    render()
                }
                override fun onNothingSelected(parent: AdapterView<*>?) {}
            }
        }
        room.addView(roomSpinner, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(8) })
        roomMessage = message(room)
        addRoomButton = button(room, "部屋を追加", icon = R.drawable.ic_add) {
            addingRoom = true
            form.status = null
            render()
            newRoomName.requestFocus()
        }
        newRoomPanel = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        room.addView(newRoomPanel, LinearLayout.LayoutParams(-1, -2))
        label(newRoomPanel, "新しい部屋の名前", 14, muted = true)
        newRoomName = field(newRoomPanel, "新しい部屋の名前", EditorInfo.IME_ACTION_DONE) { createRoom() }
        newRoomName.addTextChangedListener(changed { render() })
        val roomActions = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        newRoomPanel.addView(roomActions, LinearLayout.LayoutParams(-1, -2))
        button(roomActions, "やめる", weight = true) {
            addingRoom = false
            newRoomName.setText("")
            render()
        }
        createRoomButton = button(roomActions, "追加", weight = true) { createRoom() }

        val device = panel()
        label(device, "機器名（任意）", 18, bold = true)
        nameField = field(device, "機器名（任意）", EditorInfo.IME_ACTION_DONE) { if (form.canRegister()) register() }
        nameField.hint = "空欄でも登録できます"
        nameField.addTextChangedListener(changed { form.name = it })

        resultRow = ResultRow(content, dp(16))
        reconnectButton = button(content, "再接続") { loadRooms() }
        registerButton = button(content, "登録", primary = true) { register() }
        render()
    }

    private fun render() {
        val busy = switching != null
        lightsOnButton.isEnabled = !busy
        lightsOffButton.isEnabled = !busy
        when (switching) {
            true -> lightsRow.show("照明をオンにしています…", failed = false, R.drawable.ic_light_on)
            false -> lightsRow.show("照明をオフにしています…", failed = false, R.drawable.ic_light_off)
            null -> lightsRow.show(lightsResult?.text, lightsResult?.failed == true)
        }
        pinButton.visibility = if (canPinWidget) View.VISIBLE else View.GONE

        val payload = form.payload
        val scannedQr = payload?.startsWith("MT:") == true
        qrState.text = when {
            payload == null -> "まだ読み取っていません"
            scannedQr -> "読み取り済み：${preview(payload)}"
            else -> "入力済み：${preview(payload)}"
        }
        qrState.setTextColor(getColor(if (payload == null) R.color.muted else R.color.text))
        scanButton.text = if (scannedQr) "読み取り直す" else "QRを読み取る"
        manualButton.text = if (payload != null && !scannedQr) "数字を入力し直す" else "数字で入力"
        manualButton.visibility = if (enteringCode) View.GONE else View.VISIBLE
        manualPanel.visibility = if (enteringCode) View.VISIBLE else View.GONE
        show(qrMessage, when (form.status) {
            Status.NotMatter -> "MatterのQRコードではありません。QRコードが無い機器は「数字で入力」を使ってください"
            Status.ScanFailed -> "QRコードを読み取れませんでした。もう一度お試しください"
            else -> null
        })
        val typed = ManualCode.digits(manualField.text.toString()).length
        show(manualMessage, when (form.manualError) {
            ManualError.LENGTH -> "11桁の数字を入力してください（あと${ManualCode.LENGTH - typed}桁）"
            ManualError.CHECK_DIGIT -> "数字が正しくありません。機器に印字された数字と見比べてください"
            null -> null
        })

        val names = form.rooms.map { it.name }
        if (roomAdapter.count != names.size || (0 until roomAdapter.count).any { roomAdapter.getItem(it) != names[it] }) {
            roomAdapter.clear()
            roomAdapter.addAll(names)
        }
        val selected = form.rooms.indexOfFirst { it.id == form.roomId }
        if (selected >= 0 && roomSpinner.selectedItemPosition != selected) roomSpinner.setSelection(selected)
        roomSpinner.visibility = if (names.isEmpty()) View.GONE else View.VISIBLE
        roomState.text = when {
            loadingRooms && names.isEmpty() -> "部屋を読み込み中…"
            names.isEmpty() && form.roomsFailed -> "部屋を読み込めませんでした"
            names.isEmpty() -> "部屋がありません。「部屋を追加」から作成してください"
            else -> ""
        }
        roomState.visibility = if (roomState.text.isEmpty()) View.GONE else View.VISIBLE
        val status = form.status
        show(roomMessage, when {
            status is Status.RoomAdded -> "部屋「${status.room}」を追加しました"
            status == Status.Failed(ApiError.DUPLICATE_ROOM) -> "同じ名前の部屋があります。一覧から選んでください"
            else -> null
        }, failure = status is Status.Failed)
        addRoomButton.visibility = if (addingRoom) View.GONE else View.VISIBLE
        newRoomPanel.visibility = if (addingRoom) View.VISIBLE else View.GONE
        createRoomButton.isEnabled = newRoomName.text.isNotBlank()

        val result = when (status) {
            is Status.Registered -> if (status.name.isEmpty()) "${status.room}に機器を登録しました"
                else "${status.room}に「${status.name}」を登録しました"
            is Status.Failed -> when (status.error) {
                ApiError.UNREACHABLE -> "home-link（${Uri.parse(BuildConfig.HOME_LINK_URL).authority}）に接続できません。" +
                    "Tailscaleの接続を確認してください"
                ApiError.DUPLICATE_QR -> "この機器は登録済みです"
                ApiError.ROOM_NOT_FOUND -> "選んだ部屋が見つかりません。部屋を選び直してください"
                ApiError.INVALID_QR -> "MatterのQRコードとして読み取れませんでした。読み取り直してください"
                ApiError.INVALID_MANUAL -> "この数字はMatterの機器のコードとして使えません。数字を入力し直してください"
                ApiError.INVALID_NAME -> "名前は${MAX_NAME}文字以内で入力してください"
                ApiError.SERVER, ApiError.MATTER_UNREACHABLE, ApiError.MATTER_NOT_CONFIGURED ->
                    "home-linkでエラーが発生しました。時間をおいてお試しください"
                ApiError.DUPLICATE_ROOM -> null
            }
            else -> null
        }
        resultRow.show(result, status is Status.Failed)
        reconnectButton.visibility =
            if ((form.roomsFailed || status == Status.Failed(ApiError.UNREACHABLE)) && !loadingRooms) View.VISIBLE
            else View.GONE
        registerButton.isEnabled = form.canRegister()
        registerButton.text = if (form.busy) "登録中…" else "登録"
    }

    /** A result line: an icon and text on a surface that turns to the failure colors. */
    private inner class ResultRow(parent: LinearLayout, top: Int) {
        private val panel = LinearLayout(this@MainActivity).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(12), dp(8), dp(12), dp(8))
            accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
            parent.addView(this, LinearLayout.LayoutParams(-1, -2).apply { topMargin = top })
        }
        private val icon = ImageView(this@MainActivity).apply {
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            panel.addView(this, LinearLayout.LayoutParams(dp(24), dp(24)).apply { marginEnd = dp(8) })
        }
        private val text = TextView(this@MainActivity).apply {
            textSize = 16f
            setTextColor(getColor(R.color.text))
            panel.addView(this, LinearLayout.LayoutParams(0, -2, 1f))
        }

        fun show(message: String?, failed: Boolean, progress: Int = 0) {
            panel.visibility = if (message == null) View.GONE else View.VISIBLE
            if (message == null) return
            text.text = message
            icon.setImageResource(when {
                progress != 0 -> progress
                failed -> R.drawable.ic_error
                else -> R.drawable.ic_success
            })
            panel.background = GradientDrawable().apply {
                setColor(getColor(if (failed) R.color.danger_subtle else R.color.surface))
                cornerRadius = dp(8).toFloat()
                setStroke(dp(1), getColor(if (failed) R.color.danger else R.color.border))
            }
        }
    }

    /** Shows only the start of the code: both forms carry the setup passcode. */
    private fun preview(payload: String) =
        if (payload.startsWith("MT:")) payload.take(8) + "…" else payload.take(4) + " …"

    private fun show(view: TextView, text: String?, failure: Boolean = true) {
        view.text = text.orEmpty()
        view.visibility = if (text == null) View.GONE else View.VISIBLE
        view.setTextColor(getColor(if (failure) R.color.danger else R.color.text))
    }

    private fun changed(update: (String) -> Unit) = object : TextWatcher {
        override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
        override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}
        override fun afterTextChanged(s: Editable?) = update(s.toString())
    }

    private fun panel(): LinearLayout = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        background = surface(R.color.surface)
        setPadding(dp(12), dp(8), dp(12), dp(12))
        content.addView(this, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(12) })
    }

    private fun surface(color: Int) = GradientDrawable().apply {
        setColor(getColor(color))
        cornerRadius = dp(8).toFloat()
        setStroke(dp(1), getColor(R.color.border))
    }

    private fun label(parent: LinearLayout, value: String, size: Int, bold: Boolean = false,
                      muted: Boolean = false): TextView = TextView(this).apply {
        text = value
        textSize = size.toFloat()
        setTextColor(getColor(if (muted) R.color.muted else R.color.text))
        setPadding(0, dp(4), 0, dp(4))
        if (bold) typeface = Typeface.DEFAULT_BOLD
        parent.addView(this)
    }

    private fun message(parent: LinearLayout): TextView = label(parent, "", 14).apply {
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        visibility = View.GONE
    }

    private fun field(parent: LinearLayout, description: String, action: Int, submit: () -> Unit): EditText =
        EditText(this).apply {
            textSize = 16f
            contentDescription = description
            inputType = InputType.TYPE_CLASS_TEXT
            imeOptions = action
            filters = arrayOf(InputFilter.LengthFilter(MAX_NAME))
            minHeight = dp(48)
            setTextColor(getColor(R.color.text))
            setHintTextColor(getColor(R.color.muted))
            background = surface(R.color.background)
            setPadding(dp(12), dp(8), dp(12), dp(8))
            setOnEditorActionListener { _, id, _ -> (id == action).also { if (it) submit() } }
            parent.addView(this, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(4) })
        }

    private fun button(parent: LinearLayout, value: String, primary: Boolean = false, icon: Int = 0,
                       weight: Boolean = false, action: () -> Unit): Button = Button(this).apply {
        text = value
        textSize = 16f
        isAllCaps = false
        setSingleLine(false)
        minHeight = dp(48)
        gravity = Gravity.CENTER
        stateListAnimator = null
        val fill = if (!primary) surface(R.color.surface) else StateListDrawable().apply {
            addState(intArrayOf(-android.R.attr.state_enabled), surface(R.color.surface))
            addState(intArrayOf(), surface(R.color.accent).apply { setStroke(0, 0) })
        }
        background = RippleDrawable(ColorStateList.valueOf(getColor(R.color.border)), fill, null)
        setTextColor(ColorStateList(arrayOf(intArrayOf(-android.R.attr.state_enabled), intArrayOf()),
            intArrayOf(getColor(R.color.muted), getColor(if (primary) R.color.on_accent else R.color.text))))
        if (icon != 0) {
            setCompoundDrawablesRelativeWithIntrinsicBounds(icon, 0, 0, 0)
            compoundDrawablePadding = dp(8)
        }
        setPadding(dp(12), dp(12), dp(12), dp(12))
        setOnClickListener { action() }
        val params = if (weight) LinearLayout.LayoutParams(0, -2, 1f).apply { marginEnd = dp(8) }
            else LinearLayout.LayoutParams(-1, -2)
        parent.addView(this, params.apply { topMargin = dp(8) })
    }

    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()
}
