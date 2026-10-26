package dev.miyabisun.homelink

import android.content.Context
import android.os.Handler
import android.os.Looper

/** The phone's side of the BLE proxy, open only while one registration runs. */
interface BleLink {
    /** Connects to matterjs-server's `/ble` and greets it; [onCommand] hears each command it sends. */
    fun open(onCommand: (String) -> Unit): ProxyOpen
    /** Closes the WebSocket and every BLE connection. */
    fun close()
}

/** What the phone has home-link do with one device, taking up to minutes. */
sealed interface DeviceJob {
    val device: Long

    /** A Bluetooth registration, over the BLE proxy the phone holds open. */
    data class Commission(override val device: Long) : DeviceJob
    /** A move onto home-link's Wi-Fi for devices, which home-link does alone. */
    data class MoveWifi(override val device: Long) : DeviceJob
}

sealed interface Outcome {
    data class Done(val result: ApiResult<Device>) : Outcome
    data class Failed(val problem: BleProblem) : Outcome
}

data class Finished(val job: DeviceJob, val outcome: Outcome)

/** A job started at [startedAt] (wall-clock ms) whose outcome no screen has taken yet. */
data class Unreported(val job: DeviceJob, val startedAt: Long)

/** Where the unreported job outlives the process. */
interface JobRecord {
    fun read(): Unreported?
    fun write(unreported: Unreported?)
}

/**
 * One device job at a time, kept outside the screens so a recreated or reopened screen picks up its stage and outcome.
 * [post] runs on the screen's thread; [spawn] runs the job off it. [keep] hears the job running, and null once none
 * runs, to keep the process up meanwhile. The job stays in [record] until a screen takes its outcome, so should the
 * process end first, the next one finds it [unreported] and reads home-link for what came of it.
 * A move cannot be cancelled: the device's fail-safe, not the phone, decides where it ends up.
 */
class DeviceWork(private val post: (() -> Unit) -> Unit, private val spawn: (() -> Unit) -> Unit,
                 private val record: JobRecord, private val keep: (DeviceJob?) -> Unit = {}, private val clock: () -> Long) {
    @Volatile var job: DeviceJob? = null
        private set
    /** How far a running registration got; none for a move. */
    @Volatile var stage: Stage? = null
        private set
    /** The finished job, until the screen takes it. */
    @Volatile private var finished: Finished? = null
    private var listener: (() -> Unit)? = null
    private var link: BleLink? = null
    /** Counts jobs; a cancelled one's later news no longer matches. */
    @Volatile private var run = 0

    /** Follows the job on the screen's thread; null stops following. */
    fun follow(listener: (() -> Unit)?) {
        this.listener = listener
    }

    fun take(): Finished? = finished?.also {
        finished = null
        record.write(null)
    }

    /** The job an earlier process left without a screen taking its outcome; none while one runs or waits here. */
    fun unreported(): Unreported? = if (job == null && finished == null) record.read() else null

    /** Forgets the unreported job once a screen has shown what came of it. */
    fun settle() {
        if (job == null && finished == null) record.write(null)
    }

    /** Starts `job` unless another runs: `link` is a registration's proxy, opened before `call`. */
    fun start(job: DeviceJob, link: BleLink?, call: () -> ApiResult<Device>): Boolean {
        if (this.job != null) return false
        val id = ++run
        this.job = job
        this.link = link
        stage = if (link != null) Stage.PREPARING else null
        finished = null
        record.write(Unreported(job, clock()))
        keep(job)
        spawn {
            val opened = if (link == null) ProxyOpen.OK else if (id == run) link.open { command -> Stage.of(command)?.let { update(id, it) } } else null
            val outcome = when (opened) {
                null -> null
                ProxyOpen.OK -> if (id == run) Outcome.Done(call()) else null
                ProxyOpen.UNREACHABLE -> Outcome.Failed(BleProblem.PROXY_UNREACHABLE)
                ProxyOpen.UNSUPPORTED_VERSION -> Outcome.Failed(BleProblem.PROXY_VERSION)
            }
            link?.close()
            post {
                if (id == run) {
                    finished = outcome?.let { Finished(job, it) }
                    end()
                    listener?.invoke()
                }
            }
        }
        return true
    }

    /** Stops following a registration and closes its proxy; home-link's late answer is dropped. */
    fun cancel() {
        if (job !is DeviceJob.Commission) return
        run++
        link?.close()
        record.write(null)
        end()
    }

    private fun end() {
        job = null
        stage = null
        link = null
        keep(null)
    }

    private fun update(id: Int, next: Stage) = post {
        if (id == run) {
            stage = next
            listener?.invoke()
        }
    }

    companion object {
        @Volatile private var instance: DeviceWork? = null

        /** The app's one, kept in its process and recorded in its preferences; the service keeps the process up. */
        fun of(context: Context): DeviceWork = instance ?: synchronized(this) {
            instance ?: context.applicationContext.let { app ->
                val main = Handler(Looper.getMainLooper())
                DeviceWork(post = { main.post(it) }, spawn = { Thread(it).start() }, record = PreferenceRecord(app),
                    keep = { DeviceWorkService.keep(app, it) }, clock = System::currentTimeMillis)
            }.also { instance = it }
        }
    }
}

/** The unreported job in the app's preferences. */
private class PreferenceRecord(context: Context) : JobRecord {
    private val prefs = context.getSharedPreferences("device_work", Context.MODE_PRIVATE)

    override fun read(): Unreported? {
        val device = prefs.getLong("device", -1).takeIf { it >= 0 } ?: return null
        val job = when (prefs.getString("kind", null)) {
            "commission" -> DeviceJob.Commission(device)
            "move_wifi" -> DeviceJob.MoveWifi(device)
            else -> return null
        }
        return Unreported(job, prefs.getLong("started", 0))
    }

    override fun write(unreported: Unreported?) {
        val edit = prefs.edit().clear()
        if (unreported != null) {
            edit.putString("kind", if (unreported.job is DeviceJob.Commission) "commission" else "move_wifi")
                .putLong("device", unreported.job.device).putLong("started", unreported.startedAt)
        }
        // Written at once: the process may end right after a job starts.
        edit.commit()
    }
}
