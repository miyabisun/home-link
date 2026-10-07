package dev.miyabisun.homelink

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.wifi.WifiInfo
import android.net.wifi.WifiManager
import android.os.Handler
import android.os.Looper

/** The network this phone is on; [ssid] is null when Android hides it (no location permission or location off). */
data class CurrentWifi(val ssid: String?, val frequencyMhz: Int?)

fun interface WifiReader {
    /** Calls [done] once on the main thread, with null when the phone is not on Wi-Fi. */
    fun read(done: (CurrentWifi?) -> Unit)
}

/** Reads the connected Wi-Fi; the SSID needs the fine location permission and location turned on. */
class AndroidWifiReader(context: Context) : WifiReader {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)

    override fun read(done: (CurrentWifi?) -> Unit) {
        val active = connectivity.getNetworkCapabilities(connectivity.activeNetwork)
        if (active?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) != true) return done(null)
        val main = Handler(Looper.getMainLooper())
        var finished = false
        lateinit var callback: ConnectivityManager.NetworkCallback
        fun finish(wifi: CurrentWifi?) {
            if (finished) return
            finished = true
            connectivity.unregisterNetworkCallback(callback)
            done(wifi)
        }
        // Only a callback with this flag receives the SSID; the capabilities of activeNetwork have it redacted.
        callback = object : ConnectivityManager.NetworkCallback(FLAG_INCLUDE_LOCATION_INFO) {
            override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
                val info = capabilities.transportInfo as? WifiInfo ?: return
                val ssid = info.ssid.takeIf { it != WifiManager.UNKNOWN_SSID }?.removeSurrounding("\"")
                finish(CurrentWifi(ssid, info.frequency.takeIf { it > 0 }))
            }
        }
        connectivity.registerNetworkCallback(NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI).build(), callback, main)
        main.postDelayed({ finish(CurrentWifi(null, null)) }, 2_000)
    }
}
