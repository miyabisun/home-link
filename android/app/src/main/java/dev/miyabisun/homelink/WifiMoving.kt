package dev.miyabisun.homelink

import android.os.Handler
import android.os.Looper

/**
 * One Wi-Fi move at a time. [post] runs on the screen's thread; [spawn] runs the move off it.
 * A move cannot be cancelled: the device's fail-safe, not the phone, decides where it ends up.
 */
class WifiMoving(private val post: (() -> Unit) -> Unit, private val spawn: (() -> Unit) -> Unit) {
    /** The device being moved. */
    @Volatile var device: Long? = null
        private set
    /** The finished move's device and result, until the screen takes it. */
    private var result: Pair<Long, ApiResult<Device>>? = null
    private var listener: (() -> Unit)? = null

    /** Follows the move on the screen's thread; null stops following. */
    fun follow(listener: (() -> Unit)?) {
        this.listener = listener
    }

    fun take() = result.also { result = null }

    /** Moves device [id] unless another move runs. */
    fun start(id: Long, call: () -> ApiResult<Device>) {
        if (device != null) return
        device = id
        result = null
        spawn {
            val moved = call()
            post {
                result = id to moved
                device = null
                listener?.invoke()
            }
        }
    }
}

/** Kept outside the activity so a recreated screen picks up the move and its result. */
val WifiMove = Handler(Looper.getMainLooper()).let { main ->
    WifiMoving(post = { main.post(it) }, spawn = { Thread(it).start() })
}
