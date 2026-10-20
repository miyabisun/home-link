package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CommissioningTest {
    private class FakeLink : BleLink {
        var onCommand: ((String) -> Unit)? = null
        var closed = 0
        override fun open(onCommand: (String) -> Unit): ProxyOpen {
            this.onCommand = onCommand
            return ProxyOpen.OK
        }
        override fun close() { closed++ }
    }

    private val done = ApiResult.Ok(Device(1, "押入れ1", 2, emptySet(), DeviceState.CONNECTED, null))
    /** The registration's thread, run when the test says. */
    private var thread: (() -> Unit)? = null
    private val session = Commissioning(post = { it() }, spawn = { thread = it })

    @Test fun aFinishedRegistrationLeavesItsOutcome() {
        val link = FakeLink()
        var heard = 0
        session.follow { heard++ }
        session.start(link) { link.onCommand!!("connect"); done }
        assertEquals(Stage.PREPARING, session.stage)
        thread!!()
        assertFalse(session.running)
        assertEquals(Outcome.Done(done), session.take())
        assertNull(session.take())
        assertEquals(1, link.closed)
        assertEquals(2, heard)
    }

    @Test fun cancellingClosesTheProxyAndDropsTheLateAnswer() {
        val link = FakeLink()
        var heard = 0
        session.follow { heard++ }
        session.start(link) {
            session.cancel()
            // What arrives after the cancel shows nowhere.
            link.onCommand!!("connect")
            done
        }
        thread!!()
        assertFalse(session.running)
        assertNull(session.stage)
        assertNull(session.take())
        assertTrue(link.closed >= 1)
        assertEquals(0, heard)
    }

    @Test fun aCancelBeforeTheProxyOpensSkipsTheRegistration() {
        val link = FakeLink()
        var called = false
        session.start(link) { called = true; done }
        session.cancel()
        assertFalse(session.running)
        thread!!()
        assertFalse(called)
        assertNull(session.take())
    }

    @Test fun aNewRegistrationIsNotTakenOverByTheCancelledOne() {
        val first = FakeLink()
        session.start(first) { done }
        val cancelled = thread!!
        session.cancel()
        session.start(FakeLink()) { done }
        cancelled()
        assertTrue(session.running)
        assertEquals(Stage.PREPARING, session.stage)
        assertNull(session.take())
    }
}
