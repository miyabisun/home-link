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

    @Test fun commissionsWithTheWifiAndReadsTheLedgerEntry() {
        respond("/api/commission", 201, """{"node_id":17,"registered":true,"device":{"id":9,"room_id":2,
            "room_name":"押入れ","name":"押入れ1","vendor":"Tapo","serial_number":"CCBABDE0C244","mac":"CCBABDE0C244",
            "min_kelvin":null,"created_at":"t"}}""")
        val wifi = WifiNetwork("home-2g", "pass word")
        assertEquals(ApiResult.Ok(Commissioned(true, "押入れ", "押入れ1")),
            api.commission(2, "MT:Y.K9042C00KA0648G00", "押入れ1", wifi))
        val body = JSONObject(requests.single().substringAfter("/api/commission "))
        assertEquals(2L, body.getLong("room_id"))
        assertEquals("MT:Y.K9042C00KA0648G00", body.getString("qr_payload"))
        assertEquals("押入れ1", body.getString("name"))
        assertEquals("home-2g", body.getString("wifi_ssid"))
        assertEquals("pass word", body.getString("wifi_password"))
        assertEquals("wifi", body.getString("network"))

        // A device the ledger already holds keeps its own room and name.
        respond("/api/commission", 200, """{"node_id":17,"registered":false,"device":{"id":3,"room_id":1,
            "room_name":"寝室","name":"読書灯","created_at":"t"}}""")
        assertEquals(ApiResult.Ok(Commissioned(false, "寝室", "読書灯")), api.commission(2, "34970112332", "", wifi))
        assertEquals("34970112332", JSONObject(requests.last().substringAfter("/api/commission ")).getString("manual_code"))
    }

    @Test fun commissionsOntoThreadWithoutWifi() {
        respond("/api/commission", 201, """{"node_id":21,"registered":true,"device":{"id":5,"room_id":2,
            "room_name":"寝室","name":"T2","created_at":"t"}}""")
        assertEquals(ApiResult.Ok(Commissioned(true, "寝室", "T2")), api.commission(2, "34970112332", "T2", null))
        val body = JSONObject(requests.single().substringAfter("/api/commission "))
        assertEquals("thread", body.getString("network"))
        assertFalse(body.has("wifi_ssid"))
        assertFalse(body.has("wifi_password"))
    }

    @Test fun readsWhetherTheThreadNetworkIsReady() {
        respond("/api/thread", 200, """{"ready":true}""")
        assertEquals(ApiResult.Ok(true), api.threadReady())
        assertEquals("GET /api/thread", requests.single())
        respond("/api/thread", 200, """{"ready":false}""")
        assertEquals(ApiResult.Ok(false), api.threadReady())
        respond("/api/thread", 502, """{"error":"matter_server_unreachable"}""")
        assertEquals(ApiResult.Failed(ApiError.MATTER_UNREACHABLE), api.threadReady())
    }

    @Test fun mapsCommissioningFailures() {
        val cases = listOf(
            Triple(422, "device_not_found", ApiError.DEVICE_NOT_FOUND),
            Triple(422, "wrong_code", ApiError.WRONG_CODE),
            Triple(422, "wifi_failed", ApiError.WIFI_FAILED),
            Triple(504, "commission_timeout", ApiError.COMMISSION_TIMEOUT),
            Triple(422, "commission_failed", ApiError.COMMISSION_FAILED),
            Triple(503, "bluetooth_unavailable", ApiError.BLUETOOTH_UNAVAILABLE),
            Triple(400, "invalid_wifi", ApiError.INVALID_WIFI),
            Triple(503, "thread_not_ready", ApiError.THREAD_NOT_READY),
            Triple(422, "thread_failed", ApiError.THREAD_FAILED),
            Triple(400, "invalid_qr_payload", ApiError.INVALID_QR),
            Triple(404, "room_not_found", ApiError.ROOM_NOT_FOUND),
            Triple(502, "matter_server_unreachable", ApiError.MATTER_UNREACHABLE),
        )
        for ((status, code, expected) in cases) {
            respond("/api/commission", status, """{"error":"$code","message":"m"}""")
            assertEquals("$status $code", ApiResult.Failed(expected),
                api.commission(1, "MT:x", "", WifiNetwork("s", "p")))
        }
    }

    @Test fun switchesLightsAndCountsEachResult() {
        respond("/api/lights/on", 200, """{"action":"on","switched":7,"no_response":2,"failed":1,"missing":2,
            "lights":[],"missing_devices":[{"id":3,"name":"台所","room_name":"居間"},{"id":4,"name":"","room_name":"寝室"}]}""")
        assertEquals(ApiResult.Ok(LightsResult(7, 2, 1, listOf("台所", "寝室の機器"))), api.switchLights(true))
        assertEquals("POST /api/lights/on", requests.single())
        respond("/api/lights/off", 200, """{"action":"off","switched":3,"no_response":0,"failed":0,"missing":0,"lights":[],"missing_devices":[]}""")
        assertEquals(ApiResult.Ok(LightsResult(3, 0, 0, emptyList())), api.switchLights(false))
    }

    @Test fun mapsLightFailures() {
        val cases = listOf(
            Triple(502, "matter_server_unreachable", ApiError.MATTER_UNREACHABLE),
            Triple(503, "matter_server_not_configured", ApiError.MATTER_NOT_CONFIGURED),
            Triple(500, "internal", ApiError.SERVER),
        )
        for ((status, code, expected) in cases) {
            respond("/api/lights/off", status, """{"error":"$code"}""")
            assertEquals("$status $code", ApiResult.Failed(expected), api.switchLights(false))
        }
    }

    @Test fun reportsAnUnreachableServer() {
        val port = ServerSocket(0).use { it.localPort }
        val offline = HttpHomeLinkApi("http://127.0.0.1:$port")
        assertEquals(ApiResult.Failed(ApiError.UNREACHABLE), offline.rooms())
        assertEquals(ApiResult.Failed(ApiError.UNREACHABLE), offline.register(1, "MT:x", ""))
        assertEquals(ApiResult.Failed(ApiError.UNREACHABLE), offline.switchLights(true))
        assertEquals(ApiResult.Failed(ApiError.UNREACHABLE), offline.commission(1, "MT:x", "", WifiNetwork("s", "p")))
    }
}
