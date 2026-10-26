package dev.miyabisun.homelink

import okhttp3.Call
import okhttp3.Connection
import okhttp3.ConnectionPool
import okhttp3.EventListener
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject
import java.io.IOException
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

enum class ApiError { NOT_CONFIGURED, UNREACHABLE, TIMEOUT, INVALID_QR, INVALID_MANUAL, INVALID_NAME,
    MATTER_UNREACHABLE, MATTER_NOT_CONFIGURED, SERVER,
    DEVICE_NOT_FOUND, WRONG_CODE, WIFI_FAILED, COMMISSION_TIMEOUT, COMMISSION_FAILED, BLUETOOTH_UNAVAILABLE, INVALID_WIFI,
    THREAD_FAILED, WIFI_NOT_SET, WIFI_NOT_FOUND, LABEL_NOT_FOUND, LABEL_HAS_NO_DEVICES, DUPLICATE_LABEL, INVALID_LABEL,
    WIFI_REVERTED, WIFI_REVERTING, NOT_ON_WIFI,
    SESAME_NOT_CONFIGURED, SESAME_REJECTED, SESAME_UNREACHABLE, SWITCH_FAILED, NO_NETWORK, BLOCKED,
    CONNECT_TIMEOUT, HTTP_DENIED, HTTP_NOT_FOUND, HTTP_GATEWAY, BAD_ANSWER,
    SETTINGS_UNREADABLE, SETTINGS_MALFORMED, LOCAL_NETWORK_DENIED }

/** A Wi-Fi network to save on home-link for new devices. */
data class WifiNetwork(val ssid: String, val password: String)

/** A Wi-Fi network home-link holds; it never sends the password back. */
data class SavedNetwork(val id: Long, val ssid: String, val forDevices: Boolean)

/** How matterjs-server sees a ledger device; none when home-link cannot read it. */
enum class DeviceState {
    UNCONNECTED, CONNECTED, NO_RESPONSE;

    companion object {
        fun of(value: String?): DeviceState? = entries.firstOrNull { it.name.lowercase() == value }
    }
}

/** What home-link tells a connected device to be by its Matter device types. */
enum class DeviceKind {
    LIGHT, PLUG, OTHER;

    companion object {
        fun of(value: String?): DeviceKind? = entries.firstOrNull { it.name.lowercase() == value }
    }
}

/**
 * A ledger device; [code] is its QR payload or manual code, read only for one device at a time.
 * [wifiSsid] is the Wi-Fi it reports being on; [on] the On/Off last read of a device that switches and is not a light.
 */
data class Device(val id: Long, val name: String, val labelIds: Set<Long>, val state: DeviceState?,
                  val code: String?, val wifiSsid: String? = null, val kind: DeviceKind? = null, val on: Boolean? = null,
                  val productName: String? = null, val leftFabric: Boolean = false)

/** How a device reads in lists: its name, else its product, else its printed code. */
val Device.title: String get() = name.ifEmpty { productName ?: code?.let { ManualCode.label(it) ?: it } ?: "機器" }

/** A device once connected that matterjs-server serves no more left the fabric: registering it again brings it back. */
fun stateText(device: Device): String = when (device.state) {
    DeviceState.UNCONNECTED -> if (device.leftFabric) "網から外れた" else "未接続"
    DeviceState.CONNECTED -> "接続済み"
    DeviceState.NO_RESPONSE -> "応答なし"
    null -> ""
}

/** How a device still to connect reads in the list: one that left the fabric says so. */
fun listTitle(device: Device): String = if (device.leftFabric) "${device.title}（網から外れた）" else device.title

/** One field of a device to change. */
sealed interface DeviceUpdate {
    data class Name(val name: String) : DeviceUpdate
    data class Labels(val ids: Set<Long>) : DeviceUpdate
}

sealed interface ApiResult<out T> {
    data class Ok<T>(val value: T) : ApiResult<T>
    data class Failed(val error: ApiError) : ApiResult<Nothing>
}

/**
 * What switching the air conditioner's control did: its new state [on], kept by home-link either way, and whether the
 * air conditioner took it. Not [delivered], home-link sends it again every 10 minutes. [result] is home-link's word
 * for the send and [reason] its sentence, both null from a home-link that answered before sending.
 */
data class AirconToggle(val on: Boolean, val delivered: Boolean, val result: String? = null, val reason: String? = null)

/** The home-link ledger API. Calls block; run them off the main thread. */
interface HomeLinkApi {
    fun devices(): ApiResult<List<Device>>
    fun device(id: Long): ApiResult<Device>
    /** Enters the device behind `payload` (a QR payload, `MT:`, or a manual code's digits), or returns the one entered. */
    fun addDevice(payload: String): ApiResult<Device>
    fun updateDevice(id: Long, update: DeviceUpdate): ApiResult<Device>
    fun labels(): ApiResult<List<Label>>
    /** Switches label `id`'s lights and plugs on or off, or toggles them. */
    fun switchLabel(id: Long, op: LabelOp): ApiResult<LabelResult>
    /** Whether the air conditioner's control is on: off holds it off until switched on again. */
    fun aircon(): ApiResult<Boolean>
    /** Switches the air conditioner's control the other way: its new state, and whether the air conditioner took it. */
    fun toggleAircon(): ApiResult<AirconToggle>
    /**
     * Has matterjs-server commission ledger device `id` by its code over the BLE proxy this phone holds open;
     * home-link hands over its Wi-Fi for devices, and matterjs-server picks Wi-Fi or Thread. Takes up to minutes.
     */
    fun commission(id: Long): ApiResult<Device>
    /** Moves connected device `id` onto home-link's Wi-Fi for devices without resetting it. Takes up to minutes. */
    fun moveWifi(id: Long): ApiResult<Device>
    /** Switches device `id`, such as a plug, on or off: its On/Off, or null when it did not answer. */
    fun switchDevice(id: Long, on: Boolean): ApiResult<Boolean?>
    fun wifiNetworks(): ApiResult<List<SavedNetwork>>
    /** Saves the network, replacing the password of one with the same name; the first becomes the one for devices. */
    fun saveWifi(network: WifiNetwork): ApiResult<SavedNetwork>
    /** Makes the network the one handed to new devices. */
    fun chooseWifi(id: Long): ApiResult<SavedNetwork>
    fun deleteWifi(id: Long): ApiResult<Unit>
    /** Has the front door's SESAME unlock or lock; `Ok` once CANDY HOUSE accepts the command. */
    fun door(command: DoorCommand): ApiResult<Unit>
}

/** A phone taken as always online, waiting by sleeping: for tests and the JVM. */
object SteadyLink : PhoneLink {
    override fun state() = Link.USABLE
    override fun await(ms: Long) = Thread.sleep(ms)
}

/** Ends an attempt that connected after another URL had: it must not send. */
private class Yielded : IOException("another URL connected first")

/**
 * How long a request may take: `patience` ms to connect, as [Reach] keeps trying, then `readMs` for each read of the
 * answer. One for every screen and press; only a broadcast receiver, which has 10 s in all, reads with less.
 */
enum class Limits(val patience: Long, val readMs: Int) {
    /**
     * Over mobile data or a network switching, a path to home-link may take a while to come up. Switching a label
     * waits up to 10 s for every device's answer.
     */
    SCREEN(15_000, 20_000),
    RECEIVER(5_000, 3_000),
}

/**
 * Calls home-link at the URLs `targets` gives for each request: the primary, racing the fallbacks while it cannot
 * connect. None sends nothing. `link` is the phone's network, `log` takes a line for each failed attempt, and `now`
 * is a monotonic clock in ms. Every request keeps to `limits`, but for the minutes registering takes.
 */
class HttpHomeLinkApi(private val targets: () -> Targets, private val link: PhoneLink = SteadyLink,
                      private val log: (String) -> Unit = {}, private val limits: Limits = Limits.SCREEN,
                      now: () -> Long = { System.nanoTime() / 1_000_000 }) : HomeLinkApi {
    constructor(baseUrl: String) : this({ Targets(listOf(baseUrl)) })

    private val reach = Reach(link, now = now)

    private companion object {
        /**
         * Keeps no connection for a later request: one left from before a network switch fails, or hangs, when used
         * again, and a request never retries on its own. Each request connects afresh.
         */
        val CLIENT = OkHttpClient.Builder().retryOnConnectionFailure(false)
            .connectionPool(ConnectionPool(0, 1, TimeUnit.SECONDS)).build()
        val JSON = "application/json".toMediaType()
        val EMPTY = ByteArray(0).toRequestBody()
    }

    override fun devices(): ApiResult<List<Device>> = call("GET", "/api/devices", null) { body ->
        val array = JSONArray(body)
        (0 until array.length()).map { device(array.getJSONObject(it)) }
    }

    override fun device(id: Long): ApiResult<Device> = call("GET", "/api/devices/$id", null) { device(JSONObject(it)) }

    override fun addDevice(payload: String): ApiResult<Device> {
        val field = if (payload.startsWith("MT:")) "qr_payload" else "manual_code"
        return call("POST", "/api/devices", JSONObject().put(field, payload)) { device(JSONObject(it).getJSONObject("device")) }
    }

    override fun updateDevice(id: Long, update: DeviceUpdate): ApiResult<Device> {
        val body = when (update) {
            is DeviceUpdate.Name -> JSONObject().put("name", update.name)
            is DeviceUpdate.Labels -> JSONObject().put("label_ids", JSONArray(update.ids.sorted()))
        }
        return call("PATCH", "/api/devices/$id", body) { device(JSONObject(it)) }
    }

    override fun labels(): ApiResult<List<Label>> = call("GET", "/api/labels", null) { body ->
        val array = JSONArray(body)
        (0 until array.length()).map { label(array.getJSONObject(it)) }
    }

    override fun switchLabel(id: Long, op: LabelOp): ApiResult<LabelResult> =
        call("POST", "/api/labels/$id/${op.path}", null) { body ->
            val json = JSONObject(body)
            LabelResult(json.getString("action"), if (json.isNull("on")) null else json.getBoolean("on"), counts(json))
        }

    override fun aircon(): ApiResult<Boolean> =
        call("GET", "/api/aircon", null) { JSONObject(it).getBoolean("on") }

    // home-link answers once the air conditioner took the switch, or did not, in about 12 s at most.
    override fun toggleAircon(): ApiResult<AirconToggle> =
        call("POST", "/api/aircon/toggle", null) { body ->
            val json = JSONObject(body)
            AirconToggle(json.getBoolean("on"), json.isNull("delivered") || json.getBoolean("delivered"),
                json.optString("result").takeUnless { json.isNull("result") },
                json.optString("reason").takeUnless { json.isNull("reason") })
        }

    private fun label(json: JSONObject): Label {
        val schedule = json.optJSONArray("schedule") ?: JSONArray()
        val level = json.optInt("override_level").takeUnless { json.isNull("override_level") }
        val kelvin = json.optInt("override_kelvin").takeUnless { json.isNull("override_kelvin") }
        return Label(
            json.getLong("id"), json.getString("name"), json.optBoolean("auto"),
            if (level != null && kelvin != null) Override(level, kelvin) else null,
            if (json.isNull("on")) null else json.optBoolean("on"),
            (0 until schedule.length()).mapNotNull { index ->
                val entry = schedule.getJSONObject(index)
                minuteOf(entry.getString("time"))?.let { ScheduleEntry(it, entry.getBoolean("on")) }
            },
        )
    }

    // matterjs-server may take minutes: Bluetooth discovery, PASE, the network join and CASE.
    override fun commission(id: Long): ApiResult<Device> =
        call("POST", "/api/devices/$id/commission", null, readMs = 360_000) { device(JSONObject(it).getJSONObject("device")) }

    // Under a two-minute fail-safe, then a minute for the device to come back.
    override fun moveWifi(id: Long): ApiResult<Device> =
        call("POST", "/api/devices/$id/wifi", null, readMs = 360_000) { device(JSONObject(it).getJSONObject("device")) }

    override fun switchDevice(id: Long, on: Boolean): ApiResult<Boolean?> =
        call("POST", "/api/devices/$id/${if (on) "on" else "off"}", null) { body ->
            when (JSONObject(body).getString("state")) {
                "on" -> true
                "off" -> false
                else -> null
            }
        }

    private fun device(json: JSONObject): Device {
        val labels = json.getJSONArray("labels")
        return Device(
            json.getLong("id"), json.getString("name"),
            (0 until labels.length()).map { labels.getJSONObject(it).getLong("id") }.toSet(),
            DeviceState.of(json.optString("state").takeUnless { json.isNull("state") }),
            json.optString("qr_payload").ifEmpty { json.optString("manual_code") }.ifEmpty { null },
            json.optString("wifi_ssid").takeUnless { json.isNull("wifi_ssid") },
            DeviceKind.of(json.optString("kind").takeUnless { json.isNull("kind") }),
            if (json.isNull("on")) null else json.optBoolean("on"),
            json.optString("product_name").takeUnless { json.isNull("product_name") },
            json.optBoolean("left_fabric", false),
        )
    }

    override fun wifiNetworks(): ApiResult<List<SavedNetwork>> = call("GET", "/api/wifi", null) { body ->
        val array = JSONArray(body)
        (0 until array.length()).map { savedNetwork(array.getJSONObject(it)) }
    }

    override fun saveWifi(network: WifiNetwork): ApiResult<SavedNetwork> =
        call("POST", "/api/wifi", JSONObject().put("ssid", network.ssid).put("password", network.password)) {
            savedNetwork(JSONObject(it))
        }

    override fun chooseWifi(id: Long): ApiResult<SavedNetwork> =
        call("PUT", "/api/wifi/devices", JSONObject().put("id", id)) { savedNetwork(JSONObject(it)) }

    override fun deleteWifi(id: Long): ApiResult<Unit> = call("DELETE", "/api/wifi/$id", null) { }

    private fun savedNetwork(json: JSONObject) =
        SavedNetwork(json.getLong("id"), json.getString("ssid"), json.getBoolean("for_devices"))

    override fun door(command: DoorCommand): ApiResult<Unit> = call("POST", "/api/door/${command.path}", null) { }

    private fun counts(json: JSONObject): SwitchResult {
        val missing = json.getJSONArray("missing_devices")
        return SwitchResult(
            json.getInt("switched"), json.getInt("no_response"), json.getInt("failed"),
            (0 until missing.length()).map { index ->
                val device = missing.getJSONObject(index)
                device.getString("name").ifEmpty { "機器" }
            },
        )
    }

    /**
     * Sends one request, connecting for up to [Limits.patience] ms as [Reach] does; once connected, it waits `readMs`
     * for each read of the answer. Every failed attempt is logged with the phone's network.
     */
    private fun <T> call(method: String, path: String, body: JSONObject?, readMs: Int = limits.readMs,
                         parse: (String) -> T): ApiResult<T> {
        val to = targets()
        if (to.urls.isEmpty()) return ApiResult.Failed(to.none)
        return to.explain(send(to.urls, method, path, body, readMs, parse))
    }

    private fun <T> send(urls: List<String>, method: String, path: String, body: JSONObject?, readMs: Int,
                         parse: (String) -> T): ApiResult<T> =
        reach.send(urls.map { it.trimEnd('/') }, limits.patience) { base, connectMs, race ->
            // OkHttp, unlike the JDK's HttpURLConnection, sends PATCH.
            val request = try {
                Request.Builder().url(base + path).header("accept", "application/json")
                    .method(method, body?.toString()?.toRequestBody(JSON) ?: if (method == "POST") EMPTY else null).build()
            } catch (_: IllegalArgumentException) {
                return@send Attempt.Missed(Miss.UNRESOLVED)
            }
            val connected = AtomicBoolean()
            val client = CLIENT.newBuilder().connectTimeout(connectMs, TimeUnit.MILLISECONDS)
                .readTimeout(readMs.toLong(), TimeUnit.MILLISECONDS)
                // Name lookups have no timeout of their own; this bounds one that hangs while the network changes.
                .callTimeout(connectMs + readMs, TimeUnit.MILLISECONDS)
                .eventListener(object : EventListener() {
                    // Before anything is written: only the first URL to connect may send.
                    override fun connectionAcquired(call: Call, connection: Connection) {
                        if (!race.claim()) throw Yielded()
                        connected.set(true)
                    }
                }).build()
            val call = client.newCall(request)
            race.onCancel { if (!connected.get()) call.cancel() }
            try {
                call.execute().use { response ->
                    val text = response.body?.string().orEmpty()
                    Attempt.Answered(if (response.isSuccessful) ApiResult.Ok(parse(text)) else ApiResult.Failed(apiError(response.code, text)))
                }
            } catch (e: IOException) {
                if (e is Yielded || e.cause is Yielded || (call.isCanceled() && !connected.get())) return@send Attempt.Yielded
                miss(e, connected.get()).also { log("$method ${request.url.host}:${request.url.port}$path: ${e.javaClass.simpleName} ${e.message} → $it, phone ${link.state()}") }
                    .let { Attempt.Missed(it) }
            } catch (_: JSONException) {
                log("$method ${request.url.host}:${request.url.port}$path: an answer that is not home-link's JSON")
                Attempt.Answered(ApiResult.Failed(ApiError.BAD_ANSWER))
            }
        }
}

/** What a failed answer means: home-link's own error code, else what its status says answered instead. */
internal fun apiError(status: Int, body: String): ApiError {
    val code = try { JSONObject(body).optString("error") } catch (_: JSONException) { "" }
    return when (code) {
        "invalid_qr_payload" -> ApiError.INVALID_QR
        "invalid_manual_code" -> ApiError.INVALID_MANUAL
        "invalid_device_name", "invalid_label_name" -> ApiError.INVALID_NAME
        "matter_server_unreachable" -> ApiError.MATTER_UNREACHABLE
        "matter_server_not_configured" -> ApiError.MATTER_NOT_CONFIGURED
        "device_not_found" -> ApiError.DEVICE_NOT_FOUND
        "wrong_code" -> ApiError.WRONG_CODE
        "wifi_failed" -> ApiError.WIFI_FAILED
        "commission_timeout" -> ApiError.COMMISSION_TIMEOUT
        "commission_failed" -> ApiError.COMMISSION_FAILED
        "bluetooth_unavailable" -> ApiError.BLUETOOTH_UNAVAILABLE
        "invalid_wifi" -> ApiError.INVALID_WIFI
        "thread_failed" -> ApiError.THREAD_FAILED
        "wifi_not_set" -> ApiError.WIFI_NOT_SET
        "wifi_not_found" -> ApiError.WIFI_NOT_FOUND
        "label_not_found" -> ApiError.LABEL_NOT_FOUND
        "label_has_no_devices" -> ApiError.LABEL_HAS_NO_DEVICES
        "duplicate_label_name" -> ApiError.DUPLICATE_LABEL
        "invalid_label" -> ApiError.INVALID_LABEL
        "sesame_not_configured" -> ApiError.SESAME_NOT_CONFIGURED
        "sesame_rejected" -> ApiError.SESAME_REJECTED
        "sesame_unreachable" -> ApiError.SESAME_UNREACHABLE
        "wifi_reverted" -> ApiError.WIFI_REVERTED
        "wifi_reverting" -> ApiError.WIFI_REVERTING
        "not_on_wifi" -> ApiError.NOT_ON_WIFI
        "not_switchable", "not_connected", "switch_failed" -> ApiError.SWITCH_FAILED
        else -> when (status) {
            401, 403 -> ApiError.HTTP_DENIED
            404 -> ApiError.HTTP_NOT_FOUND
            502, 503, 504 -> ApiError.HTTP_GATEWAY
            else -> ApiError.SERVER
        }
    }
}
