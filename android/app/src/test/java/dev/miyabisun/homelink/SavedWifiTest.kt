package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SavedWifiTest {
    private val home = WifiNetwork("home-2g", "kakushi")
    private val guest = WifiNetwork("guest", "welcome")

    @Test fun theLastChosenNetworkIsSelectedAndFallsBackToTheFirst() {
        assertNull(SavedWifi().selected)
        val saved = SavedWifi().added(home).added(guest)
        assertEquals(listOf(home, guest), saved.networks)
        assertEquals(guest, saved.selected)
        assertEquals(home, saved.chose("home-2g").selected)
        // An unknown name keeps the current choice.
        assertEquals(guest, saved.chose("nowhere").selected)
        assertEquals(home, SavedWifi(listOf(home, guest), last = "gone").selected)
    }

    @Test fun addingTheSameNameReplacesItsPasswordInPlace() {
        val saved = SavedWifi().added(home).added(guest).added(WifiNetwork("home-2g", "new-pass"))
        assertEquals(listOf(WifiNetwork("home-2g", "new-pass"), guest), saved.networks)
        assertEquals("home-2g", saved.last)
    }

    @Test fun removingTheSelectedNetworkSelectsTheFirstLeft() {
        val saved = SavedWifi().added(home).added(guest)
        assertEquals(SavedWifi(listOf(home), last = "home-2g"), saved.removed("guest"))
        assertEquals(SavedWifi(listOf(guest), last = "guest"), saved.removed("home-2g"))
        assertEquals(SavedWifi(), saved.removed("guest").removed("home-2g"))
    }

    @Test fun roundTripsThroughJson() {
        val saved = SavedWifi().added(home).added(WifiNetwork("カフェ \"2.4\"", "p\\w"))
        assertEquals(saved, SavedWifi.parse(saved.json()))
        assertEquals(SavedWifi(), SavedWifi.parse(SavedWifi().json()))
    }

    @Test fun readsTheSingleNetworkEarlierVersionsSaved() {
        assertEquals(SavedWifi(listOf(home), last = "home-2g"),
            SavedWifi.parse("""{"ssid":"home-2g","password":"kakushi"}"""))
    }

    @Test fun onlyTheTwoPointFourGigahertzBandSuitsTheBulbs() {
        assertTrue(is24GHz(2412))
        assertTrue(is24GHz(2484))
        assertFalse(is24GHz(5180))
        assertFalse(is24GHz(5955))
        assertEquals("5GHz", band(5180))
        assertEquals("5GHz", band(5885))
        assertEquals("6GHz", band(5955))
        assertEquals("6GHz", band(7115))
        assertEquals("2.4GHz", band(2437))
    }
}
