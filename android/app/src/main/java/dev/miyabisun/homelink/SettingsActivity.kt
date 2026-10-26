package dev.miyabisun.homelink

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.text.InputFilter
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.Spinner
import android.widget.TextView
import android.graphics.drawable.LayerDrawable
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

/**
 * Edits where the app sends its requests, in the same YAML file that `adb push` may write,
 * and the Wi-Fi home-link hands new devices.
 */
class SettingsActivity : Activity() {
    companion object {
        /** A fake reader also skips the location permission. */
        internal var wifiReaderFactory: ((Activity) -> WifiReader)? = null
        private const val MAX_URL = 2048
        private const val HOME_LINK_EXAMPLE = "http://home-link.example:5011"
        private const val FALLBACK_EXAMPLE = "http://192.168.1.10:5011"
        private const val BLE_PROXY_EXAMPLE = "ws://192.0.2.10:5580/ble"
        private const val REQUEST_LOCATION = 1
        // Android shows the SSID of the connected network only to apps with fine location.
        private val LOCATION_PERMISSIONS = arrayOf(Manifest.permission.ACCESS_FINE_LOCATION,
            Manifest.permission.ACCESS_COARSE_LOCATION)
    }

    private lateinit var homeLinkField: EditText
    private lateinit var homeLinkMessage: TextView
    private lateinit var fallbackField: EditText
    private lateinit var fallbackMessage: TextView
    private lateinit var bleProxyField: EditText
    private lateinit var bleProxyMessage: TextView
    private lateinit var resultRow: ResultRow

    private lateinit var api: HomeLinkApi
    private lateinit var wifiReader: WifiReader
    private lateinit var worker: ExecutorService
    private var networks: List<SavedNetwork> = emptyList()
    /** null while reading home-link's networks, false when the read failed. */
    private var networksRead: Boolean? = null
    private var wifiBusy = false
    private var addingWifi = false
    /** The network the phone was on when the adder opened; null until read or when not on Wi-Fi. */
    private var currentWifi: CurrentWifi? = null
    private var currentRead = false
    private var locationDenied = false
    private var wifiNotice: LightsMessage? = null
    /** Why home-link's networks could not be read. */
    private var wifiError: ApiError? = null
    private lateinit var wifiState: TextView
    private lateinit var wifiSpinner: Spinner
    private lateinit var wifiAdapter: ArrayAdapter<String>
    private lateinit var wifiNoticeView: TextView
    private lateinit var wifiReload: Button
    private lateinit var wifiActions: LinearLayout
    private lateinit var wifiAddButton: Button
    private lateinit var wifiDeleteButton: Button
    private lateinit var wifiEditor: LinearLayout
    private lateinit var wifiCurrent: TextView
    private lateinit var wifiBand: TextView
    private lateinit var wifiSsid: EditText
    private lateinit var wifiPassword: EditText
    private lateinit var wifiMessage: TextView
    private lateinit var wifiCancelButton: Button
    private lateinit var wifiSaveButton: Button

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        api = homeLinkApi(this)
        wifiReader = wifiReaderFactory?.invoke(this) ?: AndroidWifiReader(this)
        worker = Executors.newSingleThreadExecutor()
        addingWifi = savedInstanceState?.getBoolean("addingWifi") == true
        locationDenied = savedInstanceState?.getBoolean("locationDenied") == true
        val content = headedPage(home = ::finish, subtitle = "接続先の設定")

        // Both URLs are saved together, by the button in their panel.
        val urls = panel(content)
        label(urls, "home-link", 14, muted = true)
        homeLinkField = urlField(urls, "home-linkのURL", HOME_LINK_EXAMPLE, EditorInfo.IME_ACTION_NEXT) { fallbackField.requestFocus() }
        homeLinkMessage = message(urls)
        label(urls, "予備のURL（任意・1行に1つ）", 14, muted = true).apply {
            (layoutParams as LinearLayout.LayoutParams).topMargin = dp(8)
        }
        label(urls, "home-linkのURLに1秒で接続できないとき、こちらにも接続し、先につながった方へ1回だけ送ります。家のLANのアドレスなど", 14, muted = true)
        // Enter starts the next URL, so the keyboard offers no action.
        fallbackField = urlField(urls, "予備のURL", FALLBACK_EXAMPLE, EditorInfo.IME_ACTION_NONE) { }.apply {
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI or InputType.TYPE_TEXT_FLAG_MULTI_LINE
            filters = arrayOf(InputFilter.LengthFilter(MAX_URL * 4))
        }
        fallbackMessage = message(urls)
        label(urls, "matterjs-serverのBLE Proxy", 14, muted = true).apply {
            (layoutParams as LinearLayout.LayoutParams).topMargin = dp(8)
        }
        bleProxyField = urlField(urls, "BLE ProxyのURL", BLE_PROXY_EXAMPLE, EditorInfo.IME_ACTION_DONE) { save() }
        bleProxyMessage = message(urls)
        resultRow = ResultRow(this, urls, dp(12))
        button(urls, "保存", primary = true) { save() }
        buildWifi(content)

        val saved = Endpoints.load(this)
        homeLinkField.setText(savedInstanceState?.getString("homeLink") ?: saved.homeLink)
        fallbackField.setText(savedInstanceState?.getString("fallbacks") ?: saved.fallbacks.joinToString("\n"))
        bleProxyField.setText(savedInstanceState?.getString("bleProxy") ?: saved.bleProxy)
        // A malformed value, as `adb push` may leave, is shown with what is wrong before any save.
        check()
        resultRow.show(null, failed = false)
        wifiSsid.setText(savedInstanceState?.getString("wifiSsid").orEmpty())
        if (addingWifi) readCurrentWifi(ask = false)
        loadWifi()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putString("homeLink", homeLinkField.text.toString())
        outState.putString("fallbacks", fallbackField.text.toString())
        outState.putString("bleProxy", bleProxyField.text.toString())
        outState.putBoolean("addingWifi", addingWifi)
        outState.putBoolean("locationDenied", locationDenied)
        // The password is never put in the saved state.
        outState.putString("wifiSsid", wifiSsid.text.toString())
        super.onSaveInstanceState(outState)
    }

    override fun onDestroy() {
        worker.shutdownNow()
        super.onDestroy()
    }

    /** Runs a blocking API call off the main thread and drops results for a destroyed screen. */
    private fun <T> background(call: () -> T, done: (T) -> Unit) {
        worker.execute {
            val result = call()
            runOnUiThread { if (!isDestroyed) done(result) }
        }
    }

    private fun buildWifi(content: LinearLayout) {
        val panel = panel(content).apply { (layoutParams as LinearLayout.LayoutParams).topMargin = dp(24) }
        label(panel, "機器に渡すWi-Fi", 18, bold = true)
        wifiState = label(panel, "", 16, muted = true)
        wifiAdapter = ArrayAdapter<String>(this, R.layout.select_item)
        wifiSpinner = Spinner(this).apply {
            adapter = wifiAdapter
            contentDescription = "機器に渡すWi-Fi"
            minimumHeight = dp(48)
            background = LayerDrawable(arrayOf(surface(R.color.background), getDrawable(R.drawable.ic_expand))).apply {
                setLayerGravity(1, Gravity.END or Gravity.CENTER_VERTICAL)
                setLayerInsetEnd(1, dp(12))
            }
            setPaddingRelative(0, 0, dp(40), 0)
            onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
                override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) =
                    chooseWifi(position)
                override fun onNothingSelected(parent: AdapterView<*>?) {}
            }
            panel.addView(this, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(8) })
        }
        wifiNoticeView = message(panel)
        wifiReload = button(panel, "再接続") { loadWifi() }
        wifiActions = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        panel.addView(wifiActions, LinearLayout.LayoutParams(-1, -2))
        wifiAddButton = button(wifiActions, "Wi-Fiを追加", icon = R.drawable.ic_add, weight = true) { openWifiAdder() }
        wifiDeleteButton = button(wifiActions, "このWi-Fiを削除", weight = true) { deleteWifi() }
        wifiEditor = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        panel.addView(wifiEditor, LinearLayout.LayoutParams(-1, -2))
        wifiCurrent = label(wifiEditor, "", 14, muted = true)
        wifiBand = message(wifiEditor)
        label(wifiEditor, "Wi-Fiの名前（SSID）", 14, muted = true)
        wifiSsid = field(wifiEditor, "Wi-Fiの名前（SSID）", EditorInfo.IME_ACTION_NEXT) { wifiPassword.requestFocus() }
        wifiSsid.addTextChangedListener(changed { renderWifi() })
        label(wifiEditor, "パスワード", 14, muted = true)
        wifiPassword = field(wifiEditor, "Wi-Fiのパスワード", EditorInfo.IME_ACTION_DONE) { saveWifi() }
        wifiPassword.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
        wifiPassword.addTextChangedListener(changed { renderWifi() })
        wifiMessage = message(wifiEditor)
        val editorActions = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        wifiEditor.addView(editorActions, LinearLayout.LayoutParams(-1, -2))
        wifiCancelButton = button(editorActions, "やめる", weight = true) { closeWifiAdder() }
        wifiSaveButton = button(editorActions, "保存", weight = true) { saveWifi() }
        renderWifi()
    }

    private fun loadWifi() {
        networksRead = null
        renderWifi()
        background({ api.wifiNetworks() }) { result ->
            networksRead = result is ApiResult.Ok
            if (result is ApiResult.Ok) networks = result.value
            wifiError = (result as? ApiResult.Failed)?.error
            renderWifi()
        }
    }

    /** Runs a change on home-link; on success runs [done] and reads the networks again. */
    private fun <T> changeWifi(call: () -> ApiResult<T>, done: (T) -> Unit = {}) {
        wifiBusy = true
        wifiNotice = null
        renderWifi()
        background(call) { result ->
            wifiBusy = false
            when (result) {
                is ApiResult.Ok -> {
                    done(result.value)
                    loadWifi()
                }
                is ApiResult.Failed -> {
                    wifiNotice = LightsMessage(failure(result.error), failed = true)
                    if (addingWifi) show(wifiMessage, failure(result.error))
                    // The list is unchanged; this also puts the choice back.
                    renderWifi()
                }
            }
        }
    }

    private fun failure(error: ApiError) = when (error) {
        ApiError.INVALID_WIFI -> WIFI_LIMITS
        else -> failure(error, homeLinkHost(this)).text
    }

    private fun chooseWifi(position: Int) {
        val network = networks.getOrNull(position) ?: return
        if (network.forDevices || wifiBusy) return
        changeWifi({ api.chooseWifi(network.id) })
    }

    private fun openWifiAdder() {
        addingWifi = true
        wifiNotice = null
        currentWifi = null
        currentRead = false
        wifiSsid.setText("")
        wifiPassword.setText("")
        wifiMessage.visibility = View.GONE
        renderWifi()
        wifiSsid.requestFocus()
        readCurrentWifi(ask = true)
    }

    private fun closeWifiAdder() {
        addingWifi = false
        wifiSsid.setText("")
        wifiPassword.setText("")
        wifiMessage.visibility = View.GONE
        getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(wifiPassword.windowToken, 0)
        renderWifi()
    }

    /** Fills in the network the phone is on, asking for the location permission Android needs to name it. */
    private fun readCurrentWifi(ask: Boolean) {
        if (wifiReaderFactory == null && LOCATION_PERMISSIONS.any { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }) {
            if (ask) return requestPermissions(LOCATION_PERMISSIONS, REQUEST_LOCATION)
            locationDenied = true
        } else {
            locationDenied = false
        }
        wifiReader.read { wifi ->
            if (isDestroyed || !addingWifi) return@read
            currentWifi = wifi
            currentRead = true
            val ssid = wifi?.ssid
            if (ssid != null && wifiSsid.text.isEmpty()) {
                wifiSsid.setText(ssid)
                wifiPassword.requestFocus()
            }
            renderWifi()
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == REQUEST_LOCATION) readCurrentWifi(ask = false)
    }

    private fun saveWifi() {
        val network = WifiNetwork(wifiSsid.text.toString(), wifiPassword.text.toString())
        // SSIDs are up to 32 bytes; WPA passphrases up to 64 characters.
        if (network.ssid.toByteArray().size !in 1..32 || network.password.length !in 1..64) {
            show(wifiMessage, WIFI_LIMITS)
            return
        }
        wifiMessage.visibility = View.GONE
        changeWifi({ api.saveWifi(network) }) { closeWifiAdder() }
    }

    private fun deleteWifi() {
        val network = networks.firstOrNull { it.forDevices } ?: return
        changeWifi({ api.deleteWifi(network.id) }) { wifiNotice = LightsMessage("「${network.ssid}」を削除しました", failed = false) }
    }

    private fun renderWifi() {
        val ssids = networks.map { it.ssid }
        if (wifiAdapter.count != ssids.size || (0 until wifiAdapter.count).any { wifiAdapter.getItem(it) != ssids[it] }) {
            wifiAdapter.clear()
            wifiAdapter.addAll(ssids)
        }
        val chosen = networks.indexOfFirst { it.forDevices }
        if (chosen >= 0 && wifiSpinner.selectedItemPosition != chosen) wifiSpinner.setSelection(chosen)
        val read = networksRead
        wifiState.text = when {
            addingWifi -> ""
            read == null && ssids.isEmpty() -> "Wi-Fiを読み込み中…"
            read == false -> "Wi-Fiを読み込めませんでした"
            ssids.isEmpty() -> "まだ保存していません"
            else -> ""
        }
        wifiState.visibility = if (wifiState.text.isEmpty()) View.GONE else View.VISIBLE
        val failed = read == false && !addingWifi
        // Without home-link's URL there is nothing to reconnect to; saving the URL reads the networks.
        wifiReload.visibility = if (failed && wifiError != ApiError.NOT_CONFIGURED) View.VISIBLE else View.GONE
        wifiSpinner.visibility = if (ssids.isEmpty() || addingWifi || failed) View.GONE else View.VISIBLE
        wifiSpinner.isEnabled = !wifiBusy
        wifiActions.visibility = if (addingWifi || read != true) View.GONE else View.VISIBLE
        wifiAddButton.isEnabled = !wifiBusy
        wifiDeleteButton.isEnabled = !wifiBusy
        wifiDeleteButton.visibility = if (ssids.isEmpty()) View.GONE else View.VISIBLE
        wifiEditor.visibility = if (addingWifi) View.VISIBLE else View.GONE
        wifiSaveButton.isEnabled = !wifiBusy && wifiSsid.text.isNotEmpty() && wifiPassword.text.isNotEmpty()
        wifiCancelButton.isEnabled = !wifiBusy
        val current = currentWifi
        wifiCurrent.text = when {
            !currentRead -> ""
            current == null -> "Wi-Fiにつながっていません。機器に渡すWi-Fiの名前を入力してください"
            current.ssid != null -> "今つながっているWi-Fiの名前を入れました。パスワードを入力してください"
            locationDenied -> "位置情報の「正確な位置」を許可すると、今つながっているWi-Fiの名前を自動で入れます"
            else -> "今つながっているWi-Fiの名前を読めませんでした。位置情報がオンか確かめるか、名前を入力してください"
        }
        wifiCurrent.visibility = if (wifiCurrent.text.isEmpty()) View.GONE else View.VISIBLE
        val frequency = current?.frequencyMhz
        show(wifiBand, if (frequency == null || is24GHz(frequency)) null else
            (current.ssid?.let { "今つながっている「$it」" } ?: "今つながっているWi-Fi") + "は${band(frequency)}です。" +
            "2.4GHzにしかつながらない機器があります。ルーターが2.4GHzを別の名前で出している場合は、その名前に直してください")
        val notice = when {
            addingWifi -> null
            failed -> wifiError?.let { LightsMessage(failure(it), failed = true) }
            else -> wifiNotice
        }
        show(wifiNoticeView, notice?.text, failure = notice?.failed == true)
    }

    private fun urlField(parent: LinearLayout, description: String, example: String, action: Int,
                         submit: () -> Unit): EditText = field(parent, description, action, MAX_URL, submit).apply {
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
        hint = example
        // A long URL wraps instead of running off the field with larger text; the input stays one line.
        setHorizontallyScrolling(false)
        maxLines = Int.MAX_VALUE
    }

    private fun fallbacks() = fallbackField.text.lines().map(String::trim).filter(String::isNotEmpty)

    /** Shows what is wrong with each entered URL; true when all may be saved. */
    private fun check(): Boolean {
        val homeLink = homeLinkField.text.toString().trim()
        show(fallbackMessage, fallbacks().firstOrNull { !isHomeLinkUrl(it) }?.let {
            "「$it」はURLではありません。http://かhttps://で始まるURLを1行に1つ入れてください（例：$FALLBACK_EXAMPLE）"
        })
        val bleProxy = bleProxyField.text.toString().trim()
        show(homeLinkMessage, when {
            homeLink.isEmpty() -> "home-linkのURLを入れてください（例：$HOME_LINK_EXAMPLE）"
            !isHomeLinkUrl(homeLink) -> "http://かhttps://で始まるURLを入れてください（例：$HOME_LINK_EXAMPLE）"
            else -> null
        })
        show(bleProxyMessage, if (bleProxy.isEmpty() || isBleProxyUrl(bleProxy)) null
            else "ws://かwss://で始まるURLを入れてください（例：$BLE_PROXY_EXAMPLE）")
        return homeLinkMessage.text.isEmpty() && fallbackMessage.text.isEmpty() && bleProxyMessage.text.isEmpty()
    }

    private fun save() {
        getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(bleProxyField.windowToken, 0)
        if (!check()) return resultRow.show("URLを直してから保存してください", failed = true)
        val endpoints = Endpoints(homeLinkField.text.toString().trim(), bleProxyField.text.toString().trim(), fallbacks())
        if (!Endpoints.save(this, endpoints)) return resultRow.show("保存できませんでした。端末の保存領域を確かめてください", failed = true)
        resultRow.show("保存しました。次の要求からこの接続先へ送ります", failed = false)
        loadWifi()
        // Android 17 blocks connections to LAN addresses, a likely place for home-link, without it.
        if (Build.VERSION.SDK_INT >= 37 && checkSelfPermission(Manifest.permission.ACCESS_LOCAL_NETWORK) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.ACCESS_LOCAL_NETWORK), 0)
        }
    }
}

private const val WIFI_LIMITS = "Wi-Fiの名前（32バイトまで）とパスワード（64文字まで）を入力してください"
