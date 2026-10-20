package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class WifiMovingTest {
    private val moved = ApiResult.Ok(Device(1, "押入れ1", 2, emptySet(), DeviceState.CONNECTED, null, "iot-2g"))
    /** The move's thread, run when the test says. */
    private var thread: (() -> Unit)? = null
    private val moves = WifiMoving(post = { it() }, spawn = { thread = it })

    @Test fun oneMoveRunsAtATimeAndLeavesItsResultOnce() {
        var heard = 0
        var calls = 0
        moves.follow { heard++ }
        moves.start(1) { calls++; moved }
        moves.start(2) { calls++; moved }
        // A recreated screen sees which device is moving.
        assertEquals(1L, moves.device)
        assertNull(moves.take())
        thread!!()
        assertEquals(1, calls)
        assertNull(moves.device)
        assertEquals(1L to moved, moves.take())
        assertNull(moves.take())
        assertEquals(1, heard)
    }
}
