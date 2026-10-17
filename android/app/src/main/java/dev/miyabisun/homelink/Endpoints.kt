package dev.miyabisun.homelink

import android.content.Context
import java.io.File
import java.io.IOException
import java.net.URI
import java.net.URISyntaxException

/**
 * Where the app sends its requests: home-link's URL and matterjs-server's BLE proxy, as written in
 * [FILE]. A value is kept as written, so a malformed one can be shown and fixed; only a valid one is used.
 */
data class Endpoints(val homeLink: String = "", val bleProxy: String = "") {
    val homeLinkUrl: String? get() = homeLink.takeIf(::isHomeLinkUrl)
    val bleProxyUrl: String? get() = bleProxy.takeIf(::isBleProxyUrl)

    fun yaml(): String = "# home-link: where the app sends its requests\n" +
        "$HOME_LINK: \"$homeLink\"\n$BLE_PROXY: \"$bleProxy\"\n"

    companion object {
        /** In the app's external files directory, which `adb push` can write without root. */
        const val FILE = "home-link.yaml"
        private const val HOME_LINK = "home_link_url"
        private const val BLE_PROXY = "ble_proxy_url"

        /** Reads flat `key: value` YAML: comments, quotes and unknown keys are allowed; nothing nested. */
        fun parse(text: String): Endpoints {
            val values = text.lines().mapNotNull { line ->
                val trimmed = line.trim()
                if (trimmed.isEmpty() || trimmed.startsWith("#") || ':' !in trimmed) return@mapNotNull null
                val key = trimmed.substringBefore(':').trim()
                // A comment starts at a `#` after whitespace; one inside a URL is kept.
                val value = Regex("\\s#.*$").replace(trimmed.substringAfter(':'), "").trim()
                key to unquote(value)
            }.toMap()
            return Endpoints(values[HOME_LINK].orEmpty(), values[BLE_PROXY].orEmpty())
        }

        private fun unquote(value: String) =
            if (value.length >= 2 && value.first() == value.last() && value.first() in "\"'") value.substring(1, value.length - 1)
            else value

        private fun file(context: Context): File? = context.getExternalFilesDir(null)?.resolve(FILE)

        /** Reads the file afresh; a missing or unreadable one configures nothing. */
        fun load(context: Context): Endpoints = try {
            file(context)?.takeIf { it.isFile }?.readText()?.let(::parse) ?: Endpoints()
        } catch (_: IOException) {
            Endpoints()
        }

        /** Whether [endpoints] were written. */
        fun save(context: Context, endpoints: Endpoints): Boolean = try {
            val file = file(context) ?: return false
            file.writeText(endpoints.yaml())
            true
        } catch (_: IOException) {
            false
        }

        /** The path to show for `adb push`. */
        fun path(context: Context): String = file(context)?.path ?: FILE
    }
}

private fun uri(url: String, schemes: Set<String>): URI? = try {
    URI(url).takeIf { uri ->
        uri.scheme?.lowercase() in schemes && !uri.host.isNullOrEmpty() && uri.userInfo == null &&
            uri.query == null && uri.fragment == null && (uri.port == -1 || uri.port in 1..65535)
    }
} catch (_: URISyntaxException) {
    null
}

/** home-link's base URL: http or https with a host, and no query or fragment. */
fun isHomeLinkUrl(url: String) = uri(url, setOf("http", "https")) != null

/** matterjs-server's BLE proxy: ws or wss with a host. */
fun isBleProxyUrl(url: String) = uri(url, setOf("ws", "wss")) != null

/** The host and port of `url`, as messages name the server. */
fun authority(url: String?): String = url?.let { runCatching { URI(it).rawAuthority }.getOrNull() }.orEmpty()

/** home-link as configured when each request is sent; instrumentation may supply a fake. */
fun homeLinkApi(context: Context): HomeLinkApi =
    MainActivity.apiFactory?.invoke() ?: HttpHomeLinkApi { Endpoints.load(context).homeLinkUrl }

/** The configured home-link's host and port, for messages. */
fun homeLinkHost(context: Context): String = authority(Endpoints.load(context).homeLinkUrl)
