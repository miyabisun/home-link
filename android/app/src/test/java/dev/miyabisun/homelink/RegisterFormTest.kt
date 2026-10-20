package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class RegisterFormTest {
    private val qr = "MT:Y.K9042C00KA0648G00"

    private fun device(id: Long, state: DeviceState? = DeviceState.UNCONNECTED, name: String = "", roomId: Long? = null,
                       code: String? = null) = Device(id, name, roomId, emptySet(), state, code)

    @Test fun scanningANonMatterCodeSendsNothing() {
        val form = RegisterForm()
        assertEquals(qr, form.scanned(" $qr\n"))
        assertNull(form.status)
        assertNull(form.scanned("https://example.com/"))
        assertEquals(Status.NotMatter, form.status)
        assertEquals(qr, form.scanned(qr))
        assertNull(form.status)
    }

    @Test fun aReadCodeOpensTheDeviceHomeLinkAnswersWith() {
        val form = RegisterForm()
        form.devicesRead(ApiResult.Ok(listOf(device(1, DeviceState.CONNECTED))))
        // New or already in the ledger, the answer is the device to edit.
        form.added(qr, ApiResult.Ok(device(2)))
        assertEquals(device(2, code = qr), form.device)
        assertEquals(listOf(device(2, code = qr)), form.unconnected)
        form.added(qr, ApiResult.Ok(device(2)))
        assertEquals(listOf(device(1, DeviceState.CONNECTED), device(2, code = qr)), form.devices)

        // A refused code keeps the open device.
        form.added("34970112331", ApiResult.Failed(ApiError.INVALID_MANUAL))
        assertEquals(2L, form.device?.id)
        assertEquals(Status.Failed(ApiError.INVALID_MANUAL), form.status)
    }

    @Test fun onlyUnconnectedDevicesAreListedInTheLedgersOrder() {
        val form = RegisterForm()
        form.devicesRead(ApiResult.Ok(listOf(device(1), device(2, DeviceState.CONNECTED), device(3, DeviceState.NO_RESPONSE),
            device(4, null), device(5))))
        assertEquals(listOf(1L, 5L), form.unconnected.map { it.id })
        // A failed read keeps the last list.
        form.devicesRead(ApiResult.Failed(ApiError.UNREACHABLE))
        assertEquals(listOf(1L, 5L), form.unconnected.map { it.id })
        assertTrue(form.devicesFailed)
    }

    @Test fun rereadingTheLedgerKeepsTheCodesAlreadyRead() {
        val form = RegisterForm()
        form.added(qr, ApiResult.Ok(device(1)))
        form.devicesRead(ApiResult.Ok(listOf(device(1, name = "天井1"), device(2))))
        assertEquals(device(1, name = "天井1", code = qr), form.device)
        form.detailRead(ApiResult.Ok(device(2, code = "34970112332")))
        assertEquals(listOf(qr, "34970112332"), form.unconnected.map { it.code })
        // The open device leaves the pane once home-link no longer holds it.
        form.devicesRead(ApiResult.Ok(listOf(device(2))))
        assertNull(form.device)
    }

    @Test fun statesParseFromTheApiAndUnknownIsNone() {
        assertEquals(DeviceState.UNCONNECTED, DeviceState.of("unconnected"))
        assertEquals(DeviceState.CONNECTED, DeviceState.of("connected"))
        assertEquals(DeviceState.NO_RESPONSE, DeviceState.of("no_response"))
        assertNull(DeviceState.of(null))
        assertNull(DeviceState.of("other"))
    }

    @Test fun onlyAnUnconnectedDeviceCanBeConnectedAndOneAtATime() {
        val form = RegisterForm()
        assertFalse(form.canConnect())
        form.opened(device(1, DeviceState.CONNECTED))
        assertFalse(form.canConnect())
        form.opened(device(2))
        assertTrue(form.canConnect())
        form.connect(2)
        assertTrue(form.busy)
        assertEquals(Stage.PREPARING, form.stage)
        assertFalse(form.canConnect())
    }

    @Test fun aConnectedDeviceLeavesTheListAndItsPaneShowsItConnected() {
        val form = RegisterForm()
        form.devicesRead(ApiResult.Ok(listOf(device(1), device(2), device(3))))
        form.opened(device(1, code = qr))
        form.connect(1)
        form.commissioned(ApiResult.Ok(device(1, DeviceState.CONNECTED, roomId = 2)))
        assertFalse(form.busy)
        assertNull(form.stage)
        assertEquals(listOf(2L, 3L), form.unconnected.map { it.id })
        assertEquals(DeviceState.CONNECTED, form.device?.state)
        assertEquals(qr, form.device?.code)
        assertEquals(Status.Commissioned(""), form.status)
    }

    @Test fun aRowMergedIntoAnotherIsReplacedByIt() {
        val form = RegisterForm()
        form.devicesRead(ApiResult.Ok(listOf(device(1, DeviceState.CONNECTED, name = "読書灯"), device(4))))
        form.opened(device(4, code = qr))
        form.connect(4)
        // home-link keeps the row already holding the device's identifiers and drops the read one.
        form.commissioned(ApiResult.Ok(device(1, DeviceState.CONNECTED, name = "読書灯")))
        assertEquals(listOf(1L), form.devices.map { it.id })
        assertEquals(1L, form.device?.id)
        assertEquals(Status.Commissioned("読書灯"), form.status)
    }

    @Test fun aFailedConnectionKeepsTheDeviceListedToTryAgain() {
        val form = RegisterForm()
        form.devicesRead(ApiResult.Ok(listOf(device(1), device(2))))
        form.opened(device(1))
        form.connect(1)
        form.stage = Stage.SENDING
        form.commissioned(ApiResult.Failed(ApiError.WIFI_FAILED))
        assertEquals(Status.Failed(ApiError.WIFI_FAILED), form.status)
        assertFalse(form.busy)
        assertNull(form.stage)
        assertEquals(listOf(1L, 2L), form.unconnected.map { it.id })
        assertTrue(form.canConnect())

        form.connect(1)
        form.bleFailed(BleProblem.BLUETOOTH_OFF)
        assertEquals(Status.BleFailed(BleProblem.BLUETOOTH_OFF), form.status)
        assertTrue(form.canConnect())
    }

    @Test fun cancellingReturnsToConnectingAtOnce() {
        val form = RegisterForm()
        form.opened(device(1))
        form.connect(1)
        form.stage = Stage.SENDING
        form.cancelled()
        assertFalse(form.busy)
        assertNull(form.stage)
        assertNull(form.status)
        assertTrue(form.canConnect())
    }

    @Test fun editsReplaceTheDeviceInThePaneAndTheList() {
        val form = RegisterForm()
        form.devicesRead(ApiResult.Ok(listOf(device(1))))
        form.added(qr, ApiResult.Ok(device(1)))
        form.updated(ApiResult.Ok(device(1, name = "天井1", roomId = 3)))
        assertEquals(device(1, name = "天井1", roomId = 3, code = qr), form.device)
        assertEquals("天井1", form.unconnected.single().name)
        form.updated(ApiResult.Failed(ApiError.ROOM_NOT_FOUND))
        assertEquals(Status.Failed(ApiError.ROOM_NOT_FOUND), form.status)
        assertEquals(3L, form.device?.roomId)
    }

    @Test fun aValidManualCodeIsTakenOnceAllDigitsAreTyped() {
        val form = RegisterForm()
        assertNull(form.typed("3497 011 233"))
        assertNull(form.manualError)
        assertEquals("34970112332", form.typed("3497 011 2332"))
    }

    @Test fun manualCodeErrorsShowAtOnceAndClearWhileEditing() {
        val form = RegisterForm()
        assertNull(form.typed("3497 011 2331"))
        assertEquals(ManualError.CHECK_DIGIT, form.manualError)
        form.typed("3497 011 233")
        assertNull(form.manualError)
        assertNull(form.submitted("3497 011 233"))
        assertEquals(ManualError.LENGTH, form.manualError)
        assertNull(form.submitted("3497 011 2331"))
        assertEquals(ManualError.CHECK_DIGIT, form.manualError)
        assertEquals("34970112332", form.submitted("3497 011 2332"))
        assertNull(form.manualError)
    }

    @Test fun failedRoomLoadsKeepTheKnownRooms() {
        val form = RegisterForm()
        form.rooms(ApiResult.Ok(listOf(Room(1, "寝室"))))
        form.rooms(ApiResult.Failed(ApiError.UNREACHABLE))
        assertEquals(listOf(Room(1, "寝室")), form.rooms)
        assertTrue(form.roomsFailed)
        assertEquals(Status.Failed(ApiError.UNREACHABLE), form.status)
        form.rooms(ApiResult.Ok(listOf(Room(1, "寝室"))))
        assertFalse(form.roomsFailed)
        assertNull(form.status)

        form.status = Status.Failed(ApiError.WRONG_CODE)
        form.rooms(ApiResult.Ok(listOf(Room(1, "寝室"))))
        assertEquals(Status.Failed(ApiError.WRONG_CODE), form.status)
    }

    @Test fun createdRoomsAreAdded() {
        val form = RegisterForm()
        form.rooms(ApiResult.Ok(listOf(Room(1, "寝室"))))
        form.roomCreated(ApiResult.Ok(Room(5, "書斎")))
        assertEquals(listOf(Room(1, "寝室"), Room(5, "書斎")), form.rooms)
        assertEquals(Status.RoomAdded("書斎"), form.status)
        form.roomCreated(ApiResult.Failed(ApiError.DUPLICATE_ROOM))
        assertEquals(Status.Failed(ApiError.DUPLICATE_ROOM), form.status)
    }

    @Test fun theWifiForDevicesIsMissingOnlyWhenHomeLinkHoldsNone() {
        val form = RegisterForm()
        assertFalse(form.wifiMissing)
        form.wifiRead(ApiResult.Ok(listOf(SavedNetwork(1, "home-2g", false))))
        assertTrue(form.wifiMissing)
        form.wifiRead(ApiResult.Ok(listOf(SavedNetwork(1, "home-2g", false), SavedNetwork(2, "iot-2g", true))))
        assertEquals("iot-2g", form.deviceWifi)
        assertFalse(form.wifiMissing)
        // A failed read keeps what was read last.
        form.wifiRead(ApiResult.Failed(ApiError.UNREACHABLE))
        assertEquals("iot-2g", form.deviceWifi)
    }

    @Test fun proxyCommandsMarkTheStages() {
        assertEquals(Stage.SEARCHING, Stage.of("start_scan"))
        assertEquals(Stage.CONNECTING, Stage.of("connect"))
        assertEquals(Stage.SENDING, Stage.of("write_and_subscribe"))
        // matterjs-server also disconnects when the handshake failed, so it marks no stage.
        for (other in listOf("stop_scan", "discover_services", "read_characteristic", "request_mtu", "disconnect")) {
            assertNull(other, Stage.of(other))
        }
    }

    @Test fun onlyAConnectedDeviceOffTheWifiForDevicesCanMoveAndOneAtATime() {
        val form = RegisterForm()
        val onHome = device(1, DeviceState.CONNECTED).copy(wifiSsid = "home-2g")
        form.opened(onHome)
        // Nothing to move onto until home-link's Wi-Fi for devices is read.
        assertFalse(form.canMoveWifi())
        form.wifiRead(ApiResult.Ok(listOf(SavedNetwork(1, "home-2g", false), SavedNetwork(2, "iot-2g", true))))
        assertTrue(form.canMoveWifi())
        form.moving = 1
        assertFalse(form.canMoveWifi())
        form.moving = null
        for (other in listOf(onHome.copy(wifiSsid = "iot-2g"), onHome.copy(wifiSsid = null), onHome.copy(state = DeviceState.NO_RESPONSE),
                device(2).copy(wifiSsid = "home-2g"))) {
            form.opened(other)
            assertFalse("$other", form.canMoveWifi())
        }
    }

    @Test fun aMovedDeviceShowsItsNewWifiAndAFailureKeepsTheDevice() {
        val form = RegisterForm()
        val onHome = device(1, DeviceState.CONNECTED).copy(wifiSsid = "home-2g", code = qr)
        form.opened(onHome)
        form.wifiMoved(ApiResult.Ok(onHome.copy(wifiSsid = "iot-2g", code = null)))
        assertEquals(Status.WifiMoved("iot-2g"), form.status)
        assertEquals(onHome.copy(wifiSsid = "iot-2g"), form.device)
        form.wifiMoved(ApiResult.Failed(ApiError.WIFI_REVERTED))
        assertEquals(Status.Failed(ApiError.WIFI_REVERTED), form.status)
        assertEquals("iot-2g", form.device?.wifiSsid)
    }

    @Test fun connectedDevicesAreListedInTheLedgersOrder() {
        val form = RegisterForm()
        form.devicesRead(ApiResult.Ok(listOf(device(1), device(2, DeviceState.CONNECTED), device(3, DeviceState.NO_RESPONSE),
            device(4, null), device(5, DeviceState.CONNECTED))))
        assertEquals(listOf(2L, 3L, 5L), form.connected.map { it.id })
    }

    @Test fun aPlugSwitchesToTheOtherSideAndShowsItsAnswer() {
        val form = RegisterForm()
        val plug = device(1, DeviceState.CONNECTED).copy(kind = DeviceKind.PLUG, on = false)
        form.devicesRead(ApiResult.Ok(listOf(plug)))
        form.opened(plug)
        assertTrue(form.nextOn(plug))
        assertTrue(form.nextOn(plug.copy(on = null)))
        assertFalse(form.nextOn(plug.copy(on = true)))

        form.switchStarted(1)
        assertEquals(setOf(1L), form.switching)
        form.switched(1, ApiResult.Ok(true))
        assertEquals(plug.copy(on = true), form.device)
        assertEquals(listOf(plug.copy(on = true)), form.connected)
        assertEquals(emptySet<Long>(), form.switching)

        // No answer keeps the last On/Off read; an answer again brings the plug back.
        form.switched(1, ApiResult.Ok(null))
        assertEquals(plug.copy(on = true, state = DeviceState.NO_RESPONSE), form.device)
        form.switched(1, ApiResult.Ok(false))
        assertEquals(plug, form.device)

        form.switchStarted(1)
        form.switched(1, ApiResult.Failed(ApiError.UNREACHABLE))
        assertEquals(Status.Failed(ApiError.UNREACHABLE), form.status)
        assertEquals(plug, form.device)
        assertEquals(emptySet<Long>(), form.switching)
    }
}
