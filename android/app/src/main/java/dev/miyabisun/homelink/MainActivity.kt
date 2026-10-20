package dev.miyabisun.homelink

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.content.res.ColorStateList
import android.graphics.drawable.LayerDrawable
import android.os.Build
import android.os.Bundle
import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.view.Gravity
import android.view.View
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
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
        private val BLUETOOTH_PERMISSIONS = bluetoothPermissions(Build.VERSION.SDK_INT)
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
    private var addingRoom = false
    private var enteringCode = false
    private var formattingCode = false
    /** The room position last drawn; the select box reports it back without the user choosing. */
    private var shownRoom = 0
    /** The device, its room and the rooms last drawn; drawing again otherwise would undo a pick not yet reported. */
    private var drawnRoom: Triple<Long?, Long?, List<Room>>? = null
    /** The device to connect once Bluetooth is allowed and on. */
    private var pendingConnect: Long? = null

    /** The home-link URL the rooms were last read from; another after the settings reads them again. */
    private var loadedUrl: String? = null
    private var endpoints = Endpoints()

    private lateinit var content: LinearLayout
    private lateinit var endpointNotice: LinearLayout
    private lateinit var wifiPanel: LinearLayout
    private lateinit var lightsOnButton: Button
    private lateinit var lightsOffButton: Button
    private lateinit var lightsRow: ResultRow
    private lateinit var pinButton: Button
    private lateinit var qrState: TextView
    private lateinit var deviceState: TextView
    private lateinit var qrMessage: TextView
    private lateinit var manualButton: Button
    private lateinit var manualPanel: LinearLayout
    private lateinit var manualField: EditText
    private lateinit var manualMessage: TextView
    private lateinit var devicePanels: List<LinearLayout>
    private lateinit var roomSpinner: Spinner
    private lateinit var roomAdapter: ArrayAdapter<String>
    private lateinit var roomState: TextView
    private lateinit var roomMessage: TextView
    private lateinit var addRoomButton: Button
    private lateinit var newRoomPanel: LinearLayout
    private lateinit var newRoomName: EditText
    private lateinit var createRoomButton: Button
    private lateinit var nameField: EditText
    private lateinit var labelPanel: LinearLayout
    private lateinit var labelBoxes: LinearLayout
    private lateinit var unconnectedPanel: LinearLayout
    private lateinit var unconnectedList: LinearLayout
    private lateinit var resultRow: ResultRow
    private lateinit var reconnectButton: Button
    private lateinit var connectButton: Button
    private lateinit var moveButton: Button
    private lateinit var cancelButton: Button

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        api = homeLinkApi(this)
        endpoints = Endpoints.load(this)
        loadedUrl = endpoints.homeLinkUrl
        scanner = scannerFactory?.invoke(this) ?: GmsQrScanner(this)
        wifiStore = wifiStoreFactory?.invoke(this) ?: KeystoreWifiStore(this)
        worker = Executors.newSingleThreadExecutor()
        // Networks earlier versions kept on this phone move to home-link before anything reads them.
        background({ moveToServer(wifiStore, api::saveWifi) }) { }
        // Switching lights can wait seconds for every bulb; registration does not queue behind it.
        lightsWorker = Executors.newSingleThreadExecutor()
        savedInstanceState?.let { state ->
            addingRoom = state.getBoolean("addingRoom")
            enteringCode = state.getBoolean("enteringCode")
            lightsResult = state.getString("lightsText")?.let { LightsMessage(it, state.getBoolean("lightsFailed")) }
        }
        build()
        manualField.setText(savedInstanceState?.getString("manualCode").orEmpty())
        manualField.setSelection(manualField.length())
        newRoomName.setText(savedInstanceState?.getString("newRoom").orEmpty())
        // A registration started before the screen was recreated carries on.
        if (CommissionSession.running) {
            form.connect(savedInstanceState?.getLong("connecting") ?: -1)
            form.stage = CommissionSession.stage
        }
        CommissionSession.follow { sessionChanged() }
        sessionChanged()
        WifiMove.follow { moveChanged() }
        moveChanged()
        savedInstanceState?.getLong("device", -1)?.takeIf { it >= 0 }?.let { id ->
            background({ api.device(id) }) { result ->
                if (result is ApiResult.Ok && form.device == null) open(result.value, savedInstanceState.getString("name"))
                render()
            }
        }
        loadRooms()
        loadLabels()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        form.device?.let {
            outState.putLong("device", it.id)
            outState.putString("name", nameField.text.toString())
        }
        form.connecting?.let { outState.putLong("connecting", it) }
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
        // The settings screen or `adb push` may have changed the endpoints meanwhile.
        endpoints = Endpoints.load(this)
        if (endpoints.homeLinkUrl != loadedUrl) {
            loadedUrl = endpoints.homeLinkUrl
            loadRooms()
            loadLabels()
        }
        // The settings screen may have saved the Wi-Fi for devices meanwhile.
        loadWifi()
        // Opening the app lists the devices still to connect.
        loadDevices()
        render()
    }

    override fun onPause() {
        saveName()
        super.onPause()
    }

    override fun onDestroy() {
        CommissionSession.follow(null)
        WifiMove.follow(null)
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
        background({ api.rooms() }) { result ->
            form.rooms(result)
            render()
        }
    }

    private fun loadLabels() {
        background({ api.labels() }) { result ->
            if (result is ApiResult.Ok) form.labels = result.value
            render()
        }
    }

    /** Reads the ledger, then the codes of the devices still to connect, to tell them apart. */
    private fun loadDevices() {
        background({ api.devices() }) { result ->
            form.devicesRead(result)
            if (result is ApiResult.Ok) for (device in form.unconnected.filter { it.code == null }) {
                background({ api.device(device.id) }) { detail ->
                    form.detailRead(detail)
                    render()
                }
            }
            render()
        }
    }

    private fun loadWifi() {
        background({ api.wifiNetworks() }) { result ->
            form.wifiRead(result)
            render()
        }
    }

    private fun switchLights(on: Boolean) {
        switching = on
        lightsResult = null
        render()
        val host = homeLinkHost(this)
        // All off releases every boost, so the widget's buttons read the boosts again.
        background(lightsWorker, { api.switchLights(on).also { LightsWidget.saveBoosts(this, api.boostedLabels()) } }) { result ->
            switching = null
            lightsResult = lightsMessage(on, result, host, detail = true)
            render()
            LightsWidget.show(this, lightsMessage(on, result, host, detail = false))
        }
    }

    private fun pinWidget() {
        AppWidgetManager.getInstance(this).requestPinAppWidget(ComponentName(this, LightsWidget::class.java), null, null)
    }

    private fun scan() {
        scanner.scan { result ->
            if (isDestroyed) return@scan
            when (result) {
                is ScanResult.Read -> form.scanned(result.value)?.let { code ->
                    closeManual()
                    add(code)
                }
                ScanResult.Cancelled -> Unit
                ScanResult.Failed -> form.status = Status.ScanFailed
            }
            render()
        }
    }

    /** Sends a read code to home-link at once and opens the device it answers with. */
    private fun add(code: String) {
        saveName()
        background({ api.addDevice(code) }) { result ->
            val before = form.device
            form.added(code, result)
            form.device?.takeIf { it != before }?.let { open(it) }
            render()
        }
    }

    /** Opens the device's pane; [name] is a name typed before the screen was recreated. */
    private fun open(device: Device, name: String? = null) {
        form.opened(device)
        nameField.setText(name ?: device.name)
        addingRoom = false
    }

    /** Sends the typed name once it differs from the device's. */
    private fun saveName() {
        val device = form.device ?: return
        val name = nameField.text.toString().trim()
        if (name == device.name) return
        update(device.id, DeviceUpdate.Name(name))
    }

    private fun update(id: Long, change: DeviceUpdate) {
        background({ api.updateDevice(id, change) }) { result ->
            form.updated(result)
            if (result == ApiResult.Failed(ApiError.ROOM_NOT_FOUND)) loadRooms()
            if (result == ApiResult.Failed(ApiError.LABEL_NOT_FOUND)) loadLabels()
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
        form.typed(shown)?.let { code ->
            closeManual()
            add(code)
        }
        render()
    }

    private fun submitManual() {
        form.submitted(manualField.text.toString())?.let { code ->
            closeManual()
            add(code)
        }
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
                form.device?.let { update(it.id, DeviceUpdate.Room(result.value.id)) }
            }
            render()
        }
    }

    /** Checks the Bluetooth permissions and power, asking for them, then connects device `id`. */
    private fun commission(id: Long) {
        pendingConnect = id
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
        endpoints = Endpoints.load(this)
        val proxy = endpoints.bleProxyUrl ?: run {
            form.bleFailed(BleProblem.PROXY_NOT_CONFIGURED)
            return render()
        }
        saveName()
        val link = linkFactory?.invoke(this) ?: ProxyLink(applicationContext, proxy)
        form.connect(id)
        CommissionSession.start(link) { api.commission(id) }
        form.stage = CommissionSession.stage
        render()
    }

    /** A device picked from the list opens and starts connecting. */
    private fun connectListed(device: Device) {
        if (form.device?.id != device.id) {
            saveName()
            open(device)
        }
        commission(device.id)
    }

    private fun cancel() {
        CommissionSession.cancel()
        form.cancelled()
        render()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != REQUEST_BLUETOOTH) return
        val id = pendingConnect
        if (grantResults.isNotEmpty() && grantResults.all { it == PackageManager.PERMISSION_GRANTED } && id != null) commission(id)
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
        val id = pendingConnect
        if (resultCode == RESULT_OK && id != null) commission(id)
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
                loadDevices()
            }
            is Outcome.Failed -> form.bleFailed(outcome.problem)
            null -> Unit
        }
        render()
    }

    private fun moveWifi(id: Long) {
        saveName()
        form.status = null
        WifiMove.start(id) { api.moveWifi(id) }
        moveChanged()
    }

    private fun moveChanged() {
        form.moving = WifiMove.device
        WifiMove.take()?.let { (id, result) ->
            if (form.device?.id == id) form.wifiMoved(result)
            // A failed device may answer from the old Wi-Fi or not at all: its state shows which.
            // A moved one is as home-link answered; matterjs-server may not have read its new network yet.
            if (result is ApiResult.Failed) loadDevices()
        }
        render()
    }

    private fun build() {
        content = page()

        // Until home-link's URL is set, nothing can be sent: the way to set it comes first.
        endpointNotice = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        content.addView(endpointNotice, LinearLayout.LayoutParams(-1, -2).apply { bottomMargin = dp(16) })
        ResultRow(this, endpointNotice, 0).show("接続先が未設定です。home-linkのURLを設定してください", failed = true)
        button(endpointNotice, "接続先を設定", icon = R.drawable.ic_settings) { openSettings() }

        label(content, "照明", 28, bold = true)
        val lights = panel()
        val lightActions = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        lights.addView(lightActions, LinearLayout.LayoutParams(-1, -2))
        lightsOnButton = button(lightActions, "照明オン", icon = R.drawable.ic_light_on, weight = true) { switchLights(true) }
        lightsOffButton = button(lightActions, "照明オフ", icon = R.drawable.ic_light_off, weight = true) { switchLights(false) }
        lightsRow = ResultRow(this, lights, dp(8))
        pinButton = button(lights, "ホーム画面にボタンを置く", icon = R.drawable.ic_widget) { pinWidget() }

        label(content, "機器を登録", 28, bold = true).apply {
            (layoutParams as LinearLayout.LayoutParams).topMargin = dp(24)
        }

        val qr = panel()
        label(qr, "機器のコード", 18, bold = true)
        qrState = label(qr, "", 16)
        deviceState = label(qr, "", 16).apply { compoundDrawablePadding = dp(8) }
        qrMessage = message(qr)
        button(qr, "QRを読み取る", icon = R.drawable.ic_qr) { scan() }
        manualButton = button(qr, "数字で入力", icon = R.drawable.ic_keypad) { openManual() }
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

        val name = panel()
        label(name, "機器名（任意）", 18, bold = true)
        nameField = field(name, "機器名（任意）", EditorInfo.IME_ACTION_DONE) { saveName() }
        nameField.setOnFocusChangeListener { _, focused -> if (!focused) saveName() }

        val room = panel()
        label(room, "部屋", 18, bold = true)
        roomState = label(room, "部屋を読み込めませんでした", 16)
        roomAdapter = ArrayAdapter<String>(this, R.layout.select_item)
        roomSpinner = select(room, roomAdapter, "部屋") { position ->
            val device = form.device
            if (device == null || position == shownRoom) return@select
            shownRoom = position
            update(device.id, DeviceUpdate.Room(form.rooms.getOrNull(position - 1)?.id))
        }
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

        labelPanel = panel()
        label(labelPanel, "ラベル", 18, bold = true)
        labelBoxes = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        labelPanel.addView(labelBoxes, LinearLayout.LayoutParams(-1, -2))
        devicePanels = listOf(name, room, labelPanel)

        wifiPanel = panel()
        label(wifiPanel, "機器に渡すWi-Fi", 18, bold = true)
        label(wifiPanel, "まだ保存していません", 16, muted = true)
        button(wifiPanel, "Wi-Fiを設定", icon = R.drawable.ic_settings) { openSettings() }

        unconnectedPanel = panel()
        label(unconnectedPanel, "未接続", 18, bold = true)
        unconnectedList = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        unconnectedPanel.addView(unconnectedList, LinearLayout.LayoutParams(-1, -2))

        resultRow = ResultRow(this, content, dp(16))
        reconnectButton = button(content, "再接続") {
            loadRooms()
            loadLabels()
            loadDevices()
            loadWifi()
        }
        connectButton = button(content, "接続", primary = true, icon = R.drawable.ic_bluetooth) {
            form.device?.let { commission(it.id) }
        }
        cancelButton = button(content, "キャンセル") { cancel() }
        moveButton = button(content, "", primary = true, icon = R.drawable.ic_wifi) {
            form.device?.let { moveWifi(it.id) }
        }
        button(content, "接続先の設定", icon = R.drawable.ic_settings) { openSettings() }.apply {
            (layoutParams as LinearLayout.LayoutParams).topMargin = dp(24)
        }
        render()
    }

    private fun openSettings() = startActivity(Intent(this, SettingsActivity::class.java))

    /** How a device reads in the list: its name, else its printed code. */
    private fun title(device: Device) = device.name.ifEmpty { device.code?.let { ManualCode.label(it) ?: it } ?: "機器" }

    private fun render() {
        endpointNotice.visibility = if (endpoints.homeLinkUrl == null) View.VISIBLE else View.GONE
        val busy = switching != null
        lightsOnButton.isEnabled = !busy
        lightsOffButton.isEnabled = !busy
        when (switching) {
            true -> lightsRow.show("照明をオンにしています…", failed = false, R.drawable.ic_light_on)
            false -> lightsRow.show("照明をオフにしています…", failed = false, R.drawable.ic_light_off)
            null -> lightsRow.show(lightsResult?.text, lightsResult?.failed == true)
        }
        pinButton.visibility = if (canPinWidget) View.VISIBLE else View.GONE

        val device = form.device
        val code = device?.code
        qrState.text = when {
            device == null -> "まだ読み取っていません"
            code == null -> ""
            code.startsWith("MT:") -> ManualCode.label(code)?.let { "読み取り済み：$it" }
                ?: "読み取り済み（ラベルの数字に直せないコード）：$code"
            else -> "入力済み：${ManualCode.label(code)}"
        }
        qrState.setTextColor(getColor(if (device == null) R.color.muted else R.color.text))
        qrState.visibility = if (qrState.text.isEmpty()) View.GONE else View.VISIBLE
        val state = device?.state
        deviceState.text = when (state) {
            DeviceState.UNCONNECTED -> "未接続"
            DeviceState.CONNECTED -> "接続済み"
            DeviceState.NO_RESPONSE -> "応答なし"
            null -> ""
        }
        deviceState.visibility = if (state == null) View.GONE else View.VISIBLE
        deviceState.setTextColor(getColor(when (state) {
            DeviceState.NO_RESPONSE -> R.color.danger
            DeviceState.UNCONNECTED -> R.color.muted
            else -> R.color.text
        }))
        deviceState.setCompoundDrawablesRelativeWithIntrinsicBounds(when (state) {
            DeviceState.CONNECTED -> R.drawable.ic_success
            DeviceState.NO_RESPONSE -> R.drawable.ic_error
            else -> 0
        }, 0, 0, 0)
        manualButton.visibility = if (enteringCode) View.GONE else View.VISIBLE
        manualPanel.visibility = if (enteringCode) View.VISIBLE else View.GONE
        val status = form.status
        show(qrMessage, when (status) {
            Status.NotMatter -> "MatterのQRコードではありません。QRコードが無い機器は「数字で入力」を使ってください"
            Status.ScanFailed -> "QRコードを読み取れませんでした。もう一度お試しください"
            Status.Failed(ApiError.INVALID_QR) -> "MatterのQRコードとして読み取れませんでした。読み取り直してください"
            Status.Failed(ApiError.INVALID_MANUAL) -> "この数字はMatterの機器のコードとして使えません。数字を入力し直してください"
            else -> null
        })
        val typed = ManualCode.digits(manualField.text.toString()).length
        show(manualMessage, when (form.manualError) {
            ManualError.LENGTH -> "11桁の数字を入力してください（あと${ManualCode.LENGTH - typed}桁）"
            ManualError.CHECK_DIGIT -> "数字が正しくありません。機器に印字された数字と見比べてください"
            null -> null
        })

        for (panel in devicePanels) panel.visibility = if (device == null) View.GONE else View.VISIBLE
        fill(roomAdapter, listOf("未設定") + form.rooms.map { it.name })
        val roomKey = Triple(device?.id, device?.roomId, form.rooms)
        if (roomKey != drawnRoom) {
            drawnRoom = roomKey
            shownRoom = form.rooms.indexOfFirst { it.id == device?.roomId } + 1
            roomSpinner.setSelection(shownRoom)
        }
        roomState.visibility = if (form.roomsFailed) View.VISIBLE else View.GONE
        show(roomMessage, when {
            status is Status.RoomAdded -> "部屋「${status.room}」を追加しました"
            status == Status.Failed(ApiError.DUPLICATE_ROOM) -> "同じ名前の部屋があります。一覧から選んでください"
            else -> null
        }, failure = status is Status.Failed)
        addRoomButton.visibility = if (addingRoom) View.GONE else View.VISIBLE
        newRoomPanel.visibility = if (addingRoom) View.VISIBLE else View.GONE
        createRoomButton.isEnabled = newRoomName.text.isNotBlank()
        if (device != null) renderLabels(device)

        wifiPanel.visibility = if (form.wifiMissing) View.VISIBLE else View.GONE
        renderUnconnected()

        val ssid = form.deviceWifi?.let { "「$it」" }.orEmpty()
        val result = when (status) {
            is Status.Commissioned -> if (status.name.isEmpty()) "接続しました" else "「${status.name}」を接続しました"
            is Status.WifiMoved -> "「${status.ssid}」へ移しました"
            is Status.BleFailed -> when (status.problem) {
                BleProblem.PERMISSION -> "Bluetoothで機器を探すには「付近のデバイス」の許可が必要です。" +
                    "許可してから「接続」を押し直してください"
                BleProblem.BLUETOOTH_OFF -> "Bluetoothがオフです。オンにしてから「接続」を押し直してください"
                BleProblem.PROXY_NOT_CONFIGURED -> "matterjs-serverのBLE Proxyの接続先が未設定です。「接続先の設定」で入れてください"
                BleProblem.PROXY_UNREACHABLE -> "matterjs-server（${authority(endpoints.bleProxyUrl)}）に" +
                    "接続できません。家のWi-Fiにつないでから、もう一度お試しください"
                BleProblem.PROXY_VERSION -> "matterjs-serverのBluetooth中継の版が合いません。アプリを更新してください"
            }
            is Status.Failed -> when (status.error) {
                ApiError.NOT_CONFIGURED -> NOT_CONFIGURED
                ApiError.UNREACHABLE, ApiError.TIMEOUT -> "home-link（${authority(endpoints.homeLinkUrl)}）に接続できません。$CHECK_ENDPOINT"
                ApiError.ROOM_NOT_FOUND -> "選んだ部屋が見つかりません。部屋を選び直してください"
                ApiError.INVALID_NAME -> "名前は${MAX_NAME}文字以内で入力してください"
                ApiError.INVALID_QR, ApiError.INVALID_MANUAL, ApiError.DUPLICATE_ROOM -> null
                ApiError.SERVER, ApiError.MATTER_UNREACHABLE, ApiError.MATTER_NOT_CONFIGURED,
                ApiError.LABEL_NOT_FOUND, ApiError.LABEL_HAS_NO_LIGHTS, ApiError.INVALID_WIFI, ApiError.WIFI_NOT_FOUND,
                ApiError.WIFI_NOT_SET, ApiError.NOT_ON_WIFI, ApiError.SESAME_NOT_CONFIGURED, ApiError.SESAME_REJECTED, ApiError.SESAME_UNREACHABLE ->
                    "home-linkでエラーが発生しました。時間をおいてお試しください"
                ApiError.DEVICE_NOT_FOUND -> "機器が見つかりませんでした。機器をペアリング待ちにし、" +
                    "電話を近づけてから「接続」を押し直してください"
                ApiError.WRONG_CODE -> "機器がコードを受け付けませんでした。機器のコードを読み取り直すか、入力し直してください"
                ApiError.WIFI_FAILED -> "機器がWi-Fi${ssid}につながりませんでした。Wi-Fiの名前とパスワードを確かめてください。" +
                    "2.4GHzにしか対応しない機器もあります"
                ApiError.COMMISSION_TIMEOUT -> "時間内に接続が終わりませんでした。機器をペアリング待ちにし直して、もう一度お試しください"
                ApiError.COMMISSION_FAILED -> "接続が途中で止まりました。電話を機器の近くに置いて、「接続」を押し直してください。" +
                    "機器の接続待ちが終わっていたら、機器の電源を1回入れ直してから押し直してください"
                ApiError.BLUETOOTH_UNAVAILABLE -> "matterjs-serverでBluetoothの中継が有効になっていません。サーバーの設定を確かめてください"
                ApiError.THREAD_FAILED -> "機器がThread網につながりませんでした。Thread網（OTBR）が動いているか確かめてから、" +
                    "もう一度お試しください"
                ApiError.WIFI_REVERTED -> "${ssid}につながらず、元のWi-Fiに戻りました"
                ApiError.WIFI_REVERTING -> "${ssid}につながらず、応答がありません。元のWi-Fiへ戻ります"
            }
            else -> null
        }
        val stage = form.stage
        if (device != null && form.moving == device.id) resultRow.show("Wi-Fiを移しています…", failed = false, R.drawable.ic_wifi)
        else if (form.busy && stage != null) resultRow.show(stageText(stage), failed = false, R.drawable.ic_bluetooth)
        else resultRow.show(result, status is Status.Failed || status is Status.BleFailed)
        reconnectButton.visibility =
            if (form.roomsFailed || form.devicesFailed || status == Status.Failed(ApiError.UNREACHABLE)) View.VISIBLE
            else View.GONE
        connectButton.visibility = if (form.canConnect()) View.VISIBLE else View.GONE
        cancelButton.visibility = if (form.busy) View.VISIBLE else View.GONE
        moveButton.text = "「${form.deviceWifi}」へ移す"
        moveButton.visibility = if (form.canMoveWifi()) View.VISIBLE else View.GONE
    }

    /** One check box per label, ticked for the device's. */
    private fun renderLabels(device: Device) {
        labelPanel.visibility = if (form.labels.isEmpty()) View.GONE else View.VISIBLE
        if (labelBoxes.childCount != form.labels.size ||
            form.labels.withIndex().any { (i, label) -> (labelBoxes.getChildAt(i) as CheckBox).text != label.name }) {
            labelBoxes.removeAllViews()
            for (label in form.labels) labelBoxes.addView(CheckBox(this).apply {
                text = label.name
                textSize = 16f
                minHeight = dp(48)
                setTextColor(getColor(R.color.text))
                buttonTintList = ColorStateList.valueOf(getColor(R.color.accent))
                // Clicks, not checked changes, so drawing the device's labels sends nothing.
                setOnClickListener {
                    val open = form.device ?: return@setOnClickListener
                    update(open.id, DeviceUpdate.Labels(if (isChecked) open.labelIds + label.id else open.labelIds - label.id))
                }
            })
        }
        form.labels.forEachIndexed { i, label -> (labelBoxes.getChildAt(i) as CheckBox).isChecked = label.id in device.labelIds }
    }

    /** A button per device still to connect; pressing one connects it. */
    private fun renderUnconnected() {
        val listed = form.unconnected
        unconnectedPanel.visibility = if (listed.isEmpty()) View.GONE else View.VISIBLE
        val titles = listed.map(::title)
        if (unconnectedList.childCount != listed.size ||
            titles.withIndex().any { (i, title) -> (unconnectedList.getChildAt(i) as Button).text != title }) {
            unconnectedList.removeAllViews()
        }
        listed.forEachIndexed { i, device ->
            val row = unconnectedList.getChildAt(i) as Button? ?: button(unconnectedList, titles[i], icon = R.drawable.ic_bluetooth) { }
            row.setOnClickListener { connectListed(device) }
            row.isEnabled = !form.busy
        }
    }

    private fun stageText(stage: Stage) = when (stage) {
        Stage.PREPARING -> "matterjs-serverに接続しています…"
        Stage.SEARCHING -> "機器を探しています…"
        Stage.CONNECTING -> "機器に接続しています…"
        Stage.SENDING -> "機器にコードと設定を送っています…"
    }

    /** A select box: the input surface with an expand arrow, so it reads apart from buttons. */
    private fun select(parent: LinearLayout, items: ArrayAdapter<String>, description: String,
                       selected: (Int) -> Unit): Spinner = Spinner(this).apply {
        adapter = items
        contentDescription = description
        minimumHeight = dp(48)
        background = LayerDrawable(arrayOf(surface(R.color.background), getDrawable(R.drawable.ic_expand))).apply {
            setLayerGravity(1, Gravity.END or Gravity.CENTER_VERTICAL)
            setLayerInsetEnd(1, dp(12))
        }
        setPaddingRelative(0, 0, dp(40), 0)
        onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) = selected(position)
            override fun onNothingSelected(parent: AdapterView<*>?) {}
        }
        parent.addView(this, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(8) })
    }

    /** Puts [names] in the adapter only when they changed, so an open drop-down is not reset. */
    private fun fill(adapter: ArrayAdapter<String>, names: List<String>) {
        if (adapter.count != names.size || (0 until adapter.count).any { adapter.getItem(it) != names[it] }) {
            adapter.clear()
            adapter.addAll(names)
        }
    }

    private fun panel() = panel(content)

}
