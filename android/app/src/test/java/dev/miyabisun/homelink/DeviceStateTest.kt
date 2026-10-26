package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Test

class DeviceStateTest {
    private val device = Device(19, "物置き1", emptySet(), DeviceState.UNCONNECTED, null)

    @Test fun aDeviceThatLeftTheFabricReadsApartFromOneNeverConnected() {
        assertEquals("未接続", stateText(device))
        assertEquals("網から外れた", stateText(device.copy(leftFabric = true)))
        assertEquals("接続済み", stateText(device.copy(state = DeviceState.CONNECTED)))
        assertEquals("", stateText(device.copy(state = null)))
        assertEquals("物置き1", listTitle(device))
        assertEquals("物置き1（網から外れた）", listTitle(device.copy(leftFabric = true)))
    }
}
