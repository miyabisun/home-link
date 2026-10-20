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
        assertEquals(guest, SavedWifi(listOf(home, guest), last = "guest").selected)
        assertEquals(home, SavedWifi(listOf(home, guest), last = "gone").selected)
    }

    @Test fun readsWhatEarlierVersionsSaved() {
        assertEquals(SavedWifi(listOf(home, guest), last = "guest"),
            SavedWifi.parse("""{"networks":[{"ssid":"home-2g","password":"kakushi"},""" +
                """{"ssid":"guest","password":"welcome"}],"last":"guest"}"""))
        assertEquals(SavedWifi(), SavedWifi.parse("""{"networks":[],"last":null}"""))
        // Versions up to 0.1.13 saved a single network.
        assertEquals(SavedWifi(listOf(home), last = "home-2g"),
            SavedWifi.parse("""{"ssid":"home-2g","password":"kakushi"}"""))
    }

    @Test fun theNetworksOnThePhoneMoveToHomeLinkOnceTheChosenFirst() {
        val store = MemoryStore(SavedWifi(listOf(home, guest), last = "guest"))
        val sent = mutableListOf<WifiNetwork>()
        assertTrue(moveToServer(store) { sent += it; ApiResult.Ok(Unit) })
        assertEquals(listOf(guest, home), sent)
        assertEquals(SavedWifi(), store.load())
        // Nothing is left to move the next time.
        assertTrue(moveToServer(store) { sent += it; ApiResult.Ok(Unit) })
        assertEquals(2, sent.size)
    }

    @Test fun aFailedMoveKeepsTheNetworksForTheNextTry() {
        val saved = SavedWifi(listOf(home, guest), last = "home-2g")
        val store = MemoryStore(saved)
        assertFalse(moveToServer(store) { if (it == guest) ApiResult.Failed(ApiError.UNREACHABLE) else ApiResult.Ok(Unit) })
        assertEquals(saved, store.load())
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

    private class MemoryStore(private var saved: SavedWifi) : WifiStore {
        override fun load() = saved
        override fun clear() { saved = SavedWifi() }
    }
}
