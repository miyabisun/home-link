package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Test

class BluetoothPermissionsTest {
    private val bluetooth = listOf("android.permission.BLUETOOTH_SCAN", "android.permission.BLUETOOTH_CONNECT")

    @Test
    fun android16AsksOnlyForBluetooth() {
        // Android 16 does not know ACCESS_LOCAL_NETWORK and would deny it, failing the request.
        assertEquals(bluetooth, bluetoothPermissions(36).toList())
    }

    @Test
    fun android17AlsoAsksForTheLocalNetwork() {
        assertEquals(bluetooth + "android.permission.ACCESS_LOCAL_NETWORK", bluetoothPermissions(37).toList())
    }
}
