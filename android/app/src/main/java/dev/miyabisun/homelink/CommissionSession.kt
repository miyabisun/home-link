package dev.miyabisun.homelink

import android.os.Handler
import android.os.Looper

/** The phone's side of the BLE proxy, open only while one registration runs. */
interface BleLink {
    /** Connects to matterjs-server's `/ble` and greets it; [onCommand] hears each command it sends. */
    fun open(onCommand: (String) -> Unit): ProxyOpen
    /** Closes the WebSocket and every BLE connection. */
    fun close()
}

sealed interface Outcome {
    data class Done(val result: ApiResult<Device>) : Outcome
    data class Failed(val problem: BleProblem) : Outcome
}

/**
 * One Bluetooth registration at a time. [post] runs on the screen's thread;
 * [spawn] runs the registration off it.
 */
class Commissioning(private val post: (() -> Unit) -> Unit, private val spawn: (() -> Unit) -> Unit) {
    @Volatile var stage: Stage? = null
        private set
    /** The finished registration's outcome, until the screen takes it. */
    @Volatile private var outcome: Outcome? = null
    private var listener: (() -> Unit)? = null
    private var link: BleLink? = null
    /** Counts registrations; a cancelled one's later news no longer matches. */
    @Volatile private var run = 0

    val running get() = stage != null

    /** Follows progress on the screen's thread; null stops following. */
    fun follow(listener: (() -> Unit)?) {
        this.listener = listener
    }

    fun take(): Outcome? = outcome.also { outcome = null }

    fun start(link: BleLink, call: () -> ApiResult<Device>) {
        val id = ++run
        this.link = link
        stage = Stage.PREPARING
        outcome = null
        spawn {
            val opened = if (id == run) link.open { command -> Stage.of(command)?.let { update(id, it) } } else null
            val result = when (opened) {
                null -> null
                ProxyOpen.OK -> if (id == run) Outcome.Done(call()) else null
                ProxyOpen.UNREACHABLE -> Outcome.Failed(BleProblem.PROXY_UNREACHABLE)
                ProxyOpen.UNSUPPORTED_VERSION -> Outcome.Failed(BleProblem.PROXY_VERSION)
            }
            link.close()
            post {
                if (id == run) {
                    outcome = result
                    stage = null
                    listener?.invoke()
                }
            }
        }
    }

    /** Stops following the registration and closes its proxy; home-link's late answer is dropped. */
    fun cancel() {
        run++
        stage = null
        link?.close()
        link = null
    }

    private fun update(id: Int, next: Stage) = post {
        if (id == run) {
            stage = next
            listener?.invoke()
        }
    }
}

/** Kept outside the activity so a recreated screen picks up the stage and result instead of abandoning them. */
val CommissionSession = Handler(Looper.getMainLooper()).let { main ->
    Commissioning(post = { main.post(it) }, spawn = { Thread(it).start() })
}
