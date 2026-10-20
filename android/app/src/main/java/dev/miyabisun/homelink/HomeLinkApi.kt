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
    THREAD_FAILED, WIFI_NOT_SET, WIFI_NOT_FOUND, LABEL_NOT_FOUND, LABEL_HAS_NO_LIGHTS,
    SESAME_NOT_CONFIGURED, SESAME_REJECTED, SESAME_UNREACHABLE }

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

/** A ledger device; [code] is its QR payload or manual code, read only for one device at a time. */
data class Device(val id: Long, val name: String, val roomId: Long?, val labelIds: Set<Long>, val state: DeviceState?,
                  val code: String?)

data class Label(val id: Long, val name: String)

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
    /** Switches every light matterjs-server serves on or off. */
    fun switchLights(on: Boolean): ApiResult<LightsResult>
    /** Boosts the lights of the label named `label` to the day's values, or releases them if boosted. */
    fun toggleBoost(label: String): ApiResult<BoostResult>
    /** The names of the labels whose lights are boosted now. */
    fun boostedLabels(): ApiResult<Set<String>>
    fun devices(): ApiResult<List<Device>>
    fun device(id: Long): ApiResult<Device>
    /** Enters the device behind `payload` (a QR payload, `MT:`, or a manual code's digits), or returns the one entered. */
    fun addDevice(payload: String): ApiResult<Device>
    fun updateDevice(id: Long, update: DeviceUpdate): ApiResult<Device>
    fun labels(): ApiResult<List<Label>>
    /**
     * Has matterjs-server commission ledger device `id` by its code over the BLE proxy this phone holds open;
     * home-link hands over its Wi-Fi for devices, and matterjs-server picks Wi-Fi or Thread. Takes up to minutes.
     */
    fun commission(id: Long): ApiResult<Device>
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

    override fun labels(): ApiResult<List<Label>> = call("GET", "/api/labels", null) { body ->
        val array = JSONArray(body)
        (0 until array.length()).map { array.getJSONObject(it).let { label -> Label(label.getLong("id"), label.getString("name")) } }
    }

    // matterjs-server may take minutes: Bluetooth discovery, PASE, the network join and CASE.
    override fun commission(id: Long): ApiResult<Device> =
        call("POST", "/api/devices/$id/commission", null, readTimeout = 360_000) { device(JSONObject(it).getJSONObject("device")) }

    private fun device(json: JSONObject): Device {
        val labels = json.getJSONArray("labels")
        return Device(
            json.getLong("id"), json.getString("name"), json.optLong("room_id").takeUnless { json.isNull("room_id") },
            (0 until labels.length()).map { labels.getJSONObject(it).getLong("id") }.toSet(),
            DeviceState.of(json.optString("state").takeUnless { json.isNull("state") }),
            json.optString("qr_payload").ifEmpty { json.optString("manual_code") }.ifEmpty { null },
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

    override fun switchLights(on: Boolean): ApiResult<LightsResult> =
        call("POST", if (on) "/api/lights/on" else "/api/lights/off", null) { lights(JSONObject(it)) }

    override fun toggleBoost(label: String): ApiResult<BoostResult> =
        call("POST", "/api/lights/boost", JSONObject().put("label", label)) { body ->
            val json = JSONObject(body)
            BoostResult(json.getString("action") == "boost", lights(json))
        }

    // A short wait: the widget reads this within the time a broadcast receiver may run.
    override fun boostedLabels(): ApiResult<Set<String>> = call("GET", "/api/lights/schedule", null, readTimeout = 3_000) { body ->
        val labels = JSONObject(body).getJSONArray("labels")
        (0 until labels.length()).map { labels.getJSONObject(it) }.filter { it.getBoolean("boosted") }
            .map { it.getString("name") }.toSet()
    }

    private fun lights(json: JSONObject): LightsResult {
        val missing = json.getJSONArray("missing_devices")
        return LightsResult(
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
            // Switching lights waits up to 10 s for every light's answer.
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
            "invalid_room_name", "invalid_device_name" -> ApiError.INVALID_NAME
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
            "label_has_no_lights" -> ApiError.LABEL_HAS_NO_LIGHTS
            "sesame_not_configured" -> ApiError.SESAME_NOT_CONFIGURED
            "sesame_rejected" -> ApiError.SESAME_REJECTED
            "sesame_unreachable" -> ApiError.SESAME_UNREACHABLE
            else -> ApiError.SERVER
        }
    }
}
