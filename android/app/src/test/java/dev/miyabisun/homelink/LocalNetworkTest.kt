package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LocalNetworkTest {
    private val tailnet = "http://homeserver:5011"
    private val lan = "http://192.168.1.100:5011"

    @Test fun lanUrlsArePrivateLinkLocalAndUniqueLocalAddressesAndMdnsNames() {
        for (url in listOf(lan, "http://10.0.2.2:5011", "http://172.20.0.5:5011", "http://169.254.1.2", "ws://192.168.1.100:5580/ble",
            "http://[fd00::5]:5011", "http://[fe80::1]:5011", "http://homeserver.local:5011")) {
            assertTrue(url, isLanUrl(url))
        }
        // A tailnet's addresses and names, loopback, and public addresses are not the LAN Android guards.
        for (url in listOf(tailnet, "http://100.101.102.103:5011", "http://homeserver.tail1234.ts.net", "http://127.0.0.1:5011",
            "http://172.32.0.1", "http://8.8.8.8", "https://home-link.example", "not a url")) {
            assertFalse(url, isLanUrl(url))
        }
    }

    @Test fun withoutAnEndpointsFileNothingIsSentAndTheReasonReadsApart() {
        assertEquals(Targets(emptyList(), ApiError.NOT_CONFIGURED), targets(EndpointsFile.Missing, lanAllowed = true))
        assertEquals(Targets(emptyList(), ApiError.SETTINGS_UNREADABLE), targets(EndpointsFile.Unreadable("no storage"), lanAllowed = true))
        assertEquals(Targets(emptyList(), ApiError.NOT_CONFIGURED), targets(EndpointsFile.Read(Endpoints()), lanAllowed = true))
    }

    @Test fun aMalformedFileStillSendsWhereItsOtherLinesSay() {
        assertEquals(Targets(listOf(tailnet), ApiError.SETTINGS_MALFORMED), targets(EndpointsFile.Malformed(3, Endpoints(tailnet)), lanAllowed = true))
        assertEquals(Targets(emptyList(), ApiError.SETTINGS_MALFORMED), targets(EndpointsFile.Malformed(1, Endpoints()), lanAllowed = true))
    }

    @Test fun withoutTheLanPermissionLanUrlsAreLeftOut() {
        val both = EndpointsFile.Read(Endpoints(tailnet, fallbacks = listOf(lan)))
        assertEquals(Targets(listOf(tailnet, lan)), targets(both, lanAllowed = true))
        assertEquals(Targets(listOf(tailnet), lanSkipped = true), targets(both, lanAllowed = false))
        // With only LAN URLs, nothing is tried: Android would refuse every attempt.
        assertEquals(Targets(emptyList(), ApiError.LOCAL_NETWORK_DENIED, lanSkipped = true),
            targets(EndpointsFile.Read(Endpoints(lan)), lanAllowed = false))
    }

    @Test fun aSendThatReachedNoUrlIsPutDownToTheLanPermissionWhenALanUrlWasLeftOut() {
        val skipped = Targets(listOf(tailnet), lanSkipped = true)
        for (error in listOf(ApiError.UNREACHABLE, ApiError.CONNECT_TIMEOUT)) {
            assertEquals(ApiResult.Failed(ApiError.LOCAL_NETWORK_DENIED), skipped.explain(ApiResult.Failed(error)))
            assertEquals(ApiResult.Failed(error), Targets(listOf(tailnet)).explain(ApiResult.Failed(error)))
        }
        // Sent, or stopped on the phone: the permission is not why.
        for (error in listOf(ApiError.TIMEOUT, ApiError.NO_NETWORK, ApiError.BLOCKED, ApiError.SERVER)) {
            assertEquals(ApiResult.Failed(error), skipped.explain(ApiResult.Failed(error)))
        }
        assertEquals(ApiResult.Ok(1), skipped.explain(ApiResult.Ok(1)))
    }

    @Test fun theLanPermissionIsNeededForAnyLanEndpoint() {
        assertFalse(needsLan(Endpoints(tailnet, "ws://homeserver:5580/ble")))
        assertTrue(needsLan(Endpoints(tailnet, fallbacks = listOf(lan))))
        assertTrue(needsLan(Endpoints(tailnet, "ws://192.168.1.100:5580/ble")))
        // An invalid URL is not sent to, so it needs nothing.
        assertFalse(needsLan(Endpoints("192.168.1.100:5011")))
    }
}
