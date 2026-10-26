package dev.miyabisun.homelink

import java.io.IOException
import java.io.InterruptedIOException
import java.net.UnknownHostException
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Why one attempt to reach home-link at one URL got no answer. Only a miss [beforeSent] may be tried again:
 * once the request may have reached home-link, a toggle sent twice would switch back.
 */
enum class Miss(val beforeSent: Boolean) {
    UNRESOLVED(true), REFUSED(true), CONNECT_TIMEOUT(true),
    RESPONSE_TIMEOUT(false), LOST(false),
}

/** Whether the phone has a network this app may send on now: none, or one Android blocks for it (Data Saver). */
enum class Link { USABLE, NONE, BLOCKED }

/** The phone's network as a send sees it. */
interface PhoneLink {
    fun state(): Link

    /** Waits up to `ms`, returning early once the network may have changed. */
    fun await(ms: Long)
}

/** One attempt's end: home-link's answer, success or refusal; none; or none sent, as another URL connected first. */
sealed interface Attempt<out T> {
    data class Answered<T>(val result: ApiResult<T>) : Attempt<T>
    data class Missed(val miss: Miss) : Attempt<Nothing>
    data object Yielded : Attempt<Nothing>
}

/**
 * The attempts of one round racing to connect: the first to [claim] sends, every other is cancelled and must not.
 * An attempt claims once connected and before writing anything.
 */
class Race {
    private val claimed = AtomicBoolean()
    private val cancels = mutableListOf<() -> Unit>()
    @Volatile private var over = false

    val won: Boolean get() = claimed.get()

    /** Whether this attempt may send: true for the first to ask only. */
    fun claim(): Boolean = claimed.compareAndSet(false, true)

    /** Runs `action` once the race is won or over, to stop an attempt still connecting; the winner's must do nothing. */
    fun onCancel(action: () -> Unit) {
        val now = synchronized(cancels) { if (over) true else false.also { cancels += action } }
        if (now) action()
    }

    fun cancel() {
        val actions = synchronized(cancels) { over = true; cancels.toList().also { cancels.clear() } }
        actions.forEach { it() }
    }
}

/** The miss an I/O failure is, by whether the connection to home-link was made before it. */
fun miss(e: IOException, connected: Boolean): Miss = when {
    connected -> if (e is InterruptedIOException) Miss.RESPONSE_TIMEOUT else Miss.LOST
    e is UnknownHostException -> Miss.UNRESOLVED
    e is InterruptedIOException -> Miss.CONNECT_TIMEOUT
    else -> Miss.REFUSED
}

/**
 * Sends one request to the first of home-link's URLs that connects, while the phone's network may still be coming up.
 * Each round starts the primary; a URL still connecting after `headStartMs`, or failing before that, lets the next
 * one start connecting too, and the first connected sends the request once. Rounds repeat while no URL connected and
 * `patience` lasts; a request sent is never sent again.
 */
class Reach(private val link: PhoneLink, private val headStartMs: Long = 1_000, private val now: () -> Long) {
    private companion object {
        /** The longest pause between rounds; a network coming up ends it sooner. */
        const val PAUSE_MS = 1_000L
    }

    /** `attempt` sends to a URL, connecting within the given ms and claiming the race once connected. */
    fun <T> send(urls: List<String>, patience: Long, attempt: (url: String, connectMs: Long, race: Race) -> Attempt<T>): ApiResult<T> {
        if (urls.isEmpty()) return ApiResult.Failed(ApiError.NOT_CONFIGURED)
        val deadline = now() + patience
        var last: Miss? = null
        while (true) {
            if (deadline - now() <= 0) return ApiResult.Failed(gaveUp(last))
            when (val round = round(urls, deadline, attempt)) {
                is Attempt.Answered -> return round.result
                is Attempt.Missed -> {
                    if (!round.miss.beforeSent) return ApiResult.Failed(ApiError.TIMEOUT)
                    last = round.miss
                }
                Attempt.Yielded -> {}
            }
            val left = deadline - now()
            if (left <= 0) return ApiResult.Failed(gaveUp(last))
            link.await(minOf(left, PAUSE_MS))
        }
    }

    /**
     * One round over `urls`: the winner's end, or the last miss when none connected. Attempts left connecting
     * are cancelled, and any end of theirs dropped.
     */
    private fun <T> round(urls: List<String>, deadline: Long, attempt: (String, Long, Race) -> Attempt<T>): Attempt<T> {
        val race = Race()
        val ends = LinkedBlockingQueue<Attempt<T>>()
        var started = 0
        var ended = 0
        var last: Attempt<T> = Attempt.Missed(Miss.CONNECT_TIMEOUT)
        fun start() {
            val url = urls[started++]
            val connectMs = deadline - now()
            Thread { ends.put(attempt(url, connectMs, race)) }.start()
        }
        try {
            start()
            while (true) {
                // Once one has connected, the others stop connecting; the winner ignores this.
                if (race.won) race.cancel()
                // Another URL may start once this one's head start is up, unless one has connected.
                val next = started < urls.size && !race.won && deadline - now() > 0
                val end = if (next) ends.poll(headStartMs, TimeUnit.MILLISECONDS) else ends.take()
                if (end == null) {
                    start()
                    continue
                }
                ended++
                when (end) {
                    is Attempt.Answered -> return end
                    is Attempt.Missed -> if (!end.miss.beforeSent) return end else last = end
                    Attempt.Yielded -> {}
                }
                if (started < urls.size && !race.won && deadline - now() > 0) start()
                else if (ended == started) return last
            }
        } finally {
            race.cancel()
        }
    }

    /** Where a send that never connected stopped: on the phone, or on the way to home-link. */
    private fun gaveUp(last: Miss?): ApiError = when (link.state()) {
        Link.NONE -> ApiError.NO_NETWORK
        Link.BLOCKED -> ApiError.BLOCKED
        Link.USABLE -> if (last == Miss.CONNECT_TIMEOUT) ApiError.CONNECT_TIMEOUT else ApiError.UNREACHABLE
    }
}
