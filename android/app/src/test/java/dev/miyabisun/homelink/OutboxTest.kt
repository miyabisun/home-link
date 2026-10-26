package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Test

class OutboxTest {
    private class Screen(val name: String) {
        val heard = mutableListOf<String>()
    }

    /** The changes' thread, run when the test says. */
    private val queued = mutableListOf<() -> Unit>()
    private val outbox = Outbox<Screen>(post = { it() }, spawn = { queued += it })
    private var calls = 0

    private fun send(result: String) = outbox.send({ calls++; result }) { heard += "$name: $it" }

    @Test fun aChangeGoesToTheEndAndItsResultToTheScreenOpenThen() {
        val first = Screen("first")
        outbox.follow(first)
        send("saved")
        // The screen is recreated, as on rotation, while the change is under way.
        outbox.follow(null)
        val second = Screen("second")
        outbox.follow(second)
        queued.removeAt(0)()
        assertEquals(1, calls)
        assertEquals(emptyList<String>(), first.heard)
        assertEquals(listOf("second: saved"), second.heard)
    }

    @Test fun aResultWithNoScreenOpenWaitsForTheNextOneAndIsHeardOnce() {
        outbox.follow(Screen("left"))
        send("failed")
        send("saved")
        outbox.follow(null)
        queued.forEach { it() }
        val next = Screen("next")
        outbox.follow(next)
        assertEquals(listOf("next: failed", "next: saved"), next.heard)
        outbox.follow(null)
        val again = Screen("again")
        outbox.follow(again)
        assertEquals(emptyList<String>(), again.heard)
    }

    @Test fun aScreenThatStopsFollowingOnlyStopsItsOwn() {
        val old = Screen("old")
        val new = Screen("new")
        outbox.follow(old)
        // Android may create the new screen before it destroys the old one.
        outbox.follow(new)
        outbox.unfollow(old)
        send("saved")
        queued.single()()
        assertEquals(listOf("new: saved"), new.heard)
    }
}
