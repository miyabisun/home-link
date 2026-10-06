package dev.miyabisun.homelink

import android.Manifest
import android.app.Activity
import android.appwidget.AppWidgetManager
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
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
import android.widget.RadioButton
import android.widget.RadioGroup
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
        /** A fake link also skips the Bluetooth permission and power checks. */
        internal var linkFactory: ((Activity) -> BleLink)? = null
        internal var wifiStoreFactory: ((Activity) -> WifiStore)? = null
        private const val MAX_NAME = 100
        private const val REQUEST_BLUETOOTH = 1
        private const val REQUEST_ENABLE = 2
        private val BLUETOOTH_PERMISSIONS =
            arrayOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT,
                Manifest.permission.ACCESS_LOCAL_NETWORK)
    }

    private val form = RegisterForm()
    private lateinit var api: HomeLinkApi
    private lateinit var scanner: QrScanner
    private lateinit var wifiStore: WifiStore
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
    private var editingWifi = false

    private lateinit var content: LinearLayout
    private lateinit var bluetoothChoice: RadioButton
    private lateinit var recordChoice: RadioButton
    private lateinit var bluetoothGuide: LinearLayout
    private lateinit var wifiPanel: LinearLayout
    private lateinit var wifiState: TextView
    private lateinit var wifiEditButton: Button
    private lateinit var wifiEditor: LinearLayout
    private lateinit var wifiSsid: EditText
    private lateinit var wifiPassword: EditText
    private lateinit var wifiMessage: TextView
    private lateinit var wifiCancelButton: Button
    private lateinit var wifiSaveButton: Button
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
        wifiStore = wifiStoreFactory?.invoke(this) ?: KeystoreWifiStore(this)
        form.wifi = wifiStore.load()
        worker = Executors.newSingleThreadExecutor()
        // Switching lights can wait seconds for every bulb; registration does not queue behind it.
        lightsWorker = Executors.newSingleThreadExecutor()
        savedInstanceState?.let { state ->
            form.payload = state.getString("payload")
            form.roomId = state.getLong("roomId", -1).takeIf { it >= 0 }
            form.name = state.getString("name").orEmpty()
            addingRoom = state.getBoolean("addingRoom")
            enteringCode = state.getBoolean("enteringCode")
            form.bluetooth = state.getBoolean("bluetooth", true)
            editingWifi = state.getBoolean("editingWifi")
            lightsResult = state.getString("lightsText")?.let { LightsMessage(it, state.getBoolean("lightsFailed")) }
        }
        build()
        manualField.setText(savedInstanceState?.getString("manualCode").orEmpty())
        manualField.setSelection(manualField.length())
        newRoomName.setText(savedInstanceState?.getString("newRoom").orEmpty())
        nameField.setText(form.name)
        wifiSsid.setText(savedInstanceState?.getString("wifiSsid") ?: form.wifi?.ssid.orEmpty())
        // A registration started before the screen was recreated carries on.
        form.busy = CommissionSession.running
        form.stage = CommissionSession.stage
        CommissionSession.follow { sessionChanged() }
        sessionChanged()
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
        outState.putBoolean("bluetooth", form.bluetooth)
        outState.putBoolean("editingWifi", editingWifi)
        // The password is never put in the saved state.
        outState.putString("wifiSsid", wifiSsid.text.toString())
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
        CommissionSession.follow(null)
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

    private fun saveWifi() {
        val network = WifiNetwork(wifiSsid.text.toString(), wifiPassword.text.toString())
        // SSIDs are up to 32 bytes; WPA passphrases up to 64 characters.
        if (network.ssid.toByteArray().size !in 1..32 || network.password.length !in 1..64) {
            show(wifiMessage, "Wi-Fiの名前（32バイトまで）とパスワード（64文字まで）を入力してください")
            return
        }
        wifiStore.save(network)
        form.wifi = network
        editingWifi = false
        wifiPassword.setText("")
        wifiMessage.visibility = View.GONE
        if (form.status == Status.Failed(ApiError.INVALID_WIFI) || form.status == Status.Failed(ApiError.WIFI_FAILED)) {
            form.status = null
        }
        getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(wifiPassword.windowToken, 0)
        render()
    }

    /** Checks the Bluetooth permissions and power, asking for them, then starts the registration. */
    private fun commission() {
        if (linkFactory == null) {
            if (BLUETOOTH_PERMISSIONS.any { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }) {
                requestPermissions(BLUETOOTH_PERMISSIONS, REQUEST_BLUETOOTH)
                return
            }
            val adapter = getSystemService(BluetoothManager::class.java)?.adapter
            if (adapter == null) {
                form.bleFailed(BleProblem.BLUETOOTH_OFF)
                render()
                return
            }
            if (!adapter.isEnabled) {
                @Suppress("DEPRECATION") // The result is needed to carry on; there is no other API for this prompt.
                startActivityForResult(Intent(BluetoothAdapter.ACTION_REQUEST_ENABLE), REQUEST_ENABLE)
                return
            }
        }
        val payload = form.payload ?: return
        val roomId = form.roomId ?: return
        val wifi = form.wifi ?: return
        val name = form.name.trim()
        form.busy = true
        form.status = null
        val link = linkFactory?.invoke(this) ?: ProxyLink(applicationContext, BuildConfig.BLE_PROXY_URL)
        CommissionSession.start(link) { api.commission(roomId, payload, name, wifi) }
        form.stage = CommissionSession.stage
        render()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != REQUEST_BLUETOOTH) return
        if (grantResults.isNotEmpty() && grantResults.all { it == PackageManager.PERMISSION_GRANTED }) commission()
        else {
            form.bleFailed(BleProblem.PERMISSION)
            render()
        }
    }

    @Deprecated("Pairs with startActivityForResult for the Bluetooth prompt")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQUEST_ENABLE) return
        if (resultCode == RESULT_OK) commission()
        else {
            form.bleFailed(BleProblem.BLUETOOTH_OFF)
            render()
        }
    }

    private fun sessionChanged() {
        form.stage = CommissionSession.stage
        when (val outcome = CommissionSession.take()) {
            is Outcome.Done -> {
                form.commissioned(outcome.result)
                if (outcome.result is ApiResult.Ok) nameField.setText("")
                if (outcome.result == ApiResult.Failed(ApiError.ROOM_NOT_FOUND)) loadRooms()
            }
            is Outcome.Failed -> form.bleFailed(outcome.problem)
            null -> Unit
        }
        render()
    }

    private fun register() {
        if (form.bluetooth) return commission()
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

        val mode = panel()
        label(mode, "つなぎ方", 18, bold = true)
        val choices = RadioGroup(this).apply { orientation = RadioGroup.VERTICAL }
        mode.addView(choices, LinearLayout.LayoutParams(-1, -2))
        bluetoothChoice = choice(choices, "新しいWi-Fi電球をBluetoothでつなぐ") { form.bluetooth = true }
        recordChoice = choice(choices, "記録だけ（つながっている機器）") { form.bluetooth = false }
        bluetoothGuide = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        mode.addView(bluetoothGuide, LinearLayout.LayoutParams(-1, -2))
        label(bluetoothGuide, "電球をペアリング待ちにし、電話を近づけて登録します。" +
            "BEAMTECは電源のオフ・オンを5回くり返します。ほかはメーカーの手順に従ってください。", 14, muted = true)
        label(bluetoothGuide, "ThreadやZigbeeの電球（Aqara T2など）はこの方法でつなげません。" +
            "Aqara Homeで追加してから「記録だけ」で登録してください。", 14, muted = true)

        wifiPanel = panel()
        label(wifiPanel, "電球に渡すWi-Fi", 18, bold = true)
        wifiState = label(wifiPanel, "", 16)
        wifiEditButton = button(wifiPanel, "変更") {
            editingWifi = true
            render()
            wifiPassword.requestFocus()
        }
        wifiEditor = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        wifiPanel.addView(wifiEditor, LinearLayout.LayoutParams(-1, -2))
        label(wifiEditor, "Wi-Fiの名前（SSID）", 14, muted = true)
        wifiSsid = field(wifiEditor, "Wi-Fiの名前（SSID）", EditorInfo.IME_ACTION_NEXT) { wifiPassword.requestFocus() }
        wifiSsid.addTextChangedListener(changed { render() })
        label(wifiEditor, "パスワード", 14, muted = true)
        wifiPassword = field(wifiEditor, "Wi-Fiのパスワード", EditorInfo.IME_ACTION_DONE) { saveWifi() }
        wifiPassword.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
        wifiPassword.addTextChangedListener(changed { render() })
        label(wifiEditor, "この電話の中だけに暗号化して保存し、登録のたびにmatterjs-serverへ渡します。", 14, muted = true)
        wifiMessage = message(wifiEditor)
        val wifiActions = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        wifiEditor.addView(wifiActions, LinearLayout.LayoutParams(-1, -2))
        wifiCancelButton = button(wifiActions, "やめる", weight = true) {
            editingWifi = false
            wifiSsid.setText(form.wifi?.ssid.orEmpty())
            wifiPassword.setText("")
            wifiMessage.visibility = View.GONE
            render()
        }
        wifiSaveButton = button(wifiActions, "保存", weight = true) { saveWifi() }

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

        bluetoothChoice.isChecked = form.bluetooth
        recordChoice.isChecked = !form.bluetooth
        bluetoothChoice.isEnabled = !form.busy
        recordChoice.isEnabled = !form.busy
        bluetoothGuide.visibility = if (form.bluetooth) View.VISIBLE else View.GONE
        wifiPanel.visibility = if (form.bluetooth) View.VISIBLE else View.GONE
        val wifi = form.wifi
        val editing = editingWifi || wifi == null
        wifiState.text = if (wifi == null) "まだ設定していません" else "「${wifi.ssid}」を渡します"
        wifiState.setTextColor(getColor(if (wifi == null) R.color.muted else R.color.text))
        wifiEditButton.visibility = if (editing) View.GONE else View.VISIBLE
        wifiEditButton.isEnabled = !form.busy
        wifiEditor.visibility = if (editing) View.VISIBLE else View.GONE
        wifiCancelButton.visibility = if (wifi == null) View.GONE else View.VISIBLE
        wifiSaveButton.isEnabled = wifiSsid.text.isNotEmpty() && wifiPassword.text.isNotEmpty()

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

        val ssid = form.wifi?.ssid.orEmpty()
        val result = when (status) {
            is Status.Commissioned -> with(status.device) {
                when {
                    !registered -> "Wi-Fiにつなぎました。台帳の" +
                        (if (name.isEmpty()) "${room}の機器" else "「$name」（$room）") + "として登録済みです"
                    name.isEmpty() -> "${room}に機器をつなぎ、登録しました"
                    else -> "${room}に「$name」をつなぎ、登録しました"
                }
            }
            is Status.BleFailed -> when (status.problem) {
                BleProblem.PERMISSION -> "Bluetoothで電球を探すには「付近のデバイス」の許可が必要です。" +
                    "許可してから「登録」を押し直してください"
                BleProblem.BLUETOOTH_OFF -> "Bluetoothがオフです。オンにしてから「登録」を押し直してください"
                BleProblem.PROXY_UNREACHABLE -> "matterjs-server（${Uri.parse(BuildConfig.BLE_PROXY_URL).authority}）に" +
                    "接続できません。家のWi-Fiにつないでから、もう一度お試しください"
                BleProblem.PROXY_VERSION -> "matterjs-serverのBluetooth中継の版が合いません。アプリを更新してください"
            }
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
                ApiError.DEVICE_NOT_FOUND -> "電球が見つかりませんでした。電球をペアリング待ちにし、" +
                    "電話を近づけてから「登録」を押し直してください"
                ApiError.WRONG_CODE -> "電球がコードを受け付けませんでした。電球のコードを読み取り直すか、入力し直してください"
                ApiError.WIFI_FAILED -> "電球がWi-Fi「$ssid」につながりませんでした。Wi-Fiの名前とパスワードを確かめてください。" +
                    "2.4GHzにしか対応しない電球もあります"
                ApiError.COMMISSION_TIMEOUT -> "時間内に登録が終わりませんでした。電球をペアリング待ちにし直して、もう一度お試しください"
                ApiError.COMMISSION_FAILED -> "電球を登録できませんでした。電球を初期化してから、もう一度お試しください"
                ApiError.BLUETOOTH_UNAVAILABLE -> "matterjs-serverでBluetoothの中継が有効になっていません。サーバーの設定を確かめてください"
                ApiError.INVALID_WIFI -> "Wi-Fiの名前かパスワードが長すぎます。「変更」から入力し直してください"
            }
            else -> null
        }
        val stage = form.stage
        if (form.busy && stage != null) resultRow.show(stageText(stage), failed = false, R.drawable.ic_bluetooth)
        else resultRow.show(result, status is Status.Failed || status is Status.BleFailed)
        reconnectButton.visibility =
            if ((form.roomsFailed || status == Status.Failed(ApiError.UNREACHABLE)) && !loadingRooms) View.VISIBLE
            else View.GONE
        registerButton.isEnabled = form.canRegister()
        registerButton.text = if (form.busy) "登録中…" else "登録"
    }

    private fun stageText(stage: Stage) = when (stage) {
        Stage.PREPARING -> "matterjs-serverに接続しています…"
        Stage.SEARCHING -> "電球を探しています…"
        Stage.CONNECTING -> "電球に接続しています…"
        Stage.SENDING -> "電球にコードとWi-Fiの設定を送っています…"
        Stage.JOINING -> "電球がWi-Fiにつながるのを待っています…"
    }

    private fun choice(group: RadioGroup, value: String, select: () -> Unit): RadioButton = RadioButton(this).apply {
        text = value
        textSize = 16f
        minHeight = dp(48)
        setTextColor(ColorStateList(arrayOf(intArrayOf(-android.R.attr.state_enabled), intArrayOf()),
            intArrayOf(getColor(R.color.muted), getColor(R.color.text))))
        buttonTintList = ColorStateList.valueOf(getColor(R.color.accent))
        setOnClickListener {
            select()
            form.status = null
            render()
        }
        group.addView(this, RadioGroup.LayoutParams(-1, -2))
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
