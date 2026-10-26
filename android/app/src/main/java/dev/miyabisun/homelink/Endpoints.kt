package dev.miyabisun.homelink

import android.content.Context
import java.io.File
import java.io.IOException
import java.net.URI
import java.net.URISyntaxException
import java.nio.file.Files
import java.nio.file.StandardCopyOption

/**
 * Where the app sends its requests: home-link's URL, other URLs of the same home-link to fall back on, and
 * matterjs-server's BLE proxy, as written in [FILE]. A value is kept as written, so a malformed one can be
 * shown and fixed; only a valid one is used.
 */
data class Endpoints(val homeLink: String = "", val bleProxy: String = "", val fallbacks: List<String> = emptyList()) {
    val homeLinkUrl: String? get() = homeLink.takeIf(::isHomeLinkUrl)
    val bleProxyUrl: String? get() = bleProxy.takeIf(::isBleProxyUrl)

    /** The URLs a request tries in turn: the primary, then valid fallbacks; none without the primary. */
    val homeLinkUrls: List<String> get() = homeLinkUrl?.let { (listOf(it) + fallbacks.filter(::isHomeLinkUrl)).distinct() }.orEmpty()

    fun yaml(): String = "# home-link: where the app sends its requests\n" +
        "$HOME_LINK: \"$homeLink\"\n" +
        "# Also connected to when home-link's URL does not connect within a second, such as its LAN address\n" +
        "$FALLBACKS: ${fallbacks.joinToString(", ", "[", "]") { "\"$it\"" }}\n" +
        "$BLE_PROXY: \"$bleProxy\"\n"

    companion object {
        /** In the app's external files directory, which `adb push` can write without root. */
        const val FILE = "home-link.yaml"
        private const val HOME_LINK = "home_link_url"
        private const val BLE_PROXY = "ble_proxy_url"
        private const val FALLBACKS = "home_link_fallback_urls"

        /** Reads flat `key: value` YAML, as [parseFile] does, keeping what parses of a malformed one. */
        fun parse(text: String): Endpoints = parseFile(text).endpoints

        /**
         * Reads flat `key: value` YAML: comments, quotes and unknown keys are allowed, and a leading byte order mark
         * is skipped. The one list, the fallbacks, may be a `[a, b]` flow, `- a` lines under its key, or a single
         * value. A line of any other shape makes the file [EndpointsFile.Malformed] at the first such line.
         */
        fun parseFile(text: String): EndpointsFile {
            val values = mutableMapOf<String, String>()
            val fallbacks = mutableListOf<String>()
            var key: String? = null
            var malformed: Int? = null
            for ((index, line) in text.removePrefix("\uFEFF").lines().withIndex()) {
                // A comment starts at a `#` after whitespace; one inside a URL is kept.
                val trimmed = Regex("\\s#.*$").replace(line, "").trim()
                if (trimmed.isEmpty() || trimmed.startsWith("#")) continue
                if (trimmed.startsWith("- ")) {
                    // An item belongs to the key above it; another key's list is ignored like the key.
                    if (key == FALLBACKS) fallbacks += unquote(trimmed.removePrefix("- ").trim())
                    else if (key == null && malformed == null) malformed = index + 1
                    continue
                }
                val name = trimmed.substringBefore(':', "").trim()
                val value = trimmed.substringAfter(':', "").trim()
                if (!KEY.matches(name) || (value.startsWith("[") && !value.endsWith("]"))) {
                    if (malformed == null) malformed = index + 1
                    key = null
                    continue
                }
                key = name
                if (name != FALLBACKS) values[name] = unquote(value)
                else if (value.startsWith("[")) {
                    fallbacks += value.substring(1, value.length - 1).split(',').map { unquote(it.trim()) }.filter { it.isNotEmpty() }
                } else if (value.isNotEmpty()) fallbacks += unquote(value)
            }
            val endpoints = Endpoints(values[HOME_LINK].orEmpty(), values[BLE_PROXY].orEmpty(), fallbacks)
            return malformed?.let { EndpointsFile.Malformed(it, endpoints) } ?: EndpointsFile.Read(endpoints)
        }

        private val KEY = Regex("[A-Za-z_][A-Za-z0-9_]*")

        private fun unquote(value: String) =
            if (value.length >= 2 && value.first() == value.last() && value.first() in "\"'") value.substring(1, value.length - 1)
            else value

        private fun file(context: Context): File? = context.getExternalFilesDir(null)?.resolve(FILE)

        /** Reads `file` afresh; null is no storage to read it from. */
        fun read(file: File?): EndpointsFile = try {
            when {
                file == null -> EndpointsFile.Unreadable("no storage")
                !file.exists() -> EndpointsFile.Missing
                else -> parseFile(file.readText())
            }
        } catch (e: IOException) {
            EndpointsFile.Unreadable(e.toString())
        } catch (e: SecurityException) {
            EndpointsFile.Unreadable(e.toString())
        }

        /** Reads the app's file afresh. */
        fun read(context: Context): EndpointsFile = read(file(context))

        /** The endpoints the app's file sets, read afresh; what parses of a malformed one, none of an unreadable one. */
        fun load(context: Context): Endpoints = read(context).endpoints

        /**
         * Writes `endpoints` to `file` whole: into a temporary file beside it, then moved over it, so a read meanwhile
         * sees the old file or the new one and a write cut short leaves the old one. Whether they were written.
         */
        fun write(file: File?, endpoints: Endpoints): Boolean {
            file ?: return false
            val temporary = File(file.parentFile, "${file.name}.tmp")
            return try {
                temporary.writeText(endpoints.yaml())
                Files.move(temporary.toPath(), file.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
                true
            } catch (_: IOException) {
                if (temporary.isFile) temporary.delete()
                false
            }
        }

        /** Whether [endpoints] were written to the app's file. */
        fun save(context: Context, endpoints: Endpoints): Boolean = write(file(context), endpoints)

        /** The path to show for `adb push`. */
        fun path(context: Context): String = file(context)?.path ?: FILE
    }
}

/** What the endpoints file holds as last read: nothing, nothing readable, a line of another shape, or endpoints. */
sealed interface EndpointsFile {
    /** The endpoints to use: none from a file missing or unreadable, what parses of a malformed one. */
    val endpoints: Endpoints get() = Endpoints()

    data object Missing : EndpointsFile
    /** No storage to read from, or the read failed; `reason` is for the log. */
    data class Unreadable(val reason: String) : EndpointsFile
    /** `line` (from 1) is the first that is not flat `key: value` YAML; [endpoints] is what the others set. */
    data class Malformed(val line: Int, override val endpoints: Endpoints) : EndpointsFile
    data class Read(override val endpoints: Endpoints) : EndpointsFile
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

/**
 * home-link as configured when each request is sent, to the LAN only with the permission for it; instrumentation may
 * supply a fake. A broadcast receiver passes [Limits.RECEIVER].
 */
fun homeLinkApi(context: Context, limits: Limits = Limits.SCREEN): HomeLinkApi =
    MainActivity.apiFactory?.invoke() ?: HttpHomeLinkApi({ targets(Endpoints.read(context), context.lanAllowed()) },
        PhoneNetwork.of(context), log = { android.util.Log.w(LOG, it) }, limits = limits)

/** The configured home-link's host and port, for messages. */
fun homeLinkHost(context: Context): String = authority(Endpoints.load(context).homeLinkUrl)
