package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.util.UUID

class UuidsTest {
    private val matter = UUID.fromString("0000fff6-0000-1000-8000-00805f9b34fb")
    private val c1 = UUID.fromString("18ee2ef5-263d-4559-959f-4f9c429f9d11")

    @Test fun acceptsEveryFormTheProtocolAllows() {
        for (form in listOf("fff6", "FFF6", "0000FFF6-0000-1000-8000-00805F9B34FB", "0000fff600001000800000805f9b34fb")) {
            assertEquals(form, matter, Uuids.parse(form))
        }
        assertEquals(c1, Uuids.parse("18EE2EF5263D4559959F4F9C429F9D11"))
        assertNull(Uuids.parse("not-a-uuid"))
        assertNull(Uuids.parse("fff"))
    }

    @Test fun showsStandardUuidsShortAndOthersInFull() {
        assertEquals("FFF6", Uuids.show(matter))
        assertEquals("18EE2EF5-263D-4559-959F-4F9C429F9D11", Uuids.show(c1))
    }
}
