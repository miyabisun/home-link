package dev.miyabisun.homelink

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject
import java.security.GeneralSecurityException
import java.security.KeyStore
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** The Wi-Fi networks to hand new devices, kept on this phone only, and the one chosen last. */
data class SavedWifi(val networks: List<WifiNetwork> = emptyList(), val last: String? = null) {
    val selected: WifiNetwork? get() = networks.firstOrNull { it.ssid == last } ?: networks.firstOrNull()

    /** Saves the network, replacing one of the same name in place, and chooses it. */
    fun added(network: WifiNetwork): SavedWifi {
        val index = networks.indexOfFirst { it.ssid == network.ssid }
        val list = if (index < 0) networks + network else networks.toMutableList().apply { set(index, network) }
        return SavedWifi(list, network.ssid)
    }

    fun chose(ssid: String) = if (networks.any { it.ssid == ssid }) copy(last = ssid) else this

    fun removed(ssid: String): SavedWifi {
        val list = networks.filter { it.ssid != ssid }
        return SavedWifi(list, if (list.any { it.ssid == last }) last else list.firstOrNull()?.ssid)
    }

    fun json(): String = JSONObject()
        .put("networks", JSONArray(networks.map { JSONObject().put("ssid", it.ssid).put("password", it.password) }))
        .put("last", last ?: JSONObject.NULL)
        .toString()

    companion object {
        /** Reads [json], and the single network that versions up to 0.1.13 saved. */
        fun parse(text: String): SavedWifi {
            val json = JSONObject(text)
            if (!json.has("networks")) return SavedWifi().added(network(json))
            val list = json.getJSONArray("networks")
            return SavedWifi((0 until list.length()).map { network(list.getJSONObject(it)) },
                if (json.isNull("last")) null else json.getString("last"))
        }

        private fun network(json: JSONObject) = WifiNetwork(json.getString("ssid"), json.getString("password"))
    }
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
    fun save(saved: SavedWifi)
}

/**
 * Keeps the network encrypted with an AES-GCM key that never leaves the Android Keystore.
 * Preferences are excluded from backups (`data_extraction_rules`).
 */
class KeystoreWifiStore(context: Context) : WifiStore {
    private val prefs = context.getSharedPreferences("wifi", Context.MODE_PRIVATE)

    override fun load(): SavedWifi {
        val sealed = prefs.getString(KEY, null) ?: return SavedWifi()
        return try {
            val bytes = Base64.getDecoder().decode(sealed)
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, IV))
            SavedWifi.parse(cipher.doFinal(bytes, IV, bytes.size - IV).decodeToString())
        } catch (_: GeneralSecurityException) {
            SavedWifi()
        } catch (_: IllegalArgumentException) {
            SavedWifi()
        } catch (_: JSONException) {
            SavedWifi()
        }
    }

    override fun save(saved: SavedWifi) {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        val sealed = cipher.iv + cipher.doFinal(saved.json().toByteArray())
        prefs.edit().putString(KEY, Base64.getEncoder().encodeToString(sealed)).apply()
    }

    private fun key(): SecretKey {
        val store = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (store.getKey(ALIAS, null) as SecretKey?)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE).apply {
            init(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .build())
        }.generateKey()
    }

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "home-link-wifi"
        const val KEY = "network"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV = 12
    }
}
