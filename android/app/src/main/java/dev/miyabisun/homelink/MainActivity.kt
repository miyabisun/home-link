package dev.miyabisun.homelink

import android.app.Activity
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.Intent
import android.content.pm.PackageManager
import android.content.res.ColorStateList
import android.os.Build
import android.os.Bundle
import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.view.Gravity
import android.view.View
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
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
    private lateinit var switchWorker: ExecutorService
    private var enteringCode = false
    private var formattingCode = false
    /** The device to connect once Bluetooth is allowed and on. */
    private var pendingConnect: Long? = null

    private var endpoints = Endpoints()

    private lateinit var content: LinearLayout
    private lateinit var noticePanel: LinearLayout
    private lateinit var noticeRow: ResultRow
    private lateinit var noticeButton: Button
    private lateinit var wifiPanel: LinearLayout
    private lateinit var qrPanel: LinearLayout
    private lateinit var qrMessage: TextView
    private lateinit var qrRow: ResultRow
    private lateinit var manualButton: Button
    private lateinit var manualPanel: LinearLayout
    private lateinit var manualField: EditText
    private lateinit var manualMessage: TextView
    private lateinit var devicePane: LinearLayout
    private lateinit var deviceViews: List<View>
    private lateinit var codeView: TextView
    private lateinit var deviceState: TextView
    private lateinit var faceSwitch: Button
    private lateinit var nameField: EditText
    private lateinit var labelPanel: LinearLayout
    private lateinit var labelBoxes: LinearLayout
    private lateinit var resultRow: ResultRow
    private lateinit var connectButton: Button
    private lateinit var moveButton: Button
    private lateinit var cancelButton: Button
    private lateinit var unconnectedPanel: LinearLayout
    private lateinit var unconnectedList: LinearLayout

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        api = homeLinkApi(this)
        endpoints = Endpoints.load(this)
        scanner = scannerFactory?.invoke(this) ?: GmsQrScanner(this)
        wifiStore = wifiStoreFactory?.invoke(this) ?: KeystoreWifiStore(this)
        worker = Executors.newSingleThreadExecutor()
        // Networks earlier versions kept on this phone move to home-link before anything reads them.
        background({ moveToServer(wifiStore, api::saveWifi) }) { }
        // Switching a label can wait seconds for every device; registration does not queue behind it.
        switchWorker = Executors.newSingleThreadExecutor()
        enteringCode = savedInstanceState?.getBoolean("enteringCode") == true
        build()
        manualField.setText(savedInstanceState?.getString("manualCode").orEmpty())
        manualField.setSelection(manualField.length())
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
    }

    override fun onSaveInstanceState(outState: Bundle) {
        form.device?.let {
            outState.putLong("device", it.id)
            outState.putString("name", nameField.text.toString())
        }
        form.connecting?.let { outState.putLong("connecting", it) }
        outState.putBoolean("enteringCode", enteringCode)
        outState.putString("manualCode", manualField.text.toString())
        super.onSaveInstanceState(outState)
    }

    override fun onResume() {
        super.onResume()
        // The settings screen or `adb push` may have changed the endpoints meanwhile.
        endpoints = Endpoints.load(this)
        // The admin screen may have changed the labels meanwhile.
        loadLabels()
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
        switchWorker.shutdownNow()
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

    /** A device picked from the list opens in the pane, which connects it and shows how that goes. */
    private fun connectListed(device: Device) {
        if (form.device?.id != device.id) {
            saveName()
            open(device)
        }
        commission(device.id)
        devicePane.post { (content.parent as ScrollView).smoothScrollTo(0, devicePane.top) }
    }

    private fun switchDevice(device: Device) {
        val on = form.nextOn(device)
        form.switchStarted(device.id)
        render()
        background(switchWorker, { api.switchDevice(device.id, on) }) { result ->
            form.switched(device.id, result)
            render()
        }
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
        content = headedPage(menu = listOf(MenuEntry("接続先の設定", R.drawable.ic_settings) { openSettings() }))

        // Until home-link's URL is set or its ledger read, nothing below works: the way out comes first.
        noticePanel = panel(content)
        noticeRow = ResultRow(this, noticePanel, dp(4))
        noticeButton = button(noticePanel, "") { if (endpoints.homeLinkUrl == null) openSettings() else reload() }

        wifiPanel = panel(content)
        label(wifiPanel, "機器に渡すWi-Fi", 18, bold = true)
        label(wifiPanel, "まだ保存していません", 16, muted = true)
        button(wifiPanel, "Wi-Fiを設定", icon = R.drawable.ic_settings) { openSettings() }

        qrPanel = panel(content)
        label(qrPanel, "機器を登録", 18, bold = true)
        qrMessage = message(qrPanel)
        // What sending a read code to home-link left behind, before any device is open.
        qrRow = ResultRow(this, qrPanel, dp(8))
        button(qrPanel, "QRを読み取る", icon = R.drawable.ic_qr) { scan() }
        manualButton = button(qrPanel, "数字で入力", icon = R.drawable.ic_keypad) { openManual() }
        manualPanel = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        qrPanel.addView(manualPanel, LinearLayout.LayoutParams(-1, -2))
        label(manualPanel, "11桁の数字", 14, muted = true)
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

        // The open device's pane holds everything that acts on it: its fields, how connecting goes, and the actions.
        devicePane = panel(content)
        codeView = label(devicePane, "", 18, bold = true)
        // A plug's switch sits beside its state, apart from the pane's main action.
        val stateRow = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            isBaselineAligned = false
        }
        devicePane.addView(stateRow, LinearLayout.LayoutParams(-1, -2))
        deviceState = label(stateRow, "", 16).apply {
            compoundDrawablePadding = dp(8)
            layoutParams = LinearLayout.LayoutParams(0, -2, 1f)
        }
        // Centred in the row, a top margin would push the switch past its bottom.
        faceSwitch = button(stateRow, "") { form.device?.let(::switchDevice) }.apply {
            layoutParams = LinearLayout.LayoutParams(-2, -2)
        }
        val nameCaption = label(devicePane, "機器名", 14, muted = true).apply {
            (layoutParams as LinearLayout.LayoutParams).topMargin = dp(8)
        }
        nameField = field(devicePane, "機器名", EditorInfo.IME_ACTION_DONE) { saveName() }
        nameField.setOnFocusChangeListener { _, focused -> if (!focused) saveName() }
        labelPanel = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        devicePane.addView(labelPanel, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(8) })
        label(labelPanel, "ラベル", 14, muted = true)
        labelBoxes = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        labelPanel.addView(labelBoxes, LinearLayout.LayoutParams(-1, -2))
        deviceViews = listOf(stateRow, nameCaption, nameField, labelPanel)
        resultRow = ResultRow(this, devicePane, dp(12))
        connectButton = button(devicePane, "接続", primary = true, icon = R.drawable.ic_bluetooth) {
            form.device?.let { commission(it.id) }
        }
        cancelButton = button(devicePane, "キャンセル") { cancel() }
        moveButton = button(devicePane, "", primary = true, icon = R.drawable.ic_wifi) {
            form.device?.let { moveWifi(it.id) }
        }

        unconnectedPanel = panel(content)
        label(unconnectedPanel, "未接続", 18, bold = true)
        unconnectedList = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        unconnectedPanel.addView(unconnectedList, LinearLayout.LayoutParams(-1, -2))
        render()
    }

    private fun reload() {
        loadLabels()
        loadDevices()
        loadWifi()
    }

    private fun openSettings() = startActivity(Intent(this, SettingsActivity::class.java))

    private fun render() {
        val unset = endpoints.homeLinkUrl == null
        val ledgerError = form.devicesError
        noticePanel.visibility = if (unset || ledgerError != null) View.VISIBLE else View.GONE
        noticeRow.show(when {
            unset -> "接続先が未設定です。home-linkのURLを設定してください"
            ledgerError != null -> resultText(Status.Failed(ledgerError))
            else -> null
        }, failed = true)
        noticeButton.text = if (unset) "接続先を設定" else "再接続"
        noticeButton.setCompoundDrawablesRelativeWithIntrinsicBounds(if (unset) R.drawable.ic_settings else 0, 0, 0, 0)

        val device = form.device
        val status = form.status
        val result = resultText(status)
        val failed = status is Status.Failed || status is Status.BleFailed

        // Reading another code while one connects would leave that connection without its pane.
        qrPanel.visibility = if (form.busy) View.GONE else View.VISIBLE
        manualButton.visibility = if (enteringCode) View.GONE else View.VISIBLE
        manualPanel.visibility = if (enteringCode) View.VISIBLE else View.GONE
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
        qrRow.show(if (device == null) result else null, failed)

        devicePane.visibility = if (device != null || form.busy) View.VISIBLE else View.GONE
        for (view in deviceViews) view.visibility = if (device == null) View.GONE else View.VISIBLE
        codeView.text = device?.code?.let { ManualCode.label(it) ?: it }.orEmpty()
        codeView.visibility = if (codeView.text.isEmpty()) View.GONE else View.VISIBLE
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
        val plug = device?.takeIf { it.kind == DeviceKind.PLUG && it.state != DeviceState.UNCONNECTED }
        faceSwitch.visibility = if (plug == null) View.GONE else View.VISIBLE
        plug?.let { renderSwitch(faceSwitch, it) }
        if (device != null) renderLabels(device)

        val stage = form.stage
        if (device != null && form.moving == device.id) resultRow.show("Wi-Fiを移しています…", failed = false, R.drawable.ic_wifi)
        else if (form.busy && stage != null) resultRow.show(stageText(stage), failed = false, R.drawable.ic_bluetooth)
        else resultRow.show(if (device != null) result else null, failed)
        connectButton.visibility = if (form.canConnect()) View.VISIBLE else View.GONE
        cancelButton.visibility = if (form.busy) View.VISIBLE else View.GONE
        moveButton.text = "「${form.deviceWifi}」へ移す"
        moveButton.visibility = if (form.canMoveWifi()) View.VISIBLE else View.GONE

        wifiPanel.visibility = if (form.wifiMissing) View.VISIBLE else View.GONE
        renderUnconnected()
    }

    /** What the last action left to say, in the pane or the panel it started from; none for a code's own errors. */
    private fun resultText(status: Status?): String? {
        val ssid = form.deviceWifi?.let { "「$it」" }.orEmpty()
        return when (status) {
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
                ApiError.INVALID_NAME -> "名前は${MAX_NAME}文字以内で入力してください"
                ApiError.INVALID_QR, ApiError.INVALID_MANUAL -> null
                ApiError.SERVER, ApiError.MATTER_UNREACHABLE, ApiError.MATTER_NOT_CONFIGURED,
                ApiError.LABEL_NOT_FOUND, ApiError.LABEL_HAS_NO_DEVICES, ApiError.DUPLICATE_LABEL, ApiError.INVALID_LABEL, ApiError.INVALID_WIFI, ApiError.WIFI_NOT_FOUND,
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
                ApiError.SWITCH_FAILED -> "機器が入り切りを受け付けませんでした"
            }
            else -> null
        }
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

    /** A button per device still to connect; pressing one opens it in the pane and connects it. */
    private fun renderUnconnected() {
        val listed = form.pickable
        unconnectedPanel.visibility = if (listed.isEmpty()) View.GONE else View.VISIBLE
        val titles = listed.map { it.title }
        if (unconnectedList.childCount != listed.size ||
            titles.withIndex().any { (i, title) -> (unconnectedList.getChildAt(i) as Button).text != title }) {
            unconnectedList.removeAllViews()
        }
        listed.forEachIndexed { i, device ->
            val row = unconnectedList.getChildAt(i) as Button? ?: button(unconnectedList, titles[i], icon = R.drawable.ic_bluetooth) { }
            row.setOnClickListener { connectListed(device) }
        }
    }

    /** A plug's switch: on in the accent surface, off in the usual one, unanswered with the failure icon. */
    private fun renderSwitch(button: Button, device: Device) {
        val on = device.state == DeviceState.CONNECTED && device.on == true
        val unanswered = device.state == DeviceState.NO_RESPONSE
        button.isEnabled = device.id !in form.switching
        paintSwitch(button, when {
            unanswered -> "応答なし"
            on -> "入"
            else -> "切"
        }, on, unanswered)
        button.contentDescription = "${device.title} ${button.text}"
        button.setOnClickListener { switchDevice(device) }
    }

    private fun stageText(stage: Stage) = when (stage) {
        Stage.PREPARING -> "matterjs-serverに接続しています…"
        Stage.SEARCHING -> "機器を探しています…"
        Stage.CONNECTING -> "機器に接続しています…"
        Stage.SENDING -> "機器にコードと設定を送っています…"
    }

}
