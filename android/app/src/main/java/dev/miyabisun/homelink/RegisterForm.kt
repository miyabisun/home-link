package dev.miyabisun.homelink

sealed interface Status {
    data object NotMatter : Status
    data object ScanFailed : Status
    data class Failed(val error: ApiError) : Status
    /** Connected; the device's name, empty when it has none. */
    data class Commissioned(val name: String) : Status
    data class BleFailed(val problem: BleProblem) : Status
    /** Moved onto the Wi-Fi named [ssid]. */
    data class WifiMoved(val ssid: String) : Status
    /** The app ended before `job` did; [ssid] is the Wi-Fi a moved device is on now, if read. */
    data class Interrupted(val job: DeviceJob, val ssid: String?) : Status
    /** Reading home-link for what came of `job`, which the app ended before. */
    data class Recovering(val job: DeviceJob) : Status
}

/** The longest a device job waits for home-link: the six minutes its request may read for. */
const val DEVICE_JOB_MS = 360_000L

/** What a job an ended process left turned out as, by home-link's word: [Settled], or to read again [Later]. */
sealed interface Recovery {
    /** Over: `status` says how, on `device` as read now (none when home-link no longer lists it). */
    data class Settled(val status: Status, val device: Device?) : Recovery
    /** home-link may still be at it, or was not read: `status` says which. */
    data class Later(val status: Status) : Recovery
}

/**
 * What came of `unreported` by the device `read` from home-link at wall-clock `now`, and home-link's Wi-Fi for
 * devices `deviceWifi`. A registration ends with the process, whose proxy it needs: still unconnected, it did not go
 * through. home-link moves a device's Wi-Fi alone, so a move is over once the device is on that Wi-Fi, or once the
 * request would have ended.
 */
fun recover(unreported: Unreported, read: ApiResult<Device>, deviceWifi: String?, now: Long): Recovery {
    val job = unreported.job
    val device = when (read) {
        is ApiResult.Failed ->
            // Gone after a registration: home-link kept the row that already held the device and dropped this one.
            return if (job is DeviceJob.Commission && read.error == ApiError.DEVICE_NOT_FOUND) Recovery.Settled(Status.Commissioned(""), null)
            else Recovery.Later(Status.Failed(read.error))
        is ApiResult.Ok -> read.value
    }
    return when (job) {
        is DeviceJob.Commission -> when (device.state) {
            null -> Recovery.Later(Status.Recovering(job))
            DeviceState.UNCONNECTED -> Recovery.Settled(Status.Interrupted(job, null), device)
            else -> Recovery.Settled(Status.Commissioned(device.name), device)
        }
        is DeviceJob.MoveWifi -> when {
            deviceWifi != null && device.wifiSsid == deviceWifi -> Recovery.Settled(Status.WifiMoved(deviceWifi), device)
            now - unreported.startedAt < DEVICE_JOB_MS -> Recovery.Later(Status.Recovering(job))
            else -> Recovery.Settled(Status.Interrupted(job, device.wifiSsid), device)
        }
    }
}

/** The screen reads that tell whether home-link can be read. */
enum class Read { DEVICES, LABELS, WIFI }

/** Why the phone could not lend its Bluetooth to matterjs-server. */
enum class BleProblem { PERMISSION, BLUETOOTH_OFF, PROXY_NOT_CONFIGURED, PROXY_UNREACHABLE, PROXY_VERSION }

/** How far a Bluetooth registration got, as the proxy commands show it. */
enum class Stage {
    PREPARING, SEARCHING, CONNECTING, SENDING;

    companion object {
        fun of(command: String): Stage? = when (command) {
            "start_scan" -> SEARCHING
            "connect" -> CONNECTING
            // The BTP handshake opens the channel for PASE and the network credentials.
            "write_and_subscribe" -> SENDING
            else -> null
        }
    }
}

enum class ManualError { LENGTH, CHECK_DIGIT }

class RegisterForm {
    var manualError: ManualError? = null
    var labels: List<Label> = emptyList()
    /** The ledger as last read, codes kept from earlier reads. */
    var devices: List<Device> = emptyList()
        private set
    /** Why the ledger could not be read last; none once it was. */
    var devicesError: ApiError? = null
        private set
    /** Each read failing now, in the order they failed. */
    private val readErrors = LinkedHashMap<Read, ApiError>()
    /** Why home-link cannot be read: the first of the reads failing now to fail; none once each read succeeded. */
    val readError: ApiError? get() = readErrors.values.firstOrNull()
    /** The device whose pane is open. */
    var device: Device? = null
        private set
    var busy = false
    var status: Status? = null
    /** The name of the Wi-Fi home-link hands new devices. */
    var deviceWifi: String? = null
    /** home-link was read to hold no Wi-Fi for devices. */
    var wifiMissing = false
    var stage: Stage? = null

    /** Devices still to connect, in the order they were read. */
    val unconnected get() = devices.filter { it.state == DeviceState.UNCONNECTED }

    /** The devices the list offers to connect: none while one connects, and not the one whose pane is open. */
    val pickable get() = if (busy) emptyList() else unconnected.filter { it.id != device?.id }

    /** The devices being switched. */
    var switching: Set<Long> = emptySet()
        private set

    /** What pressing a plug's switch sends: the other side of its last On/Off, on when unknown. */
    fun nextOn(device: Device) = device.on != true

    fun switchStarted(id: Long) {
        switching = switching + id
        status = null
    }

    /** home-link's answer to switching device `id`: its On/Off, or null when it did not answer. */
    fun switched(id: Long, result: ApiResult<Boolean?>) {
        switching = switching - id
        val known = devices.firstOrNull { it.id == id } ?: device?.takeIf { it.id == id } ?: return
        when (result) {
            is ApiResult.Ok -> put(result.value?.let { known.copy(on = it, state = DeviceState.CONNECTED) }
                ?: known.copy(state = DeviceState.NO_RESPONSE))
            is ApiResult.Failed -> status = Status.Failed(result.error)
        }
    }

    /** One device job runs at a time: a move holds connecting back, as connecting holds a move. */
    fun canConnect() = device?.state == DeviceState.UNCONNECTED && !busy && moving == null

    /** The device whose Wi-Fi is being moved. */
    var moving: Long? = null

    /** A connected device on another Wi-Fi than the one for devices moves onto it, one at a time. */
    fun canMoveWifi(): Boolean {
        val open = device ?: return false
        return open.state == DeviceState.CONNECTED && open.wifiSsid != null && deviceWifi != null &&
            open.wifiSsid != deviceWifi && moving == null && !busy
    }

    fun wifiMoved(result: ApiResult<Device>) {
        when (result) {
            is ApiResult.Ok -> {
                put(result.value)
                status = Status.WifiMoved(result.value.wifiSsid.orEmpty())
            }
            is ApiResult.Failed -> status = Status.Failed(result.error)
        }
    }

    /** Takes home-link's Wi-Fi networks; a failed read keeps what was read last. */
    fun wifiRead(result: ApiResult<List<SavedNetwork>>) {
        readEnded(Read.WIFI, result)
        if (result !is ApiResult.Ok) return
        deviceWifi = result.value.firstOrNull { it.forDevices }?.ssid
        wifiMissing = deviceWifi == null
    }

    /** The Matter payload to send for a scanned code; anything else is refused. */
    fun scanned(value: String): String? {
        val code = value.trim()
        status = if (code.startsWith("MT:")) null else Status.NotMatter
        return code.takeIf { status == null }
    }

    /** The code once all digits are typed; a wrong check digit shows at once. */
    fun typed(text: String): String? {
        val digits = ManualCode.digits(text)
        if (digits.length < ManualCode.LENGTH) {
            manualError = null
            return null
        }
        return accept(digits)
    }

    /** Like [typed], but also reports a code that is still too short. */
    fun submitted(text: String): String? {
        val digits = ManualCode.digits(text)
        if (digits.length < ManualCode.LENGTH) {
            manualError = ManualError.LENGTH
            return null
        }
        return accept(digits)
    }

    private fun accept(digits: String): String? {
        val valid = ManualCode.isValid(digits)
        manualError = if (valid) null else ManualError.CHECK_DIGIT
        return digits.takeIf { valid }
    }

    /** home-link's answer to a read `code`: the device to edit, new or already in the ledger. */
    fun added(code: String, result: ApiResult<Device>) {
        when (result) {
            is ApiResult.Ok -> {
                status = null
                opened(result.value.copy(code = code))
            }
            is ApiResult.Failed -> status = Status.Failed(result.error)
        }
    }

    fun opened(device: Device) {
        this.device = device
        put(device)
    }

    /** Takes home-link's labels; a failed read keeps what was read last. */
    fun labelsRead(result: ApiResult<List<Label>>) {
        readEnded(Read.LABELS, result)
        if (result is ApiResult.Ok) labels = result.value
    }

    private fun readEnded(read: Read, result: ApiResult<*>) {
        if (result is ApiResult.Failed) readErrors.putIfAbsent(read, result.error) else readErrors.remove(read)
    }

    fun devicesRead(result: ApiResult<List<Device>>) {
        readEnded(Read.DEVICES, result)
        devicesError = (result as? ApiResult.Failed)?.error
        if (result !is ApiResult.Ok) return
        // A successful read proves the server is reachable again.
        if (status == Status.Failed(ApiError.UNREACHABLE)) status = null
        val codes = devices.associate { it.id to it.code }
        devices = result.value.map { it.copy(code = it.code ?: codes[it.id]) }
        device = device?.let { open -> devices.firstOrNull { it.id == open.id } }
    }

    /** One device read with its code. */
    fun detailRead(result: ApiResult<Device>) {
        if (result is ApiResult.Ok) put(result.value)
    }

    fun updated(result: ApiResult<Device>) {
        when (result) {
            is ApiResult.Ok -> put(result.value)
            is ApiResult.Failed -> status = Status.Failed(result.error)
        }
    }

    /** Replaces the device in the list and the pane, keeping the code already read. */
    private fun put(update: Device) {
        val known = devices.firstOrNull { it.id == update.id } ?: device?.takeIf { it.id == update.id }
        val merged = update.copy(code = update.code ?: known?.code)
        devices = if (devices.any { it.id == update.id }) devices.map { if (it.id == update.id) merged else it }
            else devices + merged
        if (device?.id == update.id) device = merged
    }

    /** The device being connected, until the outcome arrives. */
    var connecting: Long? = null

    /** Device `id` starts connecting. */
    fun connect(id: Long) {
        connecting = id
        busy = true
        status = null
        stage = Stage.PREPARING
    }

    fun commissioned(result: ApiResult<Device>) {
        busy = false
        stage = null
        when (result) {
            is ApiResult.Ok -> {
                val read = connecting
                val code = (devices.firstOrNull { it.id == read } ?: device?.takeIf { it.id == read })?.code
                // home-link may have kept another row that already held the device and dropped the read one.
                devices = devices.filter { it.id != read }
                val connected = result.value.copy(code = result.value.code ?: code)
                put(connected)
                if (device?.id == read) device = connected
                status = Status.Commissioned(connected.name)
            }
            is ApiResult.Failed -> status = Status.Failed(result.error)
        }
        connecting = null
    }

    /** Back to a device that can connect again. */
    fun cancelled() {
        busy = false
        stage = null
        status = null
        connecting = null
    }

    fun bleFailed(problem: BleProblem) {
        busy = false
        stage = null
        status = Status.BleFailed(problem)
        connecting = null
    }
}
