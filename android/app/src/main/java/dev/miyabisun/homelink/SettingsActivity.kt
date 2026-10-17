package dev.miyabisun.homelink

import android.app.Activity
import android.os.Bundle
import android.text.InputType
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView

/** Edits where the app sends its requests, in the same YAML file that `adb push` may write. */
class SettingsActivity : Activity() {
    private companion object {
        const val MAX_URL = 2048
        const val HOME_LINK_EXAMPLE = "http://home-link.example:5011"
        const val BLE_PROXY_EXAMPLE = "ws://192.0.2.10:5580/ble"
    }

    private lateinit var homeLinkField: EditText
    private lateinit var homeLinkMessage: TextView
    private lateinit var bleProxyField: EditText
    private lateinit var bleProxyMessage: TextView
    private lateinit var resultRow: ResultRow

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val content = page()
        label(content, "接続先の設定", 28, bold = true)

        val homeLink = panel(content)
        label(homeLink, "home-link", 18, bold = true)
        label(homeLink, "照明・登録・玄関の要求を送るURL", 14, muted = true)
        homeLinkField = urlField(homeLink, "home-linkのURL", HOME_LINK_EXAMPLE, EditorInfo.IME_ACTION_NEXT) { bleProxyField.requestFocus() }
        homeLinkMessage = message(homeLink)

        val bleProxy = panel(content)
        label(bleProxy, "matterjs-serverのBLE Proxy", 18, bold = true)
        label(bleProxy, "Bluetoothで機器を登録するときに使うURL。登録しないなら空欄で構いません", 14, muted = true)
        bleProxyField = urlField(bleProxy, "BLE ProxyのURL", BLE_PROXY_EXAMPLE, EditorInfo.IME_ACTION_DONE) { save() }
        bleProxyMessage = message(bleProxy)

        label(content, "保存先：${Endpoints.path(this)}", 14, muted = true)
        resultRow = ResultRow(this, content, dp(16))
        button(content, "保存", primary = true) { save() }

        val saved = Endpoints.load(this)
        homeLinkField.setText(savedInstanceState?.getString("homeLink") ?: saved.homeLink)
        bleProxyField.setText(savedInstanceState?.getString("bleProxy") ?: saved.bleProxy)
        // A malformed value, as `adb push` may leave, is shown with what is wrong before any save.
        check()
        resultRow.show(null, failed = false)
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putString("homeLink", homeLinkField.text.toString())
        outState.putString("bleProxy", bleProxyField.text.toString())
        super.onSaveInstanceState(outState)
    }

    private fun urlField(parent: LinearLayout, description: String, example: String, action: Int,
                         submit: () -> Unit): EditText = field(parent, description, action, MAX_URL, submit).apply {
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
        hint = example
    }

    /** Shows what is wrong with each entered URL; true when both may be saved. */
    private fun check(): Boolean {
        val homeLink = homeLinkField.text.toString().trim()
        val bleProxy = bleProxyField.text.toString().trim()
        show(homeLinkMessage, when {
            homeLink.isEmpty() -> "home-linkのURLを入れてください（例：$HOME_LINK_EXAMPLE）"
            !isHomeLinkUrl(homeLink) -> "http://かhttps://で始まるURLを入れてください（例：$HOME_LINK_EXAMPLE）"
            else -> null
        })
        show(bleProxyMessage, if (bleProxy.isEmpty() || isBleProxyUrl(bleProxy)) null
            else "ws://かwss://で始まるURLを入れてください（例：$BLE_PROXY_EXAMPLE）")
        return homeLinkMessage.text.isEmpty() && bleProxyMessage.text.isEmpty()
    }

    private fun save() {
        getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(bleProxyField.windowToken, 0)
        if (!check()) return resultRow.show("URLを直してから保存してください", failed = true)
        val endpoints = Endpoints(homeLinkField.text.toString().trim(), bleProxyField.text.toString().trim())
        if (Endpoints.save(this, endpoints)) resultRow.show("保存しました。次の要求からこの接続先へ送ります", failed = false)
        else resultRow.show("保存できませんでした。端末の保存領域を確かめてください", failed = true)
    }
}
