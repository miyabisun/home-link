package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class EndpointsTest {
    @Test fun readsBothUrlsFromFlatYaml() {
        val endpoints = Endpoints.parse("""
            # home-link
            home_link_url: http://home-link.example:5011
            ble_proxy_url: 'ws://10.0.0.5:5580/ble'  # LAN only
            other: ignored
        """.trimIndent())
        assertEquals(Endpoints("http://home-link.example:5011", "ws://10.0.0.5:5580/ble"), endpoints)
        assertEquals("http://home-link.example:5011", endpoints.homeLinkUrl)
        assertEquals("ws://10.0.0.5:5580/ble", endpoints.bleProxyUrl)
    }

    @Test fun readsDoubleQuotesWindowsLineEndsAndAHashInsideAUrl() {
        val endpoints = Endpoints.parse("home_link_url: \"https://home-link.example/base/\"\r\nble_proxy_url: ws://a.example/ble#x\r\n")
        assertEquals(Endpoints("https://home-link.example/base/", "ws://a.example/ble#x"), endpoints)
    }

    @Test fun missingEmptyOrMalformedUrlsAreNotConfigured() {
        assertEquals(Endpoints(), Endpoints.parse(""))
        val empty = Endpoints.parse("home_link_url:\nble_proxy_url: \"\"")
        assertEquals(Endpoints(), empty)
        assertNull(empty.homeLinkUrl)
        val wrong = Endpoints.parse("home_link_url: home-link.example:5011\nble_proxy_url: http://a.example/ble")
        assertEquals("home-link.example:5011", wrong.homeLink)
        assertNull(wrong.homeLinkUrl)
        assertNull(wrong.bleProxyUrl)
    }

    @Test fun writesYamlThatReadsBack() {
        val endpoints = Endpoints("http://home-link.example:5011", "ws://10.0.0.5:5580/ble")
        val yaml = endpoints.yaml()
        assertTrue(yaml, yaml.contains("home_link_url: \"http://home-link.example:5011\"\n"))
        assertTrue(yaml, yaml.contains("ble_proxy_url: \"ws://10.0.0.5:5580/ble\"\n"))
        assertEquals(endpoints, Endpoints.parse(yaml))
        assertEquals(Endpoints(), Endpoints.parse(Endpoints().yaml()))
    }

    @Test fun homeLinkTakesHttpOrHttpsWithAHost() {
        for (url in listOf("http://home-link.example:5011", "https://home-link.example", "http://10.0.0.5:5011/",
            "HTTP://home-link.example/prefix", "http://[fd00::5]:5011")) {
            assertTrue(url, isHomeLinkUrl(url))
        }
        for (url in listOf("", "home-link.example:5011", "ws://home-link.example", "http://", "http:///api",
            "http://home-link.example:5011?x=1", "http://home-link.example/#top", "http://user@home-link.example",
            "http://home link.example", "http://home-link.example:0", "ftp://home-link.example")) {
            assertFalse(url, isHomeLinkUrl(url))
        }
    }

    @Test fun bleProxyTakesWsOrWss() {
        assertTrue(isBleProxyUrl("ws://10.0.0.5:5580/ble"))
        assertTrue(isBleProxyUrl("wss://matter.example/ble"))
        assertFalse(isBleProxyUrl("http://10.0.0.5:5580/ble"))
        assertFalse(isBleProxyUrl("ws://"))
    }

    @Test fun namesTheHostAndPortForMessages() {
        assertEquals("home-link.example:5011", authority("http://home-link.example:5011/base"))
        assertEquals("matter.example", authority("wss://matter.example/ble"))
        assertEquals("", authority(null))
    }
}
