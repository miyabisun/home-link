package dev.miyabisun.homelink

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothStatusCodes
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.os.ParcelUuid
import okhttp3.OkHttpClient
import java.util.UUID
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.ExecutionException
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException
import java.util.concurrent.atomic.AtomicInteger

/**
 * The runtime permissions the proxy needs on [sdk]. Android 17 also blocks LAN connections,
 * including to the BLE proxy, without ACCESS_LOCAL_NETWORK; Android 16 lacks it and would deny it.
 */
@SuppressLint("InlinedApi") // Only asked for when sdk is 37 or above.
fun bluetoothPermissions(sdk: Int): Array<String> {
    val bluetooth = arrayOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT)
    return if (sdk >= 37) bluetooth + Manifest.permission.ACCESS_LOCAL_NETWORK else bluetooth
}

/** The real proxy: [BleProxy] over the phone's BLE, to matterjs-server at [url]. */
class ProxyLink(private val context: Context, private val url: String) : BleLink {
    private var proxy: BleProxy? = null

    override fun open(onCommand: (String) -> Unit): ProxyOpen {
        val proxy = BleProxy(url, AndroidBle(context), OkHttpClient(), onCommand)
        this.proxy = proxy
        return proxy.open(10_000)
    }

    override fun close() {
        proxy?.close()
    }
}

/**
 * The phone's BLE for [BleProxy]. Android allows one GATT operation in flight,
 * and the proxy calls one at a time, so each call waits for its callback.
 * The activity holds the Bluetooth permissions before any call.
 */
@SuppressLint("MissingPermission")
class AndroidBle(private val context: Context) : Ble {
    private val adapter = context.getSystemService(BluetoothManager::class.java).adapter
    private var events: BleEvents? = null
    private val gatts = ConcurrentHashMap<Int, BluetoothGatt>()
    private val discovered = ConcurrentHashMap.newKeySet<Int>()
    private val closing = ConcurrentHashMap.newKeySet<Int>()
    private val nextHandle = AtomicInteger(1)
    private var scanning = false

    /** The GATT operation awaiting its callback: which callback and for which handle. */
    private class Pending(val kind: String, val handle: Int) {
        val result = CompletableFuture<Any?>()
    }
    @Volatile private var pending: Pending? = null

    override fun listen(events: BleEvents) {
        this.events = events
    }

    private val scan = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult) {
            val record = result.scanRecord
            events?.discovered(Advertisement(
                address = result.device.address,
                name = record?.deviceName,
                rssi = result.rssi,
                connectable = result.isConnectable,
                serviceData = record?.serviceData.orEmpty().mapKeys { Uuids.show(it.key.uuid) },
                serviceUuids = record?.serviceUuids.orEmpty().map { Uuids.show(it.uuid) },
            ))
        }

        override fun onScanFailed(errorCode: Int) {
            scanning = false
            events?.scanStopped("scan_failed_$errorCode")
        }
    }

    override fun startScan(serviceUuids: List<String>) {
        val scanner = adapter?.takeIf { it.isEnabled }?.bluetoothLeScanner
            ?: throw BleException("bluetooth_unavailable", "Bluetooth is off")
        if (scanning) scanner.stopScan(scan)
        // Matter advertises 0xFFF6 as service data, not always in the service UUID list; filters are OR'ed.
        val filters = serviceUuids.mapNotNull(Uuids::parse).flatMap {
            listOf(
                ScanFilter.Builder().setServiceUuid(ParcelUuid(it)).build(),
                ScanFilter.Builder().setServiceData(ParcelUuid(it), ByteArray(0)).build(),
            )
        }
        val settings = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
        scanner.startScan(filters, settings, scan)
        scanning = true
    }

    override fun stopScan() {
        if (!scanning) throw BleException("not_scanning", "No scan is active")
        adapter?.bluetoothLeScanner?.stopScan(scan)
        scanning = false
    }

    override fun connect(address: String, timeoutMs: Long): BleConnection {
        if (gatts.values.any { it.device.address.equals(address, ignoreCase = true) }) {
            throw BleException("already_connected", "Already connected to $address")
        }
        val device = try {
            adapter?.getRemoteDevice(address) ?: throw BleException("bluetooth_unavailable", "No Bluetooth")
        } catch (_: IllegalArgumentException) {
            throw BleException("device_not_found", "Invalid address $address")
        }
        val handle = nextHandle.getAndIncrement()
        val op = Pending("connect", handle)
        pending = op
        val gatt = device.connectGatt(context, false, Callback(handle), BluetoothDevice.TRANSPORT_LE)
            ?: throw BleException("connection_failed", "connectGatt returned null")
        gatts[handle] = gatt
        try {
            await(op, timeoutMs, "connection_failed")
        } catch (e: BleException) {
            gatts.remove(handle)
            gatt.close()
            throw e
        }
        // matterjs-server sizes BTP segments from this MTU and does not ask for a larger one itself.
        val mtu = try { requestMtu(handle, MATTER_MTU) } catch (_: BleException) { DEFAULT_MTU }
        return BleConnection(handle, mtu)
    }

    override fun disconnect(handle: Int) {
        val gatt = gatts[handle] ?: throw BleException("not_connected", "No connection $handle")
        closing += handle
        val op = Pending("disconnect", handle)
        pending = op
        gatt.disconnect()
        try { await(op, 5_000, "not_connected") } catch (_: BleException) { /* closed below either way */ }
        release(handle)
    }

    override fun services(handle: Int): List<String> = discover(handle).services.map { Uuids.show(it.uuid) }

    override fun characteristics(handle: Int, service: String): List<BleCharacteristic> {
        val gatt = discover(handle)
        val found = Uuids.parse(service)?.let(gatt::getService)
            ?: throw BleException("service_not_found", "No service $service")
        return found.characteristics.map { BleCharacteristic(Uuids.show(it.uuid), properties(it.properties)) }
    }

    override fun read(handle: Int, uuid: String): ByteArray {
        val (gatt, characteristic) = characteristic(handle, uuid)
        val op = Pending("read", handle)
        pending = op
        if (!gatt.readCharacteristic(characteristic)) throw BleException("read_failed", "Read refused")
        return await(op, OPERATION_TIMEOUT, "read_failed") as ByteArray
    }

    override fun write(handle: Int, uuid: String, value: ByteArray, response: Boolean) {
        val (gatt, characteristic) = characteristic(handle, uuid)
        val type = if (response) BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
            else BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
        val op = Pending("write", handle)
        pending = op
        if (gatt.writeCharacteristic(characteristic, value, type) != BluetoothStatusCodes.SUCCESS) {
            throw BleException("write_failed", "Write refused")
        }
        await(op, OPERATION_TIMEOUT, "write_failed")
    }

    override fun subscribe(handle: Int, uuid: String) {
        val (gatt, characteristic) = characteristic(handle, uuid)
        val value = when {
            characteristic.properties and BluetoothGattCharacteristic.PROPERTY_INDICATE != 0 ->
                BluetoothGattDescriptor.ENABLE_INDICATION_VALUE
            characteristic.properties and BluetoothGattCharacteristic.PROPERTY_NOTIFY != 0 ->
                BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
            else -> throw BleException("notify_not_supported", "$uuid neither notifies nor indicates")
        }
        configure(gatt, characteristic, true, value, "subscribe_failed")
    }

    override fun unsubscribe(handle: Int, uuid: String) {
        val (gatt, characteristic) = characteristic(handle, uuid)
        configure(gatt, characteristic, false, BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE, "not_subscribed")
    }

    override fun requestMtu(handle: Int, mtu: Int): Int {
        val gatt = gatts[handle] ?: throw BleException("not_connected", "No connection $handle")
        val op = Pending("mtu", handle)
        pending = op
        if (!gatt.requestMtu(mtu)) throw BleException("mtu_request_failed", "MTU request refused")
        return await(op, OPERATION_TIMEOUT, "mtu_request_failed") as Int
    }

    override fun close() {
        if (scanning) adapter?.bluetoothLeScanner?.stopScan(scan)
        scanning = false
        for (handle in gatts.keys.toList()) {
            closing += handle
            gatts[handle]?.disconnect()
            release(handle)
        }
    }

    private fun configure(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, enable: Boolean,
                          value: ByteArray, error: String) {
        if (!gatt.setCharacteristicNotification(characteristic, enable)) throw BleException(error, "Refused")
        val descriptor = characteristic.getDescriptor(CCCD) ?: throw BleException(error, "No CCCD")
        val op = Pending("descriptor", handleOf(gatt))
        pending = op
        if (gatt.writeDescriptor(descriptor, value) != BluetoothStatusCodes.SUCCESS) {
            throw BleException(error, "Descriptor write refused")
        }
        await(op, OPERATION_TIMEOUT, error)
    }

    private fun discover(handle: Int): BluetoothGatt {
        val gatt = gatts[handle] ?: throw BleException("not_connected", "No connection $handle")
        if (handle in discovered) return gatt
        val op = Pending("services", handle)
        pending = op
        if (!gatt.discoverServices()) throw BleException("discovery_failed", "Discovery refused")
        await(op, OPERATION_TIMEOUT, "discovery_failed")
        discovered += handle
        return gatt
    }

    private fun characteristic(handle: Int, uuid: String): Pair<BluetoothGatt, BluetoothGattCharacteristic> {
        val gatt = discover(handle)
        val id = Uuids.parse(uuid) ?: throw BleException("characteristic_not_found", "Bad UUID $uuid")
        val found = gatt.services.firstNotNullOfOrNull { it.getCharacteristic(id) }
            ?: throw BleException("characteristic_not_found", "No characteristic $uuid")
        return gatt to found
    }

    private fun handleOf(gatt: BluetoothGatt) = gatts.entries.first { it.value === gatt }.key

    private fun await(op: Pending, timeoutMs: Long, error: String): Any? = try {
        op.result.get(timeoutMs, TimeUnit.MILLISECONDS)
    } catch (_: TimeoutException) {
        throw BleException(if (op.kind == "connect") "timeout" else error, "${op.kind} timed out")
    } catch (e: ExecutionException) {
        throw (e.cause as? BleException) ?: BleException(error, e.cause?.message.orEmpty())
    } finally {
        if (pending === op) pending = null
    }

    /** Completes the pending operation when it is [kind] on [handle]. */
    private fun finish(kind: String, handle: Int, status: Int, value: Any?, error: String) {
        val op = pending?.takeIf { it.kind == kind && it.handle == handle } ?: return
        if (status == BluetoothGatt.GATT_SUCCESS) op.result.complete(value)
        else op.result.completeExceptionally(BleException(error, "GATT status $status"))
    }

    private fun release(handle: Int) {
        gatts.remove(handle)?.close()
        discovered -= handle
        closing -= handle
    }

    private inner class Callback(private val handle: Int) : BluetoothGattCallback() {
        override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
            when {
                newState == BluetoothProfile.STATE_CONNECTED -> finish("connect", handle, status, null, "connection_failed")
                newState == BluetoothProfile.STATE_DISCONNECTED -> {
                    val op = pending?.takeIf { it.handle == handle }
                    when (op?.kind) {
                        null -> Unit
                        "connect" -> op.result.completeExceptionally(
                            BleException("connection_failed", "GATT status $status"))
                        "disconnect" -> op.result.complete(null)
                        else -> op.result.completeExceptionally(BleException("not_connected", "Disconnected"))
                    }
                    // A link lost on its own, not one being opened or closed, is reported as an event.
                    if (op?.kind != "connect" && handle !in closing && gatts.containsKey(handle)) {
                        release(handle)
                        events?.disconnected(handle, "GATT status $status")
                    }
                }
            }
        }

        override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) =
            finish("services", handle, status, null, "discovery_failed")

        override fun onCharacteristicRead(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic,
                                          value: ByteArray, status: Int) =
            finish("read", handle, status, value, "read_failed")

        override fun onCharacteristicWrite(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, status: Int) =
            finish("write", handle, status, null, "write_failed")

        override fun onDescriptorWrite(gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, status: Int) =
            finish("descriptor", handle, status, null, "subscribe_failed")

        override fun onMtuChanged(gatt: BluetoothGatt, mtu: Int, status: Int) =
            finish("mtu", handle, status, mtu, "mtu_request_failed")

        override fun onCharacteristicChanged(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic,
                                             value: ByteArray) {
            events?.notification(handle, Uuids.show(characteristic.uuid), value)
        }
    }

    private companion object {
        const val DEFAULT_MTU = 23
        /** ATT MTU for the largest BTP segment Matter uses (244 bytes). */
        const val MATTER_MTU = 247
        const val OPERATION_TIMEOUT = 10_000L
        val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

        fun properties(flags: Int) = buildList {
            if (flags and BluetoothGattCharacteristic.PROPERTY_READ != 0) add("read")
            if (flags and BluetoothGattCharacteristic.PROPERTY_WRITE != 0) add("write")
            if (flags and BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE != 0) add("write-without-response")
            if (flags and BluetoothGattCharacteristic.PROPERTY_NOTIFY != 0) add("notify")
            if (flags and BluetoothGattCharacteristic.PROPERTY_INDICATE != 0) add("indicate")
        }
    }
}
