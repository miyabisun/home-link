package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DeviceWorkTest {
    private class FakeLink : BleLink {
        var onCommand: ((String) -> Unit)? = null
        var closed = 0
        override fun open(onCommand: (String) -> Unit): ProxyOpen {
            this.onCommand = onCommand
            return ProxyOpen.OK
        }
        override fun close() { closed++ }
    }

    private class Saved : JobRecord {
        var value: Unreported? = null
        override fun read() = value
        override fun write(unreported: Unreported?) { value = unreported }
    }

    private val done = ApiResult.Ok(Device(1, "押入れ1", emptySet(), DeviceState.CONNECTED, null))
    private val moved = ApiResult.Ok(Device(1, "押入れ1", emptySet(), DeviceState.CONNECTED, null, "iot-2g"))
    /** The job's thread, run when the test says. */
    private var thread: (() -> Unit)? = null
    private val record = Saved()
    private val kept = mutableListOf<DeviceJob?>()
    private val work = DeviceWork(post = { it() }, spawn = { thread = it }, record = record, keep = { kept += it }, clock = { 1_000 })

    @Test fun aFinishedRegistrationLeavesItsOutcome() {
        val link = FakeLink()
        var heard = 0
        work.follow { heard++ }
        assertTrue(work.start(DeviceJob.Commission(1), link) { link.onCommand!!("connect"); done })
        assertEquals(Stage.PREPARING, work.stage)
        thread!!()
        assertNull(work.job)
        assertEquals(Finished(DeviceJob.Commission(1), Outcome.Done(done)), work.take())
        assertNull(work.take())
        assertEquals(1, link.closed)
        assertEquals(2, heard)
    }

    @Test fun cancellingClosesTheProxyAndDropsTheLateAnswer() {
        val link = FakeLink()
        var heard = 0
        work.follow { heard++ }
        work.start(DeviceJob.Commission(1), link) {
            work.cancel()
            // What arrives after the cancel shows nowhere.
            link.onCommand!!("connect")
            done
        }
        thread!!()
        assertNull(work.job)
        assertNull(work.stage)
        assertNull(work.take())
        assertTrue(link.closed >= 1)
        assertEquals(0, heard)
    }

    @Test fun aCancelBeforeTheProxyOpensSkipsTheRegistration() {
        var called = false
        work.start(DeviceJob.Commission(1), FakeLink()) { called = true; done }
        work.cancel()
        assertNull(work.job)
        thread!!()
        assertFalse(called)
        assertNull(work.take())
    }

    @Test fun aNewRegistrationIsNotTakenOverByTheCancelledOne() {
        work.start(DeviceJob.Commission(1), FakeLink()) { done }
        val cancelled = thread!!
        work.cancel()
        work.start(DeviceJob.Commission(2), FakeLink()) { done }
        cancelled()
        assertEquals(DeviceJob.Commission(2), work.job)
        assertEquals(Stage.PREPARING, work.stage)
        assertNull(work.take())
    }

    @Test fun oneJobRunsAtATimeAndAMoveLeavesItsResultOnce() {
        var heard = 0
        var calls = 0
        work.follow { heard++ }
        assertTrue(work.start(DeviceJob.MoveWifi(1), null) { calls++; moved })
        assertFalse(work.start(DeviceJob.MoveWifi(2), null) { calls++; moved })
        assertFalse(work.start(DeviceJob.Commission(2), FakeLink()) { calls++; done })
        // A recreated screen sees which device is moving; a move has no stages and cannot be cancelled.
        assertEquals(DeviceJob.MoveWifi(1), work.job)
        assertNull(work.stage)
        work.cancel()
        assertEquals(DeviceJob.MoveWifi(1), work.job)
        assertNull(work.take())
        thread!!()
        assertEquals(1, calls)
        assertNull(work.job)
        assertEquals(Finished(DeviceJob.MoveWifi(1), Outcome.Done(moved)), work.take())
        assertNull(work.take())
        assertEquals(1, heard)
    }

    @Test fun theProcessIsKeptUpWhileAJobRuns() {
        work.start(DeviceJob.MoveWifi(1), null) { moved }
        assertEquals(listOf<DeviceJob?>(DeviceJob.MoveWifi(1)), kept)
        thread!!()
        assertEquals(listOf(DeviceJob.MoveWifi(1), null), kept)
        work.start(DeviceJob.Commission(2), FakeLink()) { done }
        work.cancel()
        assertEquals(listOf(DeviceJob.MoveWifi(1), null, DeviceJob.Commission(2), null), kept)
    }

    @Test fun aJobIsRecordedUntilItsOutcomeIsTaken() {
        work.start(DeviceJob.MoveWifi(1), null) { moved }
        assertEquals(Unreported(DeviceJob.MoveWifi(1), 1_000), record.value)
        // Running here, it is not one an earlier process left.
        assertNull(work.unreported())
        thread!!()
        assertEquals(Unreported(DeviceJob.MoveWifi(1), 1_000), record.value)
        assertNull(work.unreported())
        work.take()
        assertNull(record.value)
        work.start(DeviceJob.Commission(2), FakeLink()) { done }
        work.cancel()
        assertNull(record.value)
    }

    @Test fun aJobAnEarlierProcessLeftIsUnreportedUntilSettled() {
        record.value = Unreported(DeviceJob.Commission(3), 500)
        assertEquals(Unreported(DeviceJob.Commission(3), 500), work.unreported())
        work.settle()
        assertNull(work.unreported())
        assertNull(record.value)
    }
}
