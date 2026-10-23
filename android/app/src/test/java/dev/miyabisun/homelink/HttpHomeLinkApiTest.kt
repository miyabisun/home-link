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

    private val ledgerRow = """{"id":9,"name":"押入れ1","vendor":null,"serial_number":null,
        "mac":null,"vendor_id":65521,"product_id":32768,"state":"unconnected","product_name":null,
        "labels":[{"id":1,"name":"作業"},{"id":3,"name":"夜"}],"ignored":false,"created_at":"t"}"""
    private val ledgerDevice = Device(9, "押入れ1", setOf(1, 3), DeviceState.UNCONNECTED, null)

    @Test fun entersDevicesByTheirCodeAlone() {
        respond("/api/devices", 201, """{"registered":true,"device":$ledgerRow}""")
        assertEquals(ApiResult.Ok(ledgerDevice), api.addDevice("MT:Y.K9042C00KA0648G00"))
        assertEquals("""POST /api/devices {"qr_payload":"MT:Y.K9042C00KA0648G00"}""", requests.last())
        // The same device again comes back as it is.
        respond("/api/devices", 200, """{"registered":false,"device":$ledgerRow}""")
        assertEquals(ApiResult.Ok(ledgerDevice), api.addDevice("34970112332"))
        assertEquals("""POST /api/devices {"manual_code":"34970112332"}""", requests.last())
        respond("/api/devices", 400, """{"error":"invalid_qr_payload"}""")
        assertEquals(ApiResult.Failed(ApiError.INVALID_QR), api.addDevice("MT:x"))
    }

    @Test fun listsTheLedgerWithEachDevicesState() {
        val unknown = ledgerRow.replace("\"unconnected\"", "null")
            .replace("[{\"id\":1,\"name\":\"作業\"},{\"id\":3,\"name\":\"夜\"}]", "[]")
        respond("/api/devices", 200, "[$ledgerRow,${ledgerRow.replace("unconnected", "connected")},$unknown]")
        assertEquals(ApiResult.Ok(listOf(ledgerDevice, ledgerDevice.copy(state = DeviceState.CONNECTED),
            ledgerDevice.copy(labelIds = emptySet(), state = null))), api.devices())
        assertEquals("GET /api/devices", requests.last())

        // One device carries its code.
        respond("/api/devices/9", 200, ledgerRow.replace("\"ignored\"", "\"qr_payload\":\"MT:Y.K9042C00KA0648G00\",\"ignored\""))
        assertEquals(ApiResult.Ok(ledgerDevice.copy(code = "MT:Y.K9042C00KA0648G00")), api.device(9))
        respond("/api/devices/9", 200, ledgerRow.replace("\"ignored\"", "\"manual_code\":\"34970112332\",\"ignored\""))
        assertEquals(ApiResult.Ok(ledgerDevice.copy(code = "34970112332")), api.device(9))
        respond("/api/devices/9", 404, """{"error":"device_not_found"}""")
        assertEquals(ApiResult.Failed(ApiError.DEVICE_NOT_FOUND), api.device(9))
    }

    @Test fun readsAndTogglesTheAirconSwitch() {
        respond("/api/aircon", 200, """{"on":false,"target":25,"device":null,"reading":null,"sent":null,"runs":[]}""")
        assertEquals(ApiResult.Ok(false), api.aircon())
        assertEquals("GET /api/aircon", requests.last())
        respond("/api/aircon/toggle", 200, """{"on":true}""")
        assertEquals(ApiResult.Ok(true), api.toggleAircon())
        assertEquals("POST /api/aircon/toggle", requests.last())
        respond("/api/aircon/toggle", 500, """{"error":"internal"}""")
        assertEquals(ApiResult.Failed(ApiError.SERVER), api.toggleAircon())
    }

    @Test fun changesANameAndTheLabels() {
        respond("/api/devices/9", 200, ledgerRow)
        assertEquals(ApiResult.Ok(ledgerDevice), api.updateDevice(9, DeviceUpdate.Name("押入れ1")))
        assertEquals("""PATCH /api/devices/9 {"name":"押入れ1"}""", requests.last())
        api.updateDevice(9, DeviceUpdate.Labels(setOf(1, 3)))
        assertEquals("""PATCH /api/devices/9 {"label_ids":[1,3]}""", requests.last())
        for ((status, code, expected) in listOf(
                Triple(404, "label_not_found", ApiError.LABEL_NOT_FOUND), Triple(400, "invalid_device_name", ApiError.INVALID_NAME),
                Triple(400, "conflicting_labels", ApiError.SERVER))) {
            respond("/api/devices/9", status, """{"error":"$code"}""")
            assertEquals(code, ApiResult.Failed(expected), api.updateDevice(9, DeviceUpdate.Name("x")))
        }
    }

    @Test fun listsTheLabelsWithTheirTraitsAndSchedules() {
        respond("/api/labels", 200, """[{"id":1,"name":"ambient","auto":true,"override_level":null,"override_kelvin":null,"on":null,
            "schedule":[],"device_count":12,"boosted":false},{"id":3,"name":"razor","auto":false,"override_level":null,
            "override_kelvin":null,"on":false,"schedule":[{"time":"00:00","on":true},{"time":"05:00","on":false}],"device_count":1,
            "boosted":false},{"id":4,"name":"work","auto":false,"override_level":254,"override_kelvin":5000,"on":true,"schedule":[],
            "device_count":1,"boosted":true}]""")
        assertEquals(ApiResult.Ok(listOf(
            Label(1, "ambient", auto = true),
            Label(3, "razor", on = false, schedule = listOf(ScheduleEntry(0, true), ScheduleEntry(300, false))),
            Label(4, "work", override = Override(254, 5000), on = true),
        )), api.labels())
    }

    @Test fun switchesALabelAndCountsEachResult() {
        respond("/api/labels/2/toggle", 200, """{"label":"game","action":"release","on":false,"boosted":false,"switched":2,
            "no_response":1,"failed":0,"missing":1,"devices":[],"missing_devices":[{"id":3,"name":""}]}""")
        assertEquals(ApiResult.Ok(LabelResult("release", false, SwitchResult(2, 1, 0, listOf("機器")))),
            api.switchLabel(2, LabelOp.TOGGLE))
        respond("/api/labels/2/on", 200, """{"label":"game","action":"on","on":true,"boosted":true,"switched":3,"no_response":0,
            "failed":0,"missing":0,"devices":[],"missing_devices":[]}""")
        assertEquals(ApiResult.Ok(LabelResult("on", true, SwitchResult(3, 0, 0, emptyList()))), api.switchLabel(2, LabelOp.ON))
        respond("/api/labels/2/off", 422, """{"error":"label_has_no_devices"}""")
        assertEquals(ApiResult.Failed(ApiError.LABEL_HAS_NO_DEVICES), api.switchLabel(2, LabelOp.OFF))
        assertEquals(listOf("POST /api/labels/2/toggle", "POST /api/labels/2/on", "POST /api/labels/2/off"), requests)
    }

    @Test fun connectsALedgerRowWithoutABody() {
        respond("/api/devices/9/commission", 200, """{"node_id":17,"device":${ledgerRow.replace("unconnected", "connected")}}""")
        assertEquals(ApiResult.Ok(ledgerDevice.copy(state = DeviceState.CONNECTED)), api.commission(9))
        // home-link hands over the Wi-Fi it holds for devices; the phone sends none.
        assertEquals("POST /api/devices/9/commission", requests.last())
    }

    @Test fun movesADeviceOntoTheWifiForDevicesWithoutABody() {
        respond("/api/devices/9/wifi", 200,
            """{"device":${ledgerRow.replace("unconnected", "connected").replace("\"ignored\"", "\"wifi_ssid\":\"iot-2g\",\"ignored\"")}}""")
        assertEquals(ApiResult.Ok(ledgerDevice.copy(state = DeviceState.CONNECTED, wifiSsid = "iot-2g")), api.moveWifi(9))
        // home-link hands over the Wi-Fi it holds for devices; the phone sends none.
        assertEquals("POST /api/devices/9/wifi", requests.last())
        for ((status, code, expected) in listOf(
            Triple(422, "wifi_reverted", ApiError.WIFI_REVERTED),
            Triple(504, "wifi_reverting", ApiError.WIFI_REVERTING),
            Triple(409, "not_on_wifi", ApiError.NOT_ON_WIFI),
            Triple(409, "wifi_not_set", ApiError.WIFI_NOT_SET),
        )) {
            respond("/api/devices/9/wifi", status, """{"error":"$code","message":"m"}""")
            assertEquals("$status $code", ApiResult.Failed(expected), api.moveWifi(9))
        }
    }

    @Test fun savesListsChoosesAndDeletesTheServersWifi() {
        respond("/api/wifi", 200, """[{"id":1,"ssid":"home-2g","for_devices":false},{"id":4,"ssid":"iot-2g","for_devices":true}]""")
        assertEquals(ApiResult.Ok(listOf(SavedNetwork(1, "home-2g", false), SavedNetwork(4, "iot-2g", true))), api.wifiNetworks())
        assertEquals("GET /api/wifi", requests.last())

        respond("/api/wifi", 201, """{"id":5,"ssid":"離れ \"2.4\"","for_devices":false}""")
        assertEquals(ApiResult.Ok(SavedNetwork(5, "離れ \"2.4\"", false)), api.saveWifi(WifiNetwork("離れ \"2.4\"", "pass word")))
        val body = JSONObject(requests.last().substringAfter("POST /api/wifi "))
        assertEquals("離れ \"2.4\"", body.getString("ssid"))
        assertEquals("pass word", body.getString("password"))
        respond("/api/wifi", 400, """{"error":"invalid_wifi"}""")
        assertEquals(ApiResult.Failed(ApiError.INVALID_WIFI), api.saveWifi(WifiNetwork("s", "p")))

        respond("/api/wifi/devices", 200, """{"id":1,"ssid":"home-2g","for_devices":true}""")
        assertEquals(ApiResult.Ok(SavedNetwork(1, "home-2g", true)), api.chooseWifi(1))
        assertEquals(1L, JSONObject(requests.last().substringAfter("PUT /api/wifi/devices ")).getLong("id"))
        respond("/api/wifi/devices", 404, """{"error":"wifi_not_found"}""")
        assertEquals(ApiResult.Failed(ApiError.WIFI_NOT_FOUND), api.chooseWifi(9))

        respond("/api/wifi/4", 204, "")
        assertEquals(ApiResult.Ok(Unit), api.deleteWifi(4))
        assertEquals("DELETE /api/wifi/4", requests.last())
    }

    @Test fun mapsCommissioningFailures() {
        val cases = listOf(
            Triple(422, "device_not_found", ApiError.DEVICE_NOT_FOUND),
            Triple(422, "wrong_code", ApiError.WRONG_CODE),
            Triple(422, "wifi_failed", ApiError.WIFI_FAILED),
            Triple(504, "commission_timeout", ApiError.COMMISSION_TIMEOUT),
            Triple(422, "commission_failed", ApiError.COMMISSION_FAILED),
            Triple(503, "bluetooth_unavailable", ApiError.BLUETOOTH_UNAVAILABLE),
            Triple(422, "thread_failed", ApiError.THREAD_FAILED),
            Triple(404, "device_not_found", ApiError.DEVICE_NOT_FOUND),
            Triple(409, "no_setup_code", ApiError.SERVER),
            Triple(502, "matter_server_unreachable", ApiError.MATTER_UNREACHABLE),
        )
        for ((status, code, expected) in cases) {
            respond("/api/devices/1/commission", status, """{"error":"$code","message":"m"}""")
            assertEquals("$status $code", ApiResult.Failed(expected), api.commission(1))
        }
    }

    @Test fun readsAPlugsKindAndOnOff() {
        val plug = ledgerRow.replace("\"unconnected\"", "\"connected\"").replace("\"product_name\":null",
            "\"product_name\":\"Mini Plug\",\"kind\":\"plug\",\"on\":true")
        respond("/api/devices/9", 200, plug)
        assertEquals(ApiResult.Ok(ledgerDevice.copy(state = DeviceState.CONNECTED, kind = DeviceKind.PLUG, on = true,
            productName = "Mini Plug")), api.device(9))
        respond("/api/devices/9", 200, ledgerRow.replace("\"product_name\":null", "\"product_name\":null,\"kind\":\"light\",\"on\":null"))
        assertEquals(ApiResult.Ok(ledgerDevice.copy(kind = DeviceKind.LIGHT)), api.device(9))
    }

    @Test fun switchesOneDeviceAndReadsItsAnswer() {
        respond("/api/devices/9/on", 200, """{"state":"on"}""")
        assertEquals(ApiResult.Ok(true), api.switchDevice(9, true))
        assertEquals("POST /api/devices/9/on", requests.last())
        respond("/api/devices/9/off", 200, """{"state":"off"}""")
        assertEquals(ApiResult.Ok(false), api.switchDevice(9, false))
        assertEquals("POST /api/devices/9/off", requests.last())
        respond("/api/devices/9/off", 200, """{"state":"no_response"}""")
        assertEquals(ApiResult.Ok(null), api.switchDevice(9, false))
        for ((status, code) in listOf(409 to "not_switchable", 409 to "not_connected", 502 to "switch_failed")) {
            respond("/api/devices/9/off", status, """{"error":"$code"}""")
            assertEquals(code, ApiResult.Failed(ApiError.SWITCH_FAILED), api.switchDevice(9, false))
        }
    }

    @Test fun reportsAnUnreachableServer() {
        val port = ServerSocket(0).use { it.localPort }
        val offline = HttpHomeLinkApi("http://127.0.0.1:$port")
        assertEquals(ApiResult.Failed(ApiError.UNREACHABLE), offline.devices())
        assertEquals(ApiResult.Failed(ApiError.UNREACHABLE), offline.switchLabel(1, LabelOp.ON))
        assertEquals(ApiResult.Failed(ApiError.UNREACHABLE), offline.commission(1))
    }

    @Test fun sendsDoorCommandsAndMapsTheirFailures() {
        for (command in DoorCommand.entries) {
            val path = "/api/door/${command.path}"
            requests.clear()
            respond(path, 200, """{"status":"accepted"}""")
            assertEquals(ApiResult.Ok(Unit), api.door(command))
            assertEquals("POST $path", requests.single())
            val cases = listOf(
                Triple(503, "sesame_not_configured", ApiError.SESAME_NOT_CONFIGURED),
                Triple(502, "sesame_rejected", ApiError.SESAME_REJECTED),
                Triple(502, "sesame_unreachable", ApiError.SESAME_UNREACHABLE),
                Triple(502, "", ApiError.SERVER),
            )
            for ((status, code, expected) in cases) {
                respond(path, status, if (code.isEmpty()) "<html>bad gateway</html>" else """{"error":"$code"}""")
                assertEquals("$command $status $code", ApiResult.Failed(expected), api.door(command))
            }
        }
    }

    @Test fun aDoorCommandWithoutAnAnswerInTimeIsATimeout() {
        // Accepts the connection (the backlog does) but never answers.
        ServerSocket(0).use { silent ->
            val started = System.nanoTime()
            assertEquals(ApiResult.Failed(ApiError.TIMEOUT),
                HttpHomeLinkApi("http://127.0.0.1:${silent.localPort}").door(DoorCommand.LOCK))
            assert(System.nanoTime() - started >= 15_000_000_000) { "gave up before 15 s" }
        }
    }

    @Test fun readsTheBaseUrlForEveryRequestAndSendsNothingWithoutOne() {
        respond("/api/devices", 200, "[]")
        var base: String? = null
        val configured = HttpHomeLinkApi { base }
        assertEquals(ApiResult.Failed(ApiError.NOT_CONFIGURED), configured.devices())
        assertEquals(ApiResult.Failed(ApiError.NOT_CONFIGURED), configured.door(DoorCommand.UNLOCK))
        assertEquals(emptyList<String>(), requests)
        base = "http://127.0.0.1:${server.address.port}"
        assertEquals(ApiResult.Ok(emptyList<Device>()), configured.devices())
        assertEquals(listOf("GET /api/devices"), requests)
    }
}
