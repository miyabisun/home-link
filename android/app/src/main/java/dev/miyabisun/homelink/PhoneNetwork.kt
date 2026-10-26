package dev.miyabisun.homelink

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network

/**
 * The phone's default network as Android gives it to this app, watched from first use for the life of the process.
 * Android reports no active network both when there is none and when it blocks this app's use of the one there is.
 */
class PhoneNetwork private constructor(context: Context) : PhoneLink {
    companion object {
        @Volatile private var instance: PhoneNetwork? = null

        fun of(context: Context): PhoneNetwork =
            instance ?: synchronized(this) { instance ?: PhoneNetwork(context.applicationContext).also { instance = it } }
    }

    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val lock = Object()
    /** The default network, and whether this app is blocked from it. */
    @Volatile private var current: Network? = null
    @Volatile private var blocked = false

    init {
        connectivity.registerDefaultNetworkCallback(object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = changed {
                current = network
                blocked = false
            }

            override fun onLost(network: Network) = changed {
                if (current == network) {
                    current = null
                    blocked = false
                }
            }

            override fun onBlockedStatusChanged(network: Network, blocked: Boolean) = changed {
                if (current == network) this@PhoneNetwork.blocked = blocked
            }
        })
    }

    private fun changed(update: () -> Unit) = synchronized(lock) {
        update()
        lock.notifyAll()
    }

    override fun state(): Link = when {
        connectivity.activeNetwork != null -> Link.USABLE
        current != null && blocked -> Link.BLOCKED
        else -> Link.NONE
    }

    override fun await(ms: Long) = synchronized(lock) { lock.wait(ms) }
}
