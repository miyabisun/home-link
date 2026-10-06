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
    data class Done(val result: ApiResult<Commissioned>) : Outcome
    data class Failed(val problem: BleProblem) : Outcome
}

/**
 * One Bluetooth registration at a time, kept outside the activity so a
 * recreated screen picks up its stage and result instead of abandoning it.
 */
object CommissionSession {
    @Volatile var stage: Stage? = null
        private set
    /** The finished registration's outcome, until the screen takes it. */
    @Volatile private var outcome: Outcome? = null
    private var listener: (() -> Unit)? = null
    private val main = Handler(Looper.getMainLooper())

    val running get() = stage != null

    /** Follows progress on the main thread; null stops following. */
    fun follow(listener: (() -> Unit)?) {
        this.listener = listener
    }

    fun take(): Outcome? = outcome.also { outcome = null }

    fun start(link: BleLink, call: () -> ApiResult<Commissioned>) {
        stage = Stage.PREPARING
        outcome = null
        Thread {
            val opened = link.open { command -> Stage.of(command)?.let { update(it) } }
            val result = when (opened) {
                ProxyOpen.OK -> Outcome.Done(call())
                ProxyOpen.UNREACHABLE -> Outcome.Failed(BleProblem.PROXY_UNREACHABLE)
                ProxyOpen.UNSUPPORTED_VERSION -> Outcome.Failed(BleProblem.PROXY_VERSION)
            }
            link.close()
            main.post {
                outcome = result
                stage = null
                listener?.invoke()
            }
        }.start()
    }

    private fun update(next: Stage) = main.post {
        if (stage != null) {
            stage = next
            listener?.invoke()
        }
    }
}
