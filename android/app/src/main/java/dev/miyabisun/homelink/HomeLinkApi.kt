package dev.miyabisun.homelink

import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL

data class Room(val id: Long, val name: String)

enum class ApiError { UNREACHABLE, DUPLICATE_QR, ROOM_NOT_FOUND, DUPLICATE_ROOM, INVALID_QR, INVALID_NAME, SERVER }

sealed interface ApiResult<out T> {
    data class Ok<T>(val value: T) : ApiResult<T>
    data class Failed(val error: ApiError) : ApiResult<Nothing>
}

/** The home-link ledger API. Calls block; run them off the main thread. */
interface HomeLinkApi {
    fun rooms(): ApiResult<List<Room>>
    fun createRoom(name: String): ApiResult<Room>
    fun register(roomId: Long, payload: String, name: String): ApiResult<Unit>
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
        val body = JSONObject().put("room_id", roomId).put("qr_payload", payload).put("name", name)
        return call("POST", "/api/devices", body) { }
    }

    private fun room(json: JSONObject) = Room(json.getLong("id"), json.getString("name"))

    private fun <T> call(method: String, path: String, body: JSONObject?, parse: (String) -> T): ApiResult<T> {
        val connection = try {
            (URL(base + path).openConnection() as HttpURLConnection).apply {
                requestMethod = method
                connectTimeout = 5_000
                readTimeout = 10_000
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
            "invalid_room_name", "invalid_device_name" -> ApiError.INVALID_NAME
            else -> ApiError.SERVER
        }
    }
}
