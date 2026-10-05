package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ManualCodeTest {
    @Test fun keepsUpToElevenAsciiDigits() {
        assertEquals("34970112332", ManualCode.digits(" 3497-011 2332 "))
        assertEquals("34970112332", ManualCode.digits("349701123329"))
        assertEquals("12", ManualCode.digits("1a２2"))
        assertEquals("", ManualCode.digits(""))
    }

    @Test fun groupsTheDigitsFourThreeFour() {
        assertEquals("", ManualCode.format(""))
        assertEquals("3497", ManualCode.format("3497"))
        assertEquals("3497 0", ManualCode.format("34970"))
        assertEquals("3497 011", ManualCode.format("3497011"))
        assertEquals("3497 011 2", ManualCode.format("34970112"))
        assertEquals("3497 011 2332", ManualCode.format("34970112332"))
    }

    @Test fun checksTheVerhoeffDigit() {
        assertTrue(ManualCode.isValid("34970112332"))
        assertFalse(ManualCode.isValid("34970112331"))
        assertFalse(ManualCode.isValid("34970112323"))
        assertFalse(ManualCode.isValid("3497011233"))
    }

    @Test fun placesTheCursorAfterTheSameNumberOfDigits() {
        assertEquals(0, ManualCode.cursor("3497 011 2", 0))
        assertEquals(4, ManualCode.cursor("3497 011 2", 4))
        assertEquals(6, ManualCode.cursor("3497 011 2", 5))
        assertEquals(10, ManualCode.cursor("3497 011 2", 8))
        assertEquals(10, ManualCode.cursor("3497 011 2", 11))
    }
}
