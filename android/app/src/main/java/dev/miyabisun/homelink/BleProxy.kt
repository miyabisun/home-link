package dev.miyabisun.homelink

import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject
import java.util.Base64
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/** A BLE failure under the proxy protocol's error code (`device_not_found`, `write_failed`, …). */
class BleException(val code: String, message: String) : Exception(message)

data class BleConnection(val handle: Int, val mtu: Int)
data class BleCharacteristic(val uuid: String, val properties: List<String>)
data class Advertisement(
    val address: String,
    val name: String?,
    val rssi: Int,
    val connectable: Boolean,
    val serviceData: Map<String, ByteArray>,
    val serviceUuids: List<String>,
)

/** What the BLE hardware reports on its own. */
interface BleEvents {
    fun discovered(advertisement: Advertisement)
    fun notification(handle: Int, uuid: String, value: ByteArray)
    /** A peripheral dropped the connection without a `disconnect`. */
    fun disconnected(handle: Int, reason: String)
    /** Scanning ended without a `stop_scan`. */
    fun scanStopped(reason: String)
}

/**
 * The phone's BLE as the proxy drives it. Every call blocks until the operation
 * completes and throws [BleException] when it fails; the proxy issues one at a time.
 */
interface Ble {
    fun listen(events: BleEvents)
    fun startScan(serviceUuids: List<String>)
    fun stopScan()
    fun connect(address: String, timeoutMs: Long): BleConnection
    fun disconnect(handle: Int)
    fun services(handle: Int): List<String>
    fun characteristics(handle: Int, service: String): List<BleCharacteristic>
    fun read(handle: Int, uuid: String): ByteArray
    fun write(handle: Int, uuid: String, value: ByteArray, response: Boolean)
    fun subscribe(handle: Int, uuid: String)
    fun unsubscribe(handle: Int, uuid: String)
    fun requestMtu(handle: Int, mtu: Int): Int
    /** Stops scanning and disconnects every peripheral. */
    fun close()
}

enum class ProxyOpen { OK, UNREACHABLE, UNSUPPORTED_VERSION }

/**
 * A client of matterjs-server's BLE Proxy WebSocket Protocol v1: it greets the
 * server, runs the BLE commands the server sends against [ble] in order, and
 * forwards advertisements, notifications (binary for the BTP characteristic)
 * and losses. Matter itself (BTP, PASE, CASE) stays on the server.
 * [onCommand] hears each command name as it starts, to show progress.
 * Pings are answered by OkHttp.
 */
class BleProxy(
    private val url: String,
    private val ble: Ble,
    private val client: OkHttpClient,
    private val onCommand: (String) -> Unit,
) {
    private val worker = Executors.newSingleThreadExecutor()
    private val greeted = CountDownLatch(1)
    @Volatile private var hello: ProxyOpen = ProxyOpen.UNREACHABLE
    @Volatile private var socket: WebSocket? = null
    /** Per connection handle: the target of `WRITE_DATA` and the source of binary notifications. */
    private val writeTarget = ConcurrentHashMap<Int, String>()
    private val notifySource = ConcurrentHashMap<Int, String>()

    /** Connects and completes the handshake within [timeoutMs]. */
    fun open(timeoutMs: Long): ProxyOpen {
        ble.listen(Events())
        socket = client.newWebSocket(Request.Builder().url(url).build(), Listener())
        if (!greeted.await(timeoutMs, TimeUnit.MILLISECONDS) || hello != ProxyOpen.OK) {
            close()
            return if (hello == ProxyOpen.UNSUPPORTED_VERSION) ProxyOpen.UNSUPPORTED_VERSION else ProxyOpen.UNREACHABLE
        }
        return ProxyOpen.OK
    }

    /** Closes the WebSocket and releases every BLE connection. */
    fun close() {
        socket?.close(1000, null)
        worker.execute { ble.close() }
        worker.shutdown()
    }

    private inner class Listener : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            webSocket.send(JSONObject().put("type", "hello").put("version", 1).toString())
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            val message = try { JSONObject(text) } catch (_: JSONException) { return }
            if (message.optString("type") == "hello_response") {
                hello = if (message.has("error")) ProxyOpen.UNSUPPORTED_VERSION else ProxyOpen.OK
                greeted.countDown()
                return
            }
            if (!message.has("command")) return
            onCommand(message.optString("command"))
            run { webSocket.send(respond(message).toString()) }
        }

        override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
            if (bytes.size < 3 || bytes[0] != WRITE_DATA) return
            val handle = ((bytes[1].toInt() and 0xFF) shl 8) or (bytes[2].toInt() and 0xFF)
            val payload = bytes.substring(3).toByteArray()
            // Matter BTP writes to C1 always use an acknowledged write.
            run { writeTarget[handle]?.let { ble.write(handle, it, payload, true) } }
        }

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            webSocket.close(1000, null)
            ended()
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            greeted.countDown()
            ended()
        }

        private fun ended() {
            if (!worker.isShutdown) worker.execute { ble.close() }
        }
    }

    /** Runs BLE work on the single worker so operations never overlap. */
    private fun run(work: () -> Unit) {
        if (worker.isShutdown) return
        worker.execute {
            try { work() } catch (_: BleException) { /* a binary write has no reply to carry it */ }
        }
    }

    private fun respond(message: JSONObject): JSONObject {
        val id = message.optInt("id")
        val args = message.optJSONObject("args") ?: JSONObject()
        return try {
            JSONObject().put("id", id).put("success", true).put("result", execute(message.optString("command"), args))
        } catch (e: BleException) {
            failure(id, e.code, e.message.orEmpty())
        } catch (e: JSONException) {
            failure(id, "internal_error", e.message.orEmpty())
        } catch (e: RuntimeException) {
            failure(id, "internal_error", e.message ?: e.javaClass.simpleName)
        }
    }

    private fun failure(id: Int, code: String, text: String) =
        JSONObject().put("id", id).put("success", false).put("error", code).put("message", text)

    private fun execute(command: String, args: JSONObject): JSONObject {
        val result = JSONObject()
        val handle by lazy { args.getInt("connection_handle") }
        when (command) {
            "start_scan" -> ble.startScan(args.optJSONArray("service_uuids").strings())
            "stop_scan" -> ble.stopScan()
            "connect" -> ble.connect(args.getString("address"), args.optLong("timeout", 30_000)).let {
                result.put("connection_handle", it.handle).put("mtu", it.mtu)
            }
            "disconnect" -> {
                ble.disconnect(handle)
                writeTarget.remove(handle)
                notifySource.remove(handle)
            }
            "discover_services" -> result.put("services",
                JSONArray(ble.services(handle).map { JSONObject().put("uuid", it) }))
            "discover_characteristics" -> result.put("characteristics",
                JSONArray(ble.characteristics(handle, args.getString("service_uuid")).map {
                    JSONObject().put("uuid", it.uuid).put("properties", JSONArray(it.properties))
                }))
            "read_characteristic" ->
                result.put("value", base64(ble.read(handle, args.getString("characteristic_uuid"))))
            "write_characteristic" -> {
                val uuid = args.getString("characteristic_uuid")
                ble.write(handle, uuid, bytes(args.getString("value")), args.optBoolean("response", false))
                writeTarget[handle] = uuid
            }
            "subscribe_characteristic" -> {
                val uuid = args.getString("characteristic_uuid")
                ble.subscribe(handle, uuid)
                notifySource[handle] = uuid
            }
            "unsubscribe_characteristic" -> ble.unsubscribe(handle, args.getString("characteristic_uuid"))
            "write_and_subscribe" -> {
                val write = args.getString("write_uuid")
                val subscribe = args.getString("subscribe_uuid")
                ble.write(handle, write, bytes(args.getString("write_value")), args.optBoolean("write_response", false))
                writeTarget[handle] = write
                ble.subscribe(handle, subscribe)
                notifySource[handle] = subscribe
            }
            "request_mtu" -> result.put("mtu", ble.requestMtu(handle, args.getInt("mtu")))
            else -> throw BleException("internal_error", "Unknown command: $command")
        }
        return result
    }

    private inner class Events : BleEvents {
        override fun discovered(advertisement: Advertisement) {
            val data = JSONObject()
                .put("address", advertisement.address)
                .put("rssi", advertisement.rssi)
                .put("connectable", advertisement.connectable)
                .put("service_data", JSONObject(advertisement.serviceData.mapValues { base64(it.value) }))
                .put("service_uuids", JSONArray(advertisement.serviceUuids))
            advertisement.name?.let { data.put("name", it) }
            event("device_discovered", data)
        }

        override fun notification(handle: Int, uuid: String, value: ByteArray) {
            if (sameUuid(notifySource[handle], uuid)) {
                val frame = byteArrayOf(NOTIFICATION, (handle shr 8).toByte(), handle.toByte()) + value
                socket?.send(frame.toByteString())
            } else {
                event("characteristic_notification", JSONObject()
                    .put("connection_handle", handle).put("characteristic_uuid", uuid).put("value", base64(value)))
            }
        }

        override fun disconnected(handle: Int, reason: String) {
            writeTarget.remove(handle)
            notifySource.remove(handle)
            event("disconnected", JSONObject().put("connection_handle", handle).put("reason", reason))
        }

        override fun scanStopped(reason: String) = event("scan_stopped", JSONObject().put("reason", reason))

        private fun event(name: String, data: JSONObject) {
            socket?.send(JSONObject().put("event", name).put("data", data).toString())
        }
    }

    companion object {
        private const val WRITE_DATA: Byte = 0x01
        private const val NOTIFICATION: Byte = 0x02

        private fun base64(value: ByteArray) = Base64.getEncoder().encodeToString(value)
        private fun bytes(value: String) = try {
            Base64.getDecoder().decode(value)
        } catch (e: IllegalArgumentException) {
            throw BleException("internal_error", "Invalid base64: ${e.message}")
        }
        private fun JSONArray?.strings() = if (this == null) emptyList() else (0 until length()).map { getString(it) }
        private fun sameUuid(a: String?, b: String) = a != null && Uuids.parse(a) == Uuids.parse(b)
    }
}
