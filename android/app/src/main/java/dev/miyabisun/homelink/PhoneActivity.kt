package dev.miyabisun.homelink

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.location.LocationManager
import android.location.LocationRequest
import android.os.Build
import android.os.Bundle
import android.os.CancellationSignal
import android.widget.Button
import android.widget.TextView

/**
 * Registers this phone on home-link as one whose place a routine may wait on, and pins the home's place from where the
 * phone is now. Both ask for the location permission, which the routine widgets then read the location with.
 */
class PhoneActivity : Activity() {
    companion object {
        private const val REQUEST_REGISTER = 1
        private const val REQUEST_PIN = 2
        /** How long one fix is waited for when pinning the home. */
        private const val FIX_WITHIN_MS = 20_000L
        private val edits = Outbox.edits<PhoneActivity>()
        private val LOCATION_PERMISSIONS = arrayOf(Manifest.permission.ACCESS_FINE_LOCATION,
            Manifest.permission.ACCESS_COARSE_LOCATION)
    }

    private lateinit var api: HomeLinkApi
    private lateinit var reads: Reads
    /** null while home-link is being read. */
    private var registered: Boolean? = null
    private var pinned: Boolean? = null
    private var error: ApiError? = null
    private var busy = false
    private var fixing: CancellationSignal? = null

    private lateinit var phoneState: TextView
    private lateinit var registerButton: Button
    private lateinit var unregisterButton: Button
    private lateinit var homeState: TextView
    private lateinit var pinButton: Button
    private lateinit var resultRow: ResultRow
    private lateinit var reloadButton: Button

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        api = homeLinkApi(this)
        reads = Reads(this)
        edits.follow(this)
        val content = headedPage(home = ::finish, subtitle = "この電話の位置")
        val phone = panel(content)
        phoneState = label(phone, "", 16)
        registerButton = button(phone, "この電話を登録", primary = true) { register() }
        unregisterButton = button(phone, "登録を外す") { unregister() }
        val home = panel(content)
        homeState = label(home, "", 16)
        pinButton = button(home, "今いる場所を自宅にする") { pin() }
        resultRow = ResultRow(this, content, dp(12))
        reloadButton = button(content, "再接続") { load() }
        load()
    }

    override fun onDestroy() {
        fixing?.cancel()
        edits.unfollow(this)
        reads.close()
        super.onDestroy()
    }

    private fun load() {
        registered = null
        pinned = null
        error = null
        render()
        val id = PhoneIdentity.id(this)
        reads.run({ api.phones() }) { result ->
            when (result) {
                is ApiResult.Ok -> registered = id in result.value
                is ApiResult.Failed -> error = result.error
            }
            render()
        }
        reads.run({ api.homePinned() }) { result ->
            when (result) {
                is ApiResult.Ok -> pinned = result.value
                is ApiResult.Failed -> error = result.error
            }
            render()
        }
    }

    private fun allowed() = checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED

    /** Registers the phone once the location may be read: a phone that cannot tell its place ends no wait. */
    private fun register() {
        if (!allowed()) return requestPermissions(LOCATION_PERMISSIONS, REQUEST_REGISTER)
        val id = PhoneIdentity.id(this)
        send({ api.registerPhone(id, Build.MODEL) }) {
            registered = true
            resultRow.show("この電話を登録しました", failed = false)
        }
    }

    private fun unregister() {
        val id = PhoneIdentity.id(this)
        send({ api.unregisterPhone(id) }) {
            registered = false
            resultRow.show("この電話の登録を外しました", failed = false)
        }
    }

    /** Reads where the phone is now, afresh, and pins the home there when the fix is sure enough. */
    private fun pin() {
        if (!allowed()) return requestPermissions(LOCATION_PERMISSIONS, REQUEST_PIN)
        val manager = getSystemService(LocationManager::class.java)
        val provider = listOf(LocationManager.FUSED_PROVIDER, LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER)
            .firstOrNull { manager.hasProvider(it) && manager.isProviderEnabled(it) }
        if (provider == null || !manager.isLocationEnabled) {
            return resultRow.show(WatchEnd.LOCATION_OFF.message, failed = true)
        }
        busy = true
        resultRow.show("今いる場所を測っています", failed = false, progress = R.drawable.ic_home_link)
        render()
        val signal = CancellationSignal().also { fixing = it }
        val request = LocationRequest.Builder(0).setQuality(LocationRequest.QUALITY_HIGH_ACCURACY)
            .setDurationMillis(FIX_WITHIN_MS).setMaxUpdates(1).build()
        try {
            manager.getCurrentLocation(provider, request, signal, mainExecutor) { location -> pinAt(location?.fix()) }
        } catch (_: SecurityException) {
            // Taken back between the check and the read.
            fixing = null
            busy = false
            resultRow.show(WatchEnd.NO_PERMISSION.message, failed = true)
            render()
        }
    }

    /** Pins the home at `fix`, the place just measured, unless there is none or it is too unsure. */
    private fun pinAt(fix: Fix?) {
        fixing = null
        busy = false
        val refusal = if (fix == null) "今いる場所を測れませんでした。屋外か窓の近くでやり直してください" else pinRefusal(fix)
        if (fix == null || refusal != null) {
            resultRow.show(refusal, failed = true)
            return render()
        }
        send({ api.pinHome(fix.latitude, fix.longitude) }) {
            pinned = true
            resultRow.show("今いる場所を自宅にしました（精度${Math.round(fix.accuracy)}m）", failed = false)
        }
    }

    private fun send(call: () -> ApiResult<Unit>, done: () -> Unit) {
        busy = true
        render()
        edits.send(call) { result ->
            busy = false
            when (result) {
                is ApiResult.Ok -> done()
                is ApiResult.Failed -> resultRow.show(failure(result.error, homeLinkHost(this)).text, failed = true)
            }
            render()
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        if (!allowed()) {
            resultRow.show("位置情報が許可されていません。電話の設定でこのアプリに位置情報（正確な位置）を許可してください", failed = true)
            return
        }
        // The widgets drawn before the permission read the location from now on.
        Widgets.showAll(this)
        if (requestCode == REQUEST_REGISTER) register() else pin()
    }

    private fun render() {
        val failed = error
        phoneState.text = when (registered) {
            true -> "この電話：登録済み"
            false -> "この電話：未登録"
            null -> "この電話"
        }
        registerButton.visibility = if (registered == true) android.view.View.GONE else android.view.View.VISIBLE
        unregisterButton.visibility = if (registered == true) android.view.View.VISIBLE else android.view.View.GONE
        homeState.text = when (pinned) {
            true -> "自宅の位置：登録済み"
            false -> "自宅の位置：未登録"
            null -> "自宅の位置"
        }
        registerButton.isEnabled = registered != null && !busy
        unregisterButton.isEnabled = !busy
        pinButton.isEnabled = pinned != null && !busy
        if (failed != null) resultRow.show(failure(failed, homeLinkHost(this)).text, failed = true)
        reloadButton.visibility = if (failed != null) android.view.View.VISIBLE else android.view.View.GONE
    }
}
