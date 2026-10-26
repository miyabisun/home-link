package dev.miyabisun.homelink

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import java.net.URI
import java.net.URISyntaxException

private val IPV4 = Regex("""(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})""")

/**
 * Whether `url`'s host is on the LAN as Android 17 guards it without ACCESS_LOCAL_NETWORK: a private (RFC 1918),
 * link-local or unique-local address, or an mDNS name. Other names are not looked up; a tailnet's are not the LAN.
 */
fun isLanUrl(url: String): Boolean {
    val host = try { URI(url).host } catch (_: URISyntaxException) { null }?.lowercase() ?: return false
    if (host.endsWith(".local")) return true
    if (host.startsWith("[")) {
        // The first group, written in full, holds both prefixes: fc00::/7 and fe80::/10.
        val group = host.removePrefix("[").substringBefore(':').takeIf { it.length == 4 }?.toIntOrNull(16) ?: return false
        return group and 0xFE00 == 0xFC00 || group and 0xFFC0 == 0xFE80
    }
    val (a, b) = IPV4.matchEntire(host)?.destructured?.let { (a, b) -> a.toInt() to b.toInt() } ?: return false
    return a == 10 || (a == 172 && b in 16..31) || (a == 192 && b == 168) || (a == 169 && b == 254)
}

/** Whether any of `endpoints` that would be connected to is on the LAN. */
fun needsLan(endpoints: Endpoints): Boolean = (endpoints.homeLinkUrls + listOfNotNull(endpoints.bleProxyUrl)).any(::isLanUrl)

/**
 * Where a request goes: `urls` to try in turn, or, with none, why it goes nowhere ([none]). [lanSkipped] when LAN URLs
 * were left out for want of the permission to connect to the LAN.
 */
data class Targets(val urls: List<String>, val none: ApiError = ApiError.NOT_CONFIGURED, val lanSkipped: Boolean = false) {
    /** A send to [urls] told apart: reaching none of them, with a LAN URL left out, is for want of the permission. */
    fun <T> explain(result: ApiResult<T>): ApiResult<T> =
        if (lanSkipped && result is ApiResult.Failed && result.error in setOf(ApiError.UNREACHABLE, ApiError.CONNECT_TIMEOUT))
            ApiResult.Failed(ApiError.LOCAL_NETWORK_DENIED)
        else result
}

/**
 * Where requests go by the endpoints `file` as read, leaving LAN URLs out unless `lanAllowed`: Android would refuse
 * them, and a refusal looks like an unreachable home-link that is worth trying again.
 */
fun targets(file: EndpointsFile, lanAllowed: Boolean): Targets {
    val urls = file.endpoints.homeLinkUrls
    val none = when (file) {
        is EndpointsFile.Unreadable -> ApiError.SETTINGS_UNREADABLE
        is EndpointsFile.Malformed -> ApiError.SETTINGS_MALFORMED
        else -> ApiError.NOT_CONFIGURED
    }
    if (lanAllowed || urls.none(::isLanUrl)) return Targets(urls, none)
    val left = urls.filterNot(::isLanUrl)
    return Targets(left, if (left.isEmpty()) ApiError.LOCAL_NETWORK_DENIED else none, lanSkipped = true)
}

/** Whether this app may connect to the LAN: Android 16 does not guard it, 17 asks ACCESS_LOCAL_NETWORK. */
fun Context.lanAllowed(): Boolean = Build.VERSION.SDK_INT < 37 ||
    checkSelfPermission(Manifest.permission.ACCESS_LOCAL_NETWORK) == PackageManager.PERMISSION_GRANTED

/** Whether the configured endpoints need the LAN permission this app does not hold; it is asked for on a screen. */
fun Context.lanMissing(): Boolean = !lanAllowed() && needsLan(Endpoints.load(this))
