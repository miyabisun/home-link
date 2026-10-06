package dev.miyabisun.homelink

import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL

data class Room(val id: Long, val name: String)

enum class ApiError { UNREACHABLE, DUPLICATE_QR, ROOM_NOT_FOUND, DUPLICATE_ROOM, INVALID_QR, INVALID_MANUAL, INVALID_NAME,
    MATTER_UNREACHABLE, MATTER_NOT_CONFIGURED, SERVER,
    DEVICE_NOT_FOUND, WRONG_CODE, WIFI_FAILED, COMMISSION_TIMEOUT, COMMISSION_FAILED, BLUETOOTH_UNAVAILABLE, INVALID_WIFI }

/** The Wi-Fi network handed to a new device. */
data class WifiNetwork(val ssid: String, val password: String)

/** A commissioned device's ledger entry: new, or the one the ledger already held. */
data class Commissioned(val registered: Boolean, val room: String, val name: String)

sealed interface ApiResult<out T> {
    data class Ok<T>(val value: T) : ApiResult<T>
    data class Failed(val error: ApiError) : ApiResult<Nothing>
}

/** The home-link ledger API. Calls block; run them off the main thread. */
interface HomeLinkApi {
    fun rooms(): ApiResult<List<Room>>
    fun createRoom(name: String): ApiResult<Room>
    /** `payload` is a QR payload (`MT:`) or the digits of a manual pairing code. */
    fun register(roomId: Long, payload: String, name: String): ApiResult<Unit>
    /** Switches every light matterjs-server serves on or off. */
    fun switchLights(on: Boolean): ApiResult<LightsResult>
    /**
     * Has matterjs-server commission the device behind `payload` over the BLE proxy
     * this phone holds open, onto `wifi`, and records it. Takes up to minutes.
     */
    fun commission(roomId: Long, payload: String, name: String, wifi: WifiNetwork): ApiResult<Commissioned>
}

class HttpHomeLinkApi(baseUrl: String) : HomeLinkApi {
    private val base = baseUrl.trimEnd('/')

    override fun rooms(): ApiResult<List<Room>> = call("GET", "/api/rooms", null) { body ->
        val array = JSONArray(body)
        (0 until array.length()).map { room(array.getJSONObject(it)) }
    }

    override fun createRoom(name: String): ApiResult<Room> =
        call("POST", "/api/rooms", JSONObject().put("name", name)) { room(JSONObject(it)) }

    override fun register(roomId: Long, payload: String, name: String): ApiResult<Unit> {
        return call("POST", "/api/devices", device(roomId, payload, name)) { }
    }

    override fun commission(roomId: Long, payload: String, name: String, wifi: WifiNetwork): ApiResult<Commissioned> {
        val body = device(roomId, payload, name).put("wifi_ssid", wifi.ssid).put("wifi_password", wifi.password)
        // matterjs-server may take minutes: Bluetooth discovery, PASE, Wi-Fi join and CASE.
        return call("POST", "/api/commission", body, readTimeout = 360_000) {
            val json = JSONObject(it)
            val device = json.getJSONObject("device")
            Commissioned(json.getBoolean("registered"), device.getString("room_name"), device.getString("name"))
        }
    }

    private fun device(roomId: Long, payload: String, name: String): JSONObject {
        val field = if (payload.startsWith("MT:")) "qr_payload" else "manual_code"
        return JSONObject().put("room_id", roomId).put(field, payload).put("name", name)
    }

    override fun switchLights(on: Boolean): ApiResult<LightsResult> =
        call("POST", if (on) "/api/lights/on" else "/api/lights/off", null) { body ->
            val json = JSONObject(body)
            val missing = json.getJSONArray("missing_devices")
            LightsResult(
                json.getInt("switched"), json.getInt("no_response"), json.getInt("failed"),
                (0 until missing.length()).map { index ->
                    val device = missing.getJSONObject(index)
                    device.getString("name").ifEmpty { device.getString("room_name") + "の機器" }
                },
            )
        }

    private fun room(json: JSONObject) = Room(json.getLong("id"), json.getString("name"))

    private fun <T> call(method: String, path: String, body: JSONObject?, readTimeout: Int = 20_000,
                         parse: (String) -> T): ApiResult<T> {
        val connection = try {
            (URL(base + path).openConnection() as HttpURLConnection).apply {
                requestMethod = method
                connectTimeout = 5_000
                // Switching lights waits up to 10 s for every light's answer.
                this.readTimeout = readTimeout
                setRequestProperty("accept", "application/json")
                if (body != null) {
                    doOutput = true
                    setRequestProperty("content-type", "application/json")
                    outputStream.use { it.write(body.toString().toByteArray()) }
                }
            }
        } catch (_: IOException) {
            return ApiResult.Failed(ApiError.UNREACHABLE)
        }
        return try {
            val status = connection.responseCode
            val stream = if (status < 400) connection.inputStream else connection.errorStream
            val text = stream?.use { it.readBytes().decodeToString() }.orEmpty()
            if (status in 200..299) ApiResult.Ok(parse(text)) else ApiResult.Failed(error(text))
        } catch (_: IOException) {
            ApiResult.Failed(ApiError.UNREACHABLE)
        } catch (_: JSONException) {
            ApiResult.Failed(ApiError.SERVER)
        } finally {
            connection.disconnect()
        }
    }

    private fun error(body: String): ApiError {
        val code = try { JSONObject(body).optString("error") } catch (_: JSONException) { "" }
        return when (code) {
            "duplicate_qr_payload" -> ApiError.DUPLICATE_QR
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
            else -> ApiError.SERVER
        }
    }
}
