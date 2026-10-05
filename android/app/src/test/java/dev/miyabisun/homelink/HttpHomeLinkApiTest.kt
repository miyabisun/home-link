package dev.miyabisun.homelink

import com.sun.net.httpserver.HttpServer
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import java.net.InetSocketAddress
import java.net.ServerSocket

class HttpHomeLinkApiTest {
    private val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0).apply { start() }
    private val api = HttpHomeLinkApi("http://127.0.0.1:${server.address.port}/")
    private val requests = mutableListOf<String>()
    private val replies = mutableMapOf<String, Pair<Int, String>>()

    @After fun stop() = server.stop(0)

    private fun respond(path: String, status: Int, body: String) {
        if (replies.put(path, status to body) != null) return
        server.createContext(path) { exchange ->
            val sent = exchange.requestBody.readBytes().decodeToString()
            requests += "${exchange.requestMethod} ${exchange.requestURI} $sent".trim()
            val (status, body) = replies.getValue(path)
            val bytes = body.toByteArray()
            exchange.responseHeaders.add("content-type", "application/json")
            exchange.sendResponseHeaders(status, if (bytes.isEmpty()) -1 else bytes.size.toLong())
            exchange.responseBody.use { it.write(bytes) }
        }
    }

    @Test fun listsRooms() {
        respond("/api/rooms", 200, """[{"id":1,"name":"寝室","device_count":2},{"id":3,"name":"居間","device_count":0}]""")
        assertEquals(ApiResult.Ok(listOf(Room(1, "寝室"), Room(3, "居間"))), api.rooms())
    }

    @Test fun createsRoomsAndMapsDuplicates() {
        respond("/api/rooms", 201, """{"id":4,"name":"書斎","device_count":0}""")
        assertEquals(ApiResult.Ok(Room(4, "書斎")), api.createRoom("書斎"))
        assertEquals("書斎", JSONObject(requests.single().substringAfter("/api/rooms ")).getString("name"))
        respond("/api/rooms", 409, """{"error":"duplicate_room_name","message":"x"}""")
        assertEquals(ApiResult.Failed(ApiError.DUPLICATE_ROOM), api.createRoom("書斎"))
    }

    @Test fun registersDevicesWithTheEnteredFields() {
        respond("/api/devices", 201, """{"id":7,"room_id":1,"room_name":"寝室","name":"","created_at":"t"}""")
        assertEquals(ApiResult.Ok(Unit), api.register(1, "MT:Y.K9042C00KA0648G00", ""))
        val body = JSONObject(requests.single().substringAfter("/api/devices "))
        assertEquals(1L, body.getLong("room_id"))
        assertEquals("MT:Y.K9042C00KA0648G00", body.getString("qr_payload"))
        assertEquals("", body.getString("name"))
    }

    @Test fun sendsADigitCodeAsTheManualCode() {
        respond("/api/devices", 201, """{"id":8,"room_id":1,"room_name":"寝室","name":"","created_at":"t"}""")
        assertEquals(ApiResult.Ok(Unit), api.register(1, "34970112332", ""))
        val body = JSONObject(requests.single().substringAfter("/api/devices "))
        assertEquals("34970112332", body.getString("manual_code"))
        assertFalse(body.has("qr_payload"))
    }

    @Test fun mapsRegistrationFailures() {
        val cases = listOf(
            Triple(409, "duplicate_qr_payload", ApiError.DUPLICATE_QR),
            Triple(404, "room_not_found", ApiError.ROOM_NOT_FOUND),
            Triple(400, "invalid_qr_payload", ApiError.INVALID_QR),
            Triple(400, "invalid_manual_code", ApiError.INVALID_MANUAL),
            Triple(400, "invalid_device_name", ApiError.INVALID_NAME),
            Triple(500, "internal", ApiError.SERVER),
            Triple(502, "", ApiError.SERVER),
        )
        for ((status, code, expected) in cases) {
            respond("/api/devices", status, if (code.isEmpty()) "<html>bad gateway</html>" else """{"error":"$code"}""")
            assertEquals("$status $code", ApiResult.Failed(expected), api.register(1, "MT:x", "n"))
        }
    }

    @Test fun reportsAnUnreachableServer() {
        val port = ServerSocket(0).use { it.localPort }
        val offline = HttpHomeLinkApi("http://127.0.0.1:$port")
        assertEquals(ApiResult.Failed(ApiError.UNREACHABLE), offline.rooms())
        assertEquals(ApiResult.Failed(ApiError.UNREACHABLE), offline.register(1, "MT:x", ""))
    }
}
