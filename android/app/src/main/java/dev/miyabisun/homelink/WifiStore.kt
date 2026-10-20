package dev.miyabisun.homelink

import android.content.Context
import org.json.JSONException
import org.json.JSONObject
import java.security.GeneralSecurityException
import java.security.KeyStore
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** The Wi-Fi networks versions up to 0.1.31 kept on this phone, and the one chosen last. */
data class SavedWifi(val networks: List<WifiNetwork> = emptyList(), val last: String? = null) {
    val selected: WifiNetwork? get() = networks.firstOrNull { it.ssid == last } ?: networks.firstOrNull()

    companion object {
        /** Reads the saved list, and the single network that versions up to 0.1.13 saved. */
        fun parse(text: String): SavedWifi {
            val json = JSONObject(text)
            if (!json.has("networks")) return network(json).let { SavedWifi(listOf(it), it.ssid) }
            val list = json.getJSONArray("networks")
            return SavedWifi((0 until list.length()).map { network(list.getJSONObject(it)) },
                if (json.isNull("last")) null else json.getString("last"))
        }

        private fun network(json: JSONObject) = WifiNetwork(json.getString("ssid"), json.getString("password"))
    }
}

/**
 * Saves the networks left on this phone to home-link, the chosen one first so that it becomes
 * the one for devices when home-link has none, then forgets them; false keeps them for the next try.
 */
fun moveToServer(store: WifiStore, save: (WifiNetwork) -> ApiResult<*>): Boolean {
    val saved = store.load()
    val order = listOfNotNull(saved.selected) + saved.networks.filter { it != saved.selected }
    if (order.any { save(it) !is ApiResult.Ok }) return false
    if (order.isNotEmpty()) store.clear()
    return true
}

/** The bulbs join only 2.4GHz networks. */
fun is24GHz(frequencyMhz: Int) = frequencyMhz in 2400..2500

fun band(frequencyMhz: Int) = when {
    is24GHz(frequencyMhz) -> "2.4GHz"
    frequencyMhz >= 5925 -> "6GHz"
    else -> "5GHz"
}

interface WifiStore {
    fun load(): SavedWifi
    fun clear()
}

/**
 * Reads the networks earlier versions kept encrypted with an AES-GCM key that never leaves the Android Keystore.
 * Preferences are excluded from backups (`data_extraction_rules`).
 */
class KeystoreWifiStore(context: Context) : WifiStore {
    private val prefs = context.getSharedPreferences("wifi", Context.MODE_PRIVATE)

    override fun load(): SavedWifi {
        val sealed = prefs.getString(KEY, null) ?: return SavedWifi()
        return try {
            val bytes = Base64.getDecoder().decode(sealed)
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key() ?: return SavedWifi(), GCMParameterSpec(128, bytes, 0, IV))
            SavedWifi.parse(cipher.doFinal(bytes, IV, bytes.size - IV).decodeToString())
        } catch (_: GeneralSecurityException) {
            SavedWifi()
        } catch (_: IllegalArgumentException) {
            SavedWifi()
        } catch (_: JSONException) {
            SavedWifi()
        }
    }

    override fun clear() {
        prefs.edit().remove(KEY).apply()
        // The sealed networks are gone already; a key left behind decrypts nothing.
        try {
            KeyStore.getInstance(KEYSTORE).apply { load(null) }.deleteEntry(ALIAS)
        } catch (_: GeneralSecurityException) {
        } catch (_: java.io.IOException) {
        }
    }

    private fun key(): SecretKey? =
        KeyStore.getInstance(KEYSTORE).apply { load(null) }.getKey(ALIAS, null) as SecretKey?

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "home-link-wifi"
        const val KEY = "network"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV = 12
    }
}
