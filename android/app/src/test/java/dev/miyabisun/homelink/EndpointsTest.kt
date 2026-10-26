package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

class EndpointsTest {
    @get:Rule val folder = TemporaryFolder()
    private val endpoints = Endpoints("http://homeserver:5011", "ws://10.0.0.5:5580/ble", listOf("http://192.168.1.100:5011"))

    @Test fun aMissingFileReadsAsMissing() {
        assertEquals(EndpointsFile.Missing, Endpoints.read(File(folder.root, Endpoints.FILE)))
    }

    @Test fun noStorageOrAFileThatCannotBeReadReadsAsUnreadable() {
        assertTrue(Endpoints.read(null) is EndpointsFile.Unreadable)
        val directory = File(folder.root, Endpoints.FILE).apply { mkdir() }
        assertTrue(Endpoints.read(directory) is EndpointsFile.Unreadable)
    }

    @Test fun aFileOfAnotherShapeReadsAsMalformedAtItsLine() {
        val file = File(folder.root, Endpoints.FILE)
        file.writeText("# home-link\nhome_link_url = http://homeserver:5011\n")
        assertEquals(EndpointsFile.Malformed(2, Endpoints()), Endpoints.read(file))
        // What parses before and after the line is kept, to show and fix.
        file.writeText("home_link_url: http://homeserver:5011\nnot yaml at all\nble_proxy_url: ws://10.0.0.5:5580/ble\n")
        assertEquals(EndpointsFile.Malformed(2, Endpoints("http://homeserver:5011", "ws://10.0.0.5:5580/ble")), Endpoints.read(file))
        assertEquals(EndpointsFile.Malformed(1, Endpoints()), Endpoints.parseFile("- http://homeserver:5011"))
        assertEquals(EndpointsFile.Malformed(1, Endpoints()), Endpoints.parseFile("home_link_fallback_urls: [\"http://a.example\""))
    }

    @Test fun aWellFormedFileReadsAsRead() {
        val file = File(folder.root, Endpoints.FILE).apply { writeText(endpoints.yaml()) }
        assertEquals(EndpointsFile.Read(endpoints), Endpoints.read(file))
        assertEquals(EndpointsFile.Read(Endpoints()), Endpoints.parseFile("# nothing yet\n\nother: ignored\n"))
    }

    @Test fun aByteOrderMarkIsSkipped() {
        val file = File(folder.root, Endpoints.FILE)
        file.writeBytes(byteArrayOf(0xEF.toByte(), 0xBB.toByte(), 0xBF.toByte()) + "home_link_url: http://homeserver:5011\n".toByteArray())
        assertEquals(EndpointsFile.Read(Endpoints("http://homeserver:5011")), Endpoints.read(file))
    }

    @Test fun writingReplacesTheFileWholeAndLeavesNoTemporaryFile() {
        val file = File(folder.root, Endpoints.FILE).apply { writeText("home_link_url: http://old.example:5011\n") }
        assertTrue(Endpoints.write(file, endpoints))
        assertEquals(EndpointsFile.Read(endpoints), Endpoints.read(file))
        assertEquals(listOf(Endpoints.FILE), folder.root.list()!!.toList())
    }

    @Test fun aWriteThatFailsLeavesTheFileAsItWas() {
        val file = File(folder.root, Endpoints.FILE).apply { writeText("home_link_url: http://old.example:5011\n") }
        // The temporary file's place is taken, so the new text cannot be written beside the file.
        File(folder.root, "${Endpoints.FILE}.tmp").mkdir()
        assertFalse(Endpoints.write(file, endpoints))
        assertEquals("home_link_url: http://old.example:5011\n", file.readText())
        assertFalse(Endpoints.write(null, endpoints))
    }

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

    @Test fun aFileWithoutFallbacksReadsAsBefore() {
        val endpoints = Endpoints.parse("home_link_url: http://homeserver:5011\nble_proxy_url: ws://10.0.0.5:5580/ble\n")
        assertEquals(Endpoints("http://homeserver:5011", "ws://10.0.0.5:5580/ble"), endpoints)
        assertEquals(emptyList<String>(), endpoints.fallbacks)
        assertEquals(listOf("http://homeserver:5011"), endpoints.homeLinkUrls)
        assertEquals(emptyList<String>(), Endpoints().homeLinkUrls)
    }

    @Test fun readsFallbacksAsAFlowOrBlockListOrOneUrl() {
        val flow = Endpoints.parse("""
            home_link_url: http://homeserver:5011
            home_link_fallback_urls: ["http://192.168.1.100:5011", 'http://10.0.0.5:5011']  # LAN
        """.trimIndent())
        assertEquals(listOf("http://192.168.1.100:5011", "http://10.0.0.5:5011"), flow.fallbacks)
        assertEquals(listOf("http://homeserver:5011", "http://192.168.1.100:5011", "http://10.0.0.5:5011"), flow.homeLinkUrls)
        val block = Endpoints.parse("""
            home_link_url: http://homeserver:5011
            home_link_fallback_urls:
              - "http://192.168.1.100:5011"
              - http://10.0.0.5:5011  # LAN
            ble_proxy_url: ws://10.0.0.5:5580/ble
        """.trimIndent())
        assertEquals(flow.fallbacks, block.fallbacks)
        assertEquals("ws://10.0.0.5:5580/ble", block.bleProxy)
        assertEquals(listOf("http://192.168.1.100:5011"),
            Endpoints.parse("home_link_fallback_urls: http://192.168.1.100:5011").fallbacks)
        assertEquals(emptyList<String>(), Endpoints.parse("home_link_fallback_urls: []").fallbacks)
    }

    @Test fun onlyValidDistinctFallbacksAreTriedAfterThePrimary() {
        val endpoints = Endpoints("http://homeserver:5011", fallbacks = listOf("192.168.1.100:5011", "http://homeserver:5011", "http://192.168.1.100:5011"))
        assertEquals(listOf("http://homeserver:5011", "http://192.168.1.100:5011"), endpoints.homeLinkUrls)
        // Without the primary, nothing is configured: fallbacks back it up, they do not replace it.
        assertEquals(emptyList<String>(), Endpoints(fallbacks = listOf("http://192.168.1.100:5011")).homeLinkUrls)
    }

    @Test fun writesFallbacksThatReadBack() {
        val endpoints = Endpoints("http://homeserver:5011", "", listOf("http://192.168.1.100:5011", "http://10.0.0.5:5011"))
        val yaml = endpoints.yaml()
        assertTrue(yaml, yaml.contains("home_link_fallback_urls: [\"http://192.168.1.100:5011\", \"http://10.0.0.5:5011\"]\n"))
        assertEquals(endpoints, Endpoints.parse(yaml))
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
