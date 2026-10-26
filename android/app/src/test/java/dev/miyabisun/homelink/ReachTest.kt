package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Test
import java.io.IOException
import java.io.InterruptedIOException
import java.net.ConnectException
import java.net.NoRouteToHostException
import java.net.SocketException
import java.net.SocketTimeoutException
import java.net.UnknownHostException
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

class ReachTest {
    /** A phone whose clock only moves when it waits or an attempt takes time, and whose link the test sets. */
    private class Phone(var link: Link = Link.USABLE) : PhoneLink {
        @Volatile var clock = 0L
        val waits: MutableList<Long> = Collections.synchronizedList(mutableListOf())
        override fun state() = link
        override fun await(ms: Long) {
            waits += ms
            clock += ms
        }
    }

    private val phone = Phone()
    private val reach = Reach(phone, headStartMs = 200) { phone.clock }
    private val sent: MutableList<String> = Collections.synchronizedList(mutableListOf())

    /** Answers each attempt by its URL from `outcomes`, taking `takes` ms; the last outcome of a URL repeats. */
    private fun send(urls: List<String>, patience: Long, takes: Long = 0, outcomes: Map<String, List<Attempt<String>>>): ApiResult<String> {
        val left = outcomes.mapValues { it.value.toMutableList() }
        return reach.send(urls, patience) { url, _, _ ->
            sent += url
            phone.clock += takes
            val queue = left.getValue(url)
            synchronized(queue) { if (queue.size > 1) queue.removeAt(0) else queue.single() }
        }
    }

    @Test fun onlyFailuresBeforeTheRequestWasSentAreTriedAgain() {
        assertEquals(listOf(Miss.UNRESOLVED, Miss.REFUSED, Miss.CONNECT_TIMEOUT), Miss.entries.filter { it.beforeSent })
        assertEquals(Miss.UNRESOLVED, miss(UnknownHostException("homeserver"), connected = false))
        assertEquals(Miss.REFUSED, miss(ConnectException("refused"), connected = false))
        assertEquals(Miss.REFUSED, miss(NoRouteToHostException("no route"), connected = false))
        // Data Saver refuses a socket to an address with EPERM.
        assertEquals(Miss.REFUSED, miss(SocketException("Operation not permitted"), connected = false))
        assertEquals(Miss.CONNECT_TIMEOUT, miss(SocketTimeoutException("connect timed out"), connected = false))
        assertEquals(Miss.CONNECT_TIMEOUT, miss(InterruptedIOException("timeout"), connected = false))
        assertEquals(Miss.RESPONSE_TIMEOUT, miss(SocketTimeoutException("timeout"), connected = true))
        assertEquals(Miss.RESPONSE_TIMEOUT, miss(InterruptedIOException("timeout"), connected = true))
        assertEquals(Miss.LOST, miss(IOException("unexpected end of stream"), connected = true))
    }

    @Test fun anAnswerEndsTheSend() {
        assertEquals(ApiResult.Ok("on"), send(listOf("a"), 15_000, outcomes = mapOf("a" to listOf(Attempt.Answered(ApiResult.Ok("on"))))))
        assertEquals(listOf("a"), sent)
        sent.clear()
        // A refusal by home-link is an answer too.
        assertEquals(ApiResult.Failed(ApiError.LABEL_NOT_FOUND),
            send(listOf("a", "b"), 15_000, outcomes = mapOf("a" to listOf(Attempt.Answered(ApiResult.Failed(ApiError.LABEL_NOT_FOUND))))))
        assertEquals(listOf("a"), sent)
    }

    @Test fun aFailureBeforeConnectingIsTriedAgainUntilAnAnswer() {
        val result = send(listOf("a"), 15_000, outcomes = mapOf("a" to listOf(
            Attempt.Missed(Miss.UNRESOLVED), Attempt.Missed(Miss.REFUSED), Attempt.Answered(ApiResult.Ok("on")))))
        assertEquals(ApiResult.Ok("on"), result)
        assertEquals(listOf("a", "a", "a"), sent)
        assert(phone.clock < 15_000) { "took ${phone.clock} ms" }
    }

    @Test fun aSentRequestWithoutAnAnswerIsNeverSentAgain() {
        // No answer in time, or the connection lost after sending: whether home-link acted is unknown.
        assertEquals(ApiResult.Failed(ApiError.TIMEOUT),
            send(listOf("a", "b"), 15_000, outcomes = mapOf("a" to listOf(Attempt.Missed(Miss.RESPONSE_TIMEOUT)))))
        assertEquals(listOf("a"), sent)
        sent.clear()
        assertEquals(ApiResult.Failed(ApiError.TIMEOUT),
            send(listOf("a", "b"), 15_000, outcomes = mapOf("a" to listOf(Attempt.Missed(Miss.LOST)))))
        assertEquals(listOf("a"), sent)
    }

    @Test fun theFallbackGetsTheSameRequestOnceWhenThePrimaryFailsBeforeConnecting() {
        val result = send(listOf("a", "b"), 15_000, outcomes = mapOf(
            "a" to listOf(Attempt.Missed(Miss.UNRESOLVED)), "b" to listOf(Attempt.Answered(ApiResult.Ok("on")))))
        assertEquals(ApiResult.Ok("on"), result)
        assertEquals(listOf("a", "b"), sent)
    }

    @Test fun afterPatienceTheFailureNamesWhereItStopped() {
        // Nothing to send on: the phone's side.
        phone.link = Link.NONE
        assertEquals(ApiResult.Failed(ApiError.NO_NETWORK),
            send(listOf("a"), 15_000, outcomes = mapOf("a" to listOf(Attempt.Missed(Miss.UNRESOLVED)))))
        assertEquals(15_000L, phone.clock)
        phone.clock = 0
        phone.link = Link.BLOCKED
        assertEquals(ApiResult.Failed(ApiError.BLOCKED),
            send(listOf("a"), 15_000, outcomes = mapOf("a" to listOf(Attempt.Missed(Miss.UNRESOLVED)))))
        // A network to send on, but home-link not there: its side, refused or never connected in time.
        phone.clock = 0
        phone.link = Link.USABLE
        assertEquals(ApiResult.Failed(ApiError.UNREACHABLE),
            send(listOf("a", "b"), 15_000, outcomes = mapOf("a" to listOf(Attempt.Missed(Miss.REFUSED)), "b" to listOf(Attempt.Missed(Miss.UNRESOLVED)))))
        phone.clock = 0
        assertEquals(ApiResult.Failed(ApiError.CONNECT_TIMEOUT),
            send(listOf("a"), 15_000, takes = 15_000, outcomes = mapOf("a" to listOf(Attempt.Missed(Miss.CONNECT_TIMEOUT)))))
    }

    @Test fun eachAttemptConnectsWithinWhatIsLeftAndNoneStartsAfterPatience() {
        val connects: MutableList<Pair<String, Long>> = Collections.synchronizedList(mutableListOf())
        reach.send(listOf("a"), 15_000) { url, connect, _ ->
            connects += url to connect
            phone.clock += 6_000
            Attempt.Missed(Miss.CONNECT_TIMEOUT)
        }
        // At 0 s, a second's wait, at 7 s, a second's wait, at 14 s; none at 21 s.
        assertEquals(listOf("a" to 15_000L, "a" to 8_000L, "a" to 1_000L), connects)
        assertEquals(listOf(1_000L, 1_000L), phone.waits)
    }

    @Test fun betweenRoundsItWaitsForTheNetworkAtMostASecond() {
        send(listOf("a"), 3_000, outcomes = mapOf("a" to listOf(Attempt.Missed(Miss.UNRESOLVED))))
        assertEquals(listOf(1_000L, 1_000L, 1_000L), phone.waits)
        assertEquals(listOf("a", "a", "a"), sent)
    }

    @Test fun nothingConfiguredSendsNothing() {
        assertEquals(ApiResult.Failed(ApiError.NOT_CONFIGURED), send(emptyList(), 15_000, outcomes = emptyMap()))
        assertEquals(emptyList<String>(), sent)
    }

    private val starts: MutableList<Pair<String, Long>> = Collections.synchronizedList(mutableListOf())
    private val realReach = Reach(phone, headStartMs = 200) { System.nanoTime() / 1_000_000 }

    /**
     * Attempts that connect after their URL's delay in real time (never for null), then send only when they win
     * the race; cancelling one ends its wait.
     */
    private fun connecting(vararg delays: Pair<String, Long?>): (String, Long, Race) -> Attempt<String> {
        val delay = delays.toMap()
        return { url, _, race ->
            val cancelled = CountDownLatch(1)
            race.onCancel { cancelled.countDown() }
            starts += url to System.nanoTime()
            val wait = delay.getValue(url)
            when {
                cancelled.await(wait ?: 10_000, TimeUnit.MILLISECONDS) || wait == null -> Attempt.Missed(Miss.CONNECT_TIMEOUT)
                !race.claim() -> Attempt.Yielded
                else -> {
                    sent += url
                    Attempt.Answered(ApiResult.Ok(url))
                }
            }
        }
    }

    @Test fun aPrimaryStillConnectingAfterItsHeadStartRacesTheFallbackAndTheFirstConnectedSendsOnce() {
        val began = System.nanoTime()
        assertEquals(ApiResult.Ok("b"), realReach.send(listOf("a", "b"), 15_000, connecting("a" to null, "b" to 0L)))
        assertEquals(listOf("b"), sent)
        val (first, second) = starts.toList()
        assertEquals("a", first.first)
        assertEquals("b", second.first)
        assert(second.second - first.second >= 190_000_000) { "the fallback started before the primary's head start" }
        assert(System.nanoTime() - began < 2_000_000_000) { "waited for the primary to give up" }
    }

    @Test fun aPrimaryConnectingWithinItsHeadStartIsTheOnlyAttempt() {
        assertEquals(ApiResult.Ok("a"), realReach.send(listOf("a", "b"), 15_000, connecting("a" to 50L, "b" to 0L)))
        assertEquals(listOf("a"), starts.map { it.first })
        assertEquals(listOf("a"), sent)
    }

    @Test fun whenBothConnectOnlyTheFirstSendsAndTheOtherYields() {
        // The fallback starts at 200 ms and connects at once; the primary connects at 300 ms, after it.
        assertEquals(ApiResult.Ok("b"), realReach.send(listOf("a", "b"), 15_000, connecting("a" to 300L, "b" to 0L)))
        Thread.sleep(300)
        assertEquals(listOf("b"), sent)
    }

    @Test fun aPrimaryFailingFastHandsOverWithoutItsHeadStart() {
        val began = System.nanoTime()
        val result = realReach.send(listOf("a", "b"), 15_000) { url, _, race ->
            starts += url to System.nanoTime()
            if (url == "a") Attempt.Missed(Miss.UNRESOLVED)
            else if (race.claim()) Attempt.Answered(ApiResult.Ok(url)) else Attempt.Yielded
        }
        assertEquals(ApiResult.Ok("b"), result)
        assert(System.nanoTime() - began < 150_000_000) { "waited out the head start" }
    }
}
