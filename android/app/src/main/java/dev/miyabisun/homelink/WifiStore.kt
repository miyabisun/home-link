package dev.miyabisun.homelink

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import org.json.JSONException
import org.json.JSONObject
import java.security.GeneralSecurityException
import java.security.KeyStore
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** The Wi-Fi network to hand new devices, kept on this phone only. */
interface WifiStore {
    fun load(): WifiNetwork?
    fun save(network: WifiNetwork)
}

/**
 * Keeps the network encrypted with an AES-GCM key that never leaves the Android Keystore.
 * Preferences are excluded from backups (`data_extraction_rules`).
 */
class KeystoreWifiStore(context: Context) : WifiStore {
    private val prefs = context.getSharedPreferences("wifi", Context.MODE_PRIVATE)

    override fun load(): WifiNetwork? {
        val sealed = prefs.getString(KEY, null) ?: return null
        return try {
            val bytes = Base64.getDecoder().decode(sealed)
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, IV))
            val json = JSONObject(cipher.doFinal(bytes, IV, bytes.size - IV).decodeToString())
            WifiNetwork(json.getString("ssid"), json.getString("password"))
        } catch (_: GeneralSecurityException) {
            null
        } catch (_: IllegalArgumentException) {
            null
        } catch (_: JSONException) {
            null
        }
    }

    override fun save(network: WifiNetwork) {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        val plain = JSONObject().put("ssid", network.ssid).put("password", network.password).toString()
        val sealed = cipher.iv + cipher.doFinal(plain.toByteArray())
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
