package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
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

    @Test fun acceptsRealCodesAndRejectsAnySingleDigitChange() {
        // Codes that commissioned real devices; their sharing windows have expired.
        for (code in listOf("10482735974", "04691737344", "00726849676")) {
            assertTrue(code, ManualCode.isValid(code))
            for (i in code.indices) for (d in '0'..'9') {
                if (code[i] == d) continue
                val changed = code.replaceRange(i, i + 1, d.toString())
                assertFalse(changed, ManualCode.isValid(changed))
            }
        }
    }

    @Test fun placesTheCursorAfterTheSameNumberOfDigits() {
        assertEquals(0, ManualCode.cursor("3497 011 2", 0))
        assertEquals(4, ManualCode.cursor("3497 011 2", 4))
        assertEquals(6, ManualCode.cursor("3497 011 2", 5))
        assertEquals(10, ManualCode.cursor("3497 011 2", 8))
        assertEquals(10, ManualCode.cursor("3497 011 2", 11))
    }

    @Test fun derivesTheManualCodeFromAQrPayload() {
        // The Matter specification's example pair.
        assertEquals("34970112332", ManualCode.fromQr("MT:Y.K9042C00KA0648G00"))
        // A made-up pair: discriminator 2770, passcode 61842907.
        assertEquals("24245937741", ManualCode.fromQr("MT:634J0KQS02MEGE2PB10"))
    }

    @Test fun derivesNothingFromPayloadsItCannotDecode() {
        for (payload in listOf(
            "", "MT:", "Y.K9042C00KA0648G00", "MT:Y.K9042C00KA0648G0", "MT:Y.K9042C00KA0648G0a",
            "MT:Y.K9042C00KA0648G00*Y.K9042C00KA0648G00",
            // A custom commissioning flow needs the 21-digit code.
            "MT:634J0CKV02MEGE2PB10",
            // Version 1 is not defined.
            "MT:734J0KQS02MEGE2PB10",
        )) assertNull(payload, ManualCode.fromQr(payload))
    }

    @Test fun labelsBothFormsLikeTheDevice() {
        assertEquals("3497-011-2332", ManualCode.label("MT:Y.K9042C00KA0648G00"))
        assertEquals("2424-593-7741", ManualCode.label("24245937741"))
        assertNull(ManualCode.label("MT:Y.K9042C00KA0648G0"))
    }
}
