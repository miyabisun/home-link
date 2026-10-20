package dev.miyabisun.homelink

import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject
import java.io.IOException
import java.net.SocketTimeoutException
import java.util.concurrent.TimeUnit

data class Room(val id: Long, val name: String)

enum class ApiError { NOT_CONFIGURED, UNREACHABLE, TIMEOUT, ROOM_NOT_FOUND, DUPLICATE_ROOM, INVALID_QR, INVALID_MANUAL, INVALID_NAME,
    MATTER_UNREACHABLE, MATTER_NOT_CONFIGURED, SERVER,
    DEVICE_NOT_FOUND, WRONG_CODE, WIFI_FAILED, COMMISSION_TIMEOUT, COMMISSION_FAILED, BLUETOOTH_UNAVAILABLE, INVALID_WIFI,
    THREAD_FAILED, WIFI_NOT_SET, WIFI_NOT_FOUND, LABEL_NOT_FOUND, LABEL_HAS_NO_DEVICES, DUPLICATE_LABEL, INVALID_LABEL,
    WIFI_REVERTED, WIFI_REVERTING, NOT_ON_WIFI,
    SESAME_NOT_CONFIGURED, SESAME_REJECTED, SESAME_UNREACHABLE, SWITCH_FAILED }

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
data class Device(val id: Long, val name: String, val roomId: Long?, val labelIds: Set<Long>, val state: DeviceState?,
                  val code: String?, val wifiSsid: String? = null, val kind: DeviceKind? = null, val on: Boolean? = null,
                  val productName: String? = null)

/** How a device reads in lists: its name, else its product, else its printed code. */
val Device.title: String get() = name.ifEmpty { productName ?: code?.let { ManualCode.label(it) ?: it } ?: "機器" }

/** One field of a device to change. */
sealed interface DeviceUpdate {
    data class Name(val name: String) : DeviceUpdate
    /** Null takes the device out of its room. */
    data class Room(val id: Long?) : DeviceUpdate
    data class Labels(val ids: Set<Long>) : DeviceUpdate
}

sealed interface ApiResult<out T> {
    data class Ok<T>(val value: T) : ApiResult<T>
    data class Failed(val error: ApiError) : ApiResult<Nothing>
}

/** The home-link ledger API. Calls block; run them off the main thread. */
interface HomeLinkApi {
    fun rooms(): ApiResult<List<Room>>
    fun createRoom(name: String): ApiResult<Room>
    fun devices(): ApiResult<List<Device>>
    fun device(id: Long): ApiResult<Device>
    /** Enters the device behind `payload` (a QR payload, `MT:`, or a manual code's digits), or returns the one entered. */
    fun addDevice(payload: String): ApiResult<Device>
    fun updateDevice(id: Long, update: DeviceUpdate): ApiResult<Device>
    fun labels(): ApiResult<List<Label>>
    fun createLabel(name: String): ApiResult<Label>
    fun updateLabel(id: Long, update: LabelUpdate): ApiResult<Label>
    fun deleteLabel(id: Long): ApiResult<Unit>
    /** Switches label `id`'s lights and plugs on or off, or toggles them. */
    fun switchLabel(id: Long, op: LabelOp): ApiResult<LabelResult>
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

/** Calls home-link at the URL `baseUrl` gives for each request; none sends nothing. */
class HttpHomeLinkApi(private val baseUrl: () -> String?) : HomeLinkApi {
    constructor(baseUrl: String) : this({ baseUrl })

    private companion object {
        val CLIENT = OkHttpClient.Builder().retryOnConnectionFailure(false).build()
        val JSON = "application/json".toMediaType()
        val EMPTY = ByteArray(0).toRequestBody()
    }

    override fun rooms(): ApiResult<List<Room>> = call("GET", "/api/rooms", null) { body ->
        val array = JSONArray(body)
        (0 until array.length()).map { room(array.getJSONObject(it)) }
    }

    override fun createRoom(name: String): ApiResult<Room> =
        call("POST", "/api/rooms", JSONObject().put("name", name)) { room(JSONObject(it)) }

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
            is DeviceUpdate.Room -> JSONObject().put("room_id", update.id ?: JSONObject.NULL)
            is DeviceUpdate.Labels -> JSONObject().put("label_ids", JSONArray(update.ids.sorted()))
        }
        return call("PATCH", "/api/devices/$id", body) { device(JSONObject(it)) }
    }

    // A short wait: a widget reads this within the time a broadcast receiver may run.
    override fun labels(): ApiResult<List<Label>> = call("GET", "/api/labels", null, readTimeout = 3_000) { body ->
        val array = JSONArray(body)
        (0 until array.length()).map { label(array.getJSONObject(it)) }
    }

    override fun createLabel(name: String): ApiResult<Label> =
        call("POST", "/api/labels", JSONObject().put("name", name)) { label(JSONObject(it)) }

    override fun updateLabel(id: Long, update: LabelUpdate): ApiResult<Label> {
        val body = when (update) {
            is LabelUpdate.Name -> JSONObject().put("name", update.name)
            is LabelUpdate.Dimming -> JSONObject().put("auto", update.auto)
                .put("override_level", update.override?.level ?: JSONObject.NULL)
                .put("override_kelvin", update.override?.kelvin ?: JSONObject.NULL)
            is LabelUpdate.Schedule -> JSONObject().put("schedule", JSONArray(update.entries.sortedBy { it.minute }.map {
                JSONObject().put("time", clock(it.minute)).put("on", it.on)
            }))
        }
        return call("PATCH", "/api/labels/$id", body) { label(JSONObject(it)) }
    }

    override fun deleteLabel(id: Long): ApiResult<Unit> = call("DELETE", "/api/labels/$id", null) { }

    override fun switchLabel(id: Long, op: LabelOp): ApiResult<LabelResult> =
        call("POST", "/api/labels/$id/${op.path}", null) { body ->
            val json = JSONObject(body)
            LabelResult(json.getString("action"), if (json.isNull("on")) null else json.getBoolean("on"), counts(json))
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
        call("POST", "/api/devices/$id/commission", null, readTimeout = 360_000) { device(JSONObject(it).getJSONObject("device")) }

    // Under a two-minute fail-safe, then a minute for the device to come back.
    override fun moveWifi(id: Long): ApiResult<Device> =
        call("POST", "/api/devices/$id/wifi", null, readTimeout = 360_000) { device(JSONObject(it).getJSONObject("device")) }

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
            json.getLong("id"), json.getString("name"), json.optLong("room_id").takeUnless { json.isNull("room_id") },
            (0 until labels.length()).map { labels.getJSONObject(it).getLong("id") }.toSet(),
            DeviceState.of(json.optString("state").takeUnless { json.isNull("state") }),
            json.optString("qr_payload").ifEmpty { json.optString("manual_code") }.ifEmpty { null },
            json.optString("wifi_ssid").takeUnless { json.isNull("wifi_ssid") },
            DeviceKind.of(json.optString("kind").takeUnless { json.isNull("kind") }),
            if (json.isNull("on")) null else json.optBoolean("on"),
            json.optString("product_name").takeUnless { json.isNull("product_name") },
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

    // Longer than the lights' wait: over mobile data, a path to home-link may take a while to set up.
    override fun door(command: DoorCommand): ApiResult<Unit> =
        call("POST", "/api/door/${command.path}", null, connectTimeout = 15_000, timedOut = ApiError.TIMEOUT) { }

    private fun counts(json: JSONObject): SwitchResult {
        val missing = json.getJSONArray("missing_devices")
        return SwitchResult(
            json.getInt("switched"), json.getInt("no_response"), json.getInt("failed"),
            (0 until missing.length()).map { index ->
                val device = missing.getJSONObject(index)
                device.getString("name").ifEmpty { device.getString("room_name") + "の機器" }
            },
        )
    }

    private fun room(json: JSONObject) = Room(json.getLong("id"), json.getString("name"))

    private fun <T> call(method: String, path: String, body: JSONObject?, readTimeout: Int = 20_000,
                         connectTimeout: Int = 5_000, timedOut: ApiError = ApiError.UNREACHABLE,
                         parse: (String) -> T): ApiResult<T> {
        val base = baseUrl()?.trimEnd('/') ?: return ApiResult.Failed(ApiError.NOT_CONFIGURED)
        // OkHttp, unlike the JDK's HttpURLConnection, sends PATCH.
        val request = try {
            Request.Builder().url(base + path).header("accept", "application/json")
                .method(method, body?.toString()?.toRequestBody(JSON) ?: if (method == "POST") EMPTY else null).build()
        } catch (_: IllegalArgumentException) {
            return ApiResult.Failed(ApiError.UNREACHABLE)
        }
        val client = CLIENT.newBuilder().connectTimeout(connectTimeout.toLong(), TimeUnit.MILLISECONDS)
            // Switching a label waits up to 10 s for every device's answer.
            .readTimeout(readTimeout.toLong(), TimeUnit.MILLISECONDS).build()
        return try {
            client.newCall(request).execute().use { response ->
                val text = response.body?.string().orEmpty()
                if (response.isSuccessful) ApiResult.Ok(parse(text)) else ApiResult.Failed(error(text))
            }
        } catch (_: SocketTimeoutException) {
            ApiResult.Failed(timedOut)
        } catch (_: IOException) {
            ApiResult.Failed(ApiError.UNREACHABLE)
        } catch (_: JSONException) {
            ApiResult.Failed(ApiError.SERVER)
        }
    }

    private fun error(body: String): ApiError {
        val code = try { JSONObject(body).optString("error") } catch (_: JSONException) { "" }
        return when (code) {
            "room_not_found" -> ApiError.ROOM_NOT_FOUND
            "duplicate_room_name" -> ApiError.DUPLICATE_ROOM
            "invalid_qr_payload" -> ApiError.INVALID_QR
            "invalid_manual_code" -> ApiError.INVALID_MANUAL
            "invalid_room_name", "invalid_device_name", "invalid_label_name" -> ApiError.INVALID_NAME
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
            else -> ApiError.SERVER
        }
    }
}
