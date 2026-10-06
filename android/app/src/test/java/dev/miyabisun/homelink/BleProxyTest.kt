package dev.miyabisun.homelink

import okhttp3.OkHttpClient
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import okio.ByteString
import okio.ByteString.Companion.toByteString
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Base64
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

class BleProxyTest {
    private val c1 = "18EE2EF5-263D-4559-959F-4F9C429F9D11"
    private val c2 = "18EE2EF5-263D-4559-959F-4F9C429F9D12"
    private val c3 = "18EE2EF5-263D-4559-959F-4F9C429F9D13"

    /** The matterjs-server side of `/ble`: what the client sent, and a socket to command it. */
    private class FakeServer(private val helloReply: String?) : WebSocketListener() {
        val texts = LinkedBlockingQueue<JSONObject>()
        val binaries = LinkedBlockingQueue<ByteString>()
        val closed = CountDownLatch(1)
        lateinit var socket: WebSocket
        private var id = 0

        override fun onOpen(webSocket: WebSocket, response: Response) { socket = webSocket }

        override fun onMessage(webSocket: WebSocket, text: String) {
            val message = JSONObject(text)
            if (message.optString("type") == "hello" && helloReply != null) webSocket.send(helloReply) else texts += message
        }

        override fun onMessage(webSocket: WebSocket, bytes: ByteString) { binaries += bytes }
        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) { webSocket.close(1000, null); closed.countDown() }
        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) { closed.countDown() }

        /** Sends a command and returns the client's response to it. */
        fun command(name: String, args: JSONObject? = null): JSONObject {
            val sent = JSONObject().put("id", ++id).put("command", name)
            if (args != null) sent.put("args", args)
            socket.send(sent.toString())
            val reply = next()
            assertEquals(id, reply.getInt("id"))
            return reply
        }

        fun next(): JSONObject = texts.poll(5, TimeUnit.SECONDS) ?: error("no message from the client")
        fun nextBinary(): ByteString = binaries.poll(5, TimeUnit.SECONDS) ?: error("no binary frame from the client")
    }

    /** Records every BLE operation and answers from canned values. */
    private class FakeBle : Ble {
        val calls = CopyOnWriteArrayList<String>()
        val written = CopyOnWriteArrayList<Pair<String, ByteArray>>()
        var failure: BleException? = null
        lateinit var events: BleEvents
        val closed = CountDownLatch(1)

        private fun record(call: String) {
            calls += call
            failure?.let { throw it }
        }

        override fun listen(events: BleEvents) { this.events = events }
        override fun startScan(serviceUuids: List<String>) = record("startScan $serviceUuids")
        override fun stopScan() = record("stopScan")
        override fun connect(address: String, timeoutMs: Long): BleConnection {
            record("connect $address $timeoutMs")
            return BleConnection(1, 23)
        }
        override fun disconnect(handle: Int) = record("disconnect $handle")
        override fun services(handle: Int): List<String> {
            record("services $handle")
            return listOf("FFF6")
        }
        override fun characteristics(handle: Int, service: String): List<BleCharacteristic> {
            record("characteristics $handle $service")
            return listOf(
                BleCharacteristic("18EE2EF5-263D-4559-959F-4F9C429F9D11", listOf("write")),
                BleCharacteristic("18EE2EF5-263D-4559-959F-4F9C429F9D12", listOf("indicate")),
                BleCharacteristic("18EE2EF5-263D-4559-959F-4F9C429F9D13", listOf("read")),
            )
        }
        override fun read(handle: Int, uuid: String): ByteArray {
            record("read $handle $uuid")
            return byteArrayOf(0x15, 0x30)
        }
        override fun write(handle: Int, uuid: String, value: ByteArray, response: Boolean) {
            record("write $handle $uuid $response")
            written += uuid to value
        }
        override fun subscribe(handle: Int, uuid: String) = record("subscribe $handle $uuid")
        override fun unsubscribe(handle: Int, uuid: String) = record("unsubscribe $handle $uuid")
        override fun requestMtu(handle: Int, mtu: Int): Int {
            record("requestMtu $handle $mtu")
            return 185
        }
        override fun close() {
            calls += "close"
            closed.countDown()
        }
    }

    private val mock = MockWebServer()
    private val ble = FakeBle()
    private val stages = CopyOnWriteArrayList<String>()

    @After fun stop() = mock.shutdown()

    private fun open(server: FakeServer): BleProxy {
        mock.enqueue(MockResponse().withWebSocketUpgrade(server))
        val proxy = BleProxy(mock.url("/ble").toString().replace("http", "ws"), ble, OkHttpClient()) { stages += it }
        assertEquals(ProxyOpen.OK, proxy.open(5_000))
        return proxy
    }

    private fun handshake() = FakeServer("""{"type":"hello_response","version":1}""")

    private fun args(vararg pairs: Pair<String, Any>) = JSONObject().apply { pairs.forEach { (k, v) -> put(k, v) } }
    private fun b64(vararg bytes: Int) = Base64.getEncoder().encodeToString(ByteArray(bytes.size) { bytes[it].toByte() })

    @Test fun greetsWithHelloAndRefusesAnUnsupportedVersion() {
        val server = handshake()
        val proxy = open(server)
        val hello = mock.takeRequest()
        assertEquals("/ble", hello.path)
        proxy.close()

        val refusing = FakeServer("""{"type":"hello_response","version":1,"error":"unsupported_version","message":"no"}""")
        mock.enqueue(MockResponse().withWebSocketUpgrade(refusing))
        val other = BleProxy(mock.url("/ble").toString().replace("http", "ws"), FakeBle(), OkHttpClient()) {}
        assertEquals(ProxyOpen.UNSUPPORTED_VERSION, other.open(5_000))

        val silent = FakeServer(null)
        mock.enqueue(MockResponse().withWebSocketUpgrade(silent))
        val waiting = BleProxy(mock.url("/ble").toString().replace("http", "ws"), FakeBle(), OkHttpClient()) {}
        assertEquals(ProxyOpen.UNREACHABLE, waiting.open(300))
        // The registration closes its link again after a failed open.
        waiting.close()

        val closed = MockWebServer().apply { start(); shutdown() }
        val nobody = BleProxy(closed.url("/ble").toString().replace("http", "ws"), FakeBle(), OkHttpClient()) {}
        assertEquals(ProxyOpen.UNREACHABLE, nobody.open(2_000))
    }

    @Test fun runsTheMatterCommissioningSequence() {
        val server = handshake()
        val proxy = open(server)
        val ok = { reply: JSONObject -> assertTrue(reply.toString(), reply.getBoolean("success")); reply.optJSONObject("result") }

        ok(server.command("start_scan", args("service_uuids" to JSONArray(listOf("fff6")))))
        ble.events.discovered(Advertisement("AA:BB:CC:DD:EE:FF", "MATTER-3840", -61, true,
            mapOf("FFF6" to byteArrayOf(0, 0, 15, -15, -1, 1, -128, 0)), listOf("FFF6")))
        val discovered = server.next()
        assertEquals("device_discovered", discovered.getString("event"))
        val data = discovered.getJSONObject("data")
        assertEquals("AA:BB:CC:DD:EE:FF", data.getString("address"))
        assertEquals("MATTER-3840", data.getString("name"))
        assertEquals(-61, data.getInt("rssi"))
        assertTrue(data.getBoolean("connectable"))
        assertEquals(b64(0, 0, 15, 0xF1, 0xFF, 1, 0x80, 0), data.getJSONObject("service_data").getString("FFF6"))
        assertEquals("FFF6", data.getJSONArray("service_uuids").getString(0))
        ok(server.command("stop_scan"))

        val connected = ok(server.command("connect", args("address" to "AA:BB:CC:DD:EE:FF", "timeout" to 10_000)))!!
        assertEquals(1, connected.getInt("connection_handle"))
        assertEquals(23, connected.getInt("mtu"))
        assertEquals(185, ok(server.command("request_mtu", args("connection_handle" to 1, "mtu" to 247)))!!.getInt("mtu"))
        val services = ok(server.command("discover_services", args("connection_handle" to 1)))!!.getJSONArray("services")
        assertEquals("FFF6", services.getJSONObject(0).getString("uuid"))
        val characteristics = ok(server.command("discover_characteristics",
            args("connection_handle" to 1, "service_uuid" to "fff6")))!!.getJSONArray("characteristics")
        assertEquals(c2, characteristics.getJSONObject(1).getString("uuid"))
        assertEquals("indicate", characteristics.getJSONObject(1).getJSONArray("properties").getString(0))
        assertEquals(b64(0x15, 0x30), ok(server.command("read_characteristic",
            args("connection_handle" to 1, "characteristic_uuid" to c3)))!!.getString("value"))

        // The BTP handshake: write C1, then subscribe C2, in that order.
        ok(server.command("write_and_subscribe", args("connection_handle" to 1, "write_uuid" to c1,
            "write_value" to b64(0x65, 0x6C), "write_response" to true, "subscribe_uuid" to c2)))
        assertEquals(listOf("write 1 $c1 true", "subscribe 1 $c2"), ble.calls.takeLast(2))
        assertArrayEquals(byteArrayOf(0x65, 0x6C), ble.written.last().second)

        // BTP frames: WRITE_DATA to the last written characteristic, with response.
        server.socket.send(byteArrayOf(0x01, 0x00, 0x01, 0x05, 0x06).toByteString())
        ble.events.notification(1, c2, byteArrayOf(0x07, 0x08))
        assertEquals(byteArrayOf(0x02, 0x00, 0x01, 0x07, 0x08).toByteString(), server.nextBinary())
        assertEquals("write 1 $c1 true", ble.calls.last())
        assertArrayEquals(byteArrayOf(0x05, 0x06), ble.written.last().second)

        ok(server.command("write_characteristic", args("connection_handle" to 1, "characteristic_uuid" to c3,
            "value" to b64(9))))
        assertEquals("write 1 $c3 false", ble.calls.last())
        ok(server.command("subscribe_characteristic", args("connection_handle" to 1, "characteristic_uuid" to "18ee2ef5263d4559959f4f9c429f9d13")))
        // Binary frames carry the latest subscribed characteristic, whatever its UUID form; others go as JSON.
        ble.events.notification(1, c3, byteArrayOf(0x01))
        assertEquals(byteArrayOf(0x02, 0x00, 0x01, 0x01).toByteString(), server.nextBinary())
        ble.events.notification(1, c2, byteArrayOf(0x64))
        val notification = server.next()
        assertEquals("characteristic_notification", notification.getString("event"))
        assertEquals(1, notification.getJSONObject("data").getInt("connection_handle"))
        assertEquals(c2, notification.getJSONObject("data").getString("characteristic_uuid"))
        assertEquals(b64(0x64), notification.getJSONObject("data").getString("value"))
        ok(server.command("unsubscribe_characteristic", args("connection_handle" to 1, "characteristic_uuid" to c3)))
        assertEquals("unsubscribe 1 $c3", ble.calls.last())

        ok(server.command("disconnect", args("connection_handle" to 1)))
        assertEquals(listOf("start_scan", "stop_scan", "connect", "request_mtu", "discover_services",
            "discover_characteristics", "read_characteristic", "write_and_subscribe", "write_characteristic",
            "subscribe_characteristic", "unsubscribe_characteristic", "disconnect"), stages)
        proxy.close()
        assertTrue(ble.closed.await(5, TimeUnit.SECONDS))
        assertTrue(server.closed.await(5, TimeUnit.SECONDS))
    }

    @Test fun reportsEachDeviceOnceAScanWhenDuplicatesAreNotAllowed() {
        val server = handshake()
        open(server)
        val a = Advertisement("AA:BB:CC:DD:EE:FF", null, -61, true, emptyMap(), listOf("FFF6"))
        val end = a.copy(address = "11:22:33:44:55:66")
        /** Starts a scan, advertises [a] twice, and returns the addresses reported up to [end]. */
        fun scan(args: JSONObject): List<String> {
            assertTrue(server.command("start_scan", args).getBoolean("success"))
            listOf(a, a, end).forEach { ble.events.discovered(it) }
            val reported = mutableListOf<String>()
            do reported += server.next().getJSONObject("data").getString("address") while (reported.last() != end.address)
            return reported.dropLast(1)
        }
        val fff6 = JSONArray(listOf("fff6"))

        assertEquals(listOf(a.address), scan(args("service_uuids" to fff6, "allow_duplicates" to false)))
        assertEquals(listOf(a.address), scan(args("service_uuids" to fff6, "allow_duplicates" to false)))
        assertEquals(listOf(a.address, a.address), scan(args("service_uuids" to fff6)))
        assertEquals(listOf(a.address, a.address), scan(args("service_uuids" to fff6, "allow_duplicates" to true)))
    }

    @Test fun reportsBleFailuresAndUnexpectedEvents() {
        val server = handshake()
        open(server)
        ble.failure = BleException("device_not_found", "not advertising")
        val failed = server.command("connect", args("address" to "AA:BB:CC:DD:EE:FF"))
        assertEquals(false, failed.getBoolean("success"))
        assertEquals("device_not_found", failed.getString("error"))
        assertEquals("not advertising", failed.getString("message"))
        assertEquals("connect AA:BB:CC:DD:EE:FF 30000", ble.calls.last())
        ble.failure = null

        val unknown = server.command("pair")
        assertEquals("internal_error", unknown.getString("error"))
        val missing = server.command("read_characteristic", args("connection_handle" to 1))
        assertEquals("internal_error", missing.getString("error"))

        ble.events.disconnected(1, "link lost")
        val lost = server.next()
        assertEquals("disconnected", lost.getString("event"))
        assertEquals(1, lost.getJSONObject("data").getInt("connection_handle"))
        assertEquals("link lost", lost.getJSONObject("data").getString("reason"))
        ble.events.scanStopped("adapter_off")
        val stopped = server.next()
        assertEquals("scan_stopped", stopped.getString("event"))
        assertEquals("adapter_off", stopped.getJSONObject("data").getString("reason"))
        // A short or unknown binary frame is dropped.
        server.socket.send(byteArrayOf(0x01, 0x00).toByteString())
        server.socket.send(byteArrayOf(0x09, 0x00, 0x01, 0x02).toByteString())
        assertEquals(false, server.command("pair").getBoolean("success"))
        assertNull(ble.written.lastOrNull())
    }

    @Test fun cleansUpBleWhenTheServerCloses() {
        val server = handshake()
        open(server)
        server.socket.close(1000, "done")
        assertTrue(ble.closed.await(5, TimeUnit.SECONDS))
    }
}
