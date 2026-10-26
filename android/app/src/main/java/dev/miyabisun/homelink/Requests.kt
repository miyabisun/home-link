package dev.miyabisun.homelink

import android.app.Activity
import android.os.Handler
import android.os.Looper
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

/**
 * Requests that change home-link, run to the end whatever the screen does: rotating it, turning dark or leaving it
 * must not drop a name or a label being saved. Each result goes to the screen of its kind that is open then, or waits
 * for the next one, so a failure is shown. [post] runs on the screens' thread; [spawn] runs the request off it.
 */
class Outbox<S : Any>(private val post: (() -> Unit) -> Unit, private val spawn: (() -> Unit) -> Unit) {
    private var screen: S? = null
    private val waiting = mutableListOf<S.() -> Unit>()

    /** Sends results to `screen` from now on, first those that waited; null holds them. */
    fun follow(screen: S?) {
        this.screen = screen
        if (screen == null) return
        val held = waiting.toList()
        waiting.clear()
        held.forEach { screen.it() }
    }

    /** Holds results again unless another screen follows by now, as a recreated one may before the old one goes. */
    fun unfollow(screen: S) {
        if (this.screen === screen) this.screen = null
    }

    fun <T> send(call: () -> T, done: S.(T) -> Unit) = spawn {
        val result = call()
        post {
            val open = screen
            if (open != null) open.done(result) else waiting += { done(result) }
        }
    }

    companion object {
        // Made on first use: a JVM test of the class itself has no main looper.
        private val main by lazy { Handler(Looper.getMainLooper()) }
        /** Edits run one after another, in the order made; a plug's switch, which waits on the device, apart. */
        private val EDITS: ExecutorService = Executors.newSingleThreadExecutor()
        private val SWITCHES: ExecutorService = Executors.newSingleThreadExecutor()

        fun <S : Any> edits() = Outbox<S>({ main.post(it) }, { EDITS.execute(it) })
        fun <S : Any> switches() = Outbox<S>({ main.post(it) }, { SWITCHES.execute(it) })
    }
}

/**
 * A screen's reads: side by side, so a home-link that cannot be reached shows at the first failure rather than after
 * each read in turn, and dropped with the screen, which reads again when it is open again.
 */
class Reads(private val activity: Activity) {
    private val pool = Executors.newFixedThreadPool(4)

    fun <T> run(call: () -> T, done: (T) -> Unit) = pool.execute {
        val result = call()
        activity.runOnUiThread { if (!activity.isDestroyed) done(result) }
    }

    /**
     * Takes no more reads; those under way end on their own, within their limits, and their results are dropped.
     * Interrupting one would throw out of its wait for the network and end the app.
     */
    fun close() {
        pool.shutdown()
    }
}
