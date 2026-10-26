package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class RegisterFormTest {
    private val qr = "MT:Y.K9042C00KA0648G00"

    private fun device(id: Long, state: DeviceState? = DeviceState.UNCONNECTED, name: String = "",
                       code: String? = null) = Device(id, name, emptySet(), state, code)

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
        assertEquals(ApiError.UNREACHABLE, form.devicesError)
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
        form.commissioned(ApiResult.Ok(device(1, DeviceState.CONNECTED)))
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
        form.updated(ApiResult.Ok(device(1, name = "天井1")))
        assertEquals(device(1, name = "天井1", code = qr), form.device)
        assertEquals("天井1", form.unconnected.single().name)
        form.updated(ApiResult.Failed(ApiError.LABEL_NOT_FOUND))
        assertEquals(Status.Failed(ApiError.LABEL_NOT_FOUND), form.status)
        assertEquals("天井1", form.device?.name)
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

    @Test fun failedLedgerReadsKeepTheKnownDevicesAndSayWhy() {
        val form = RegisterForm()
        form.devicesRead(ApiResult.Ok(listOf(device(1))))
        form.devicesRead(ApiResult.Failed(ApiError.UNREACHABLE))
        assertEquals(listOf(1L), form.devices.map { it.id })
        // The ledger's failure is its own, apart from what the open device last did.
        assertEquals(ApiError.UNREACHABLE, form.devicesError)
        assertNull(form.status)
        // A successful read proves the server is reachable again.
        form.status = Status.Failed(ApiError.UNREACHABLE)
        form.devicesRead(ApiResult.Ok(listOf(device(1))))
        assertNull(form.devicesError)
        assertNull(form.status)

        form.status = Status.Failed(ApiError.WRONG_CODE)
        form.devicesRead(ApiResult.Ok(listOf(device(1))))
        assertEquals(Status.Failed(ApiError.WRONG_CODE), form.status)
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

    @Test fun theListOffersTheUnconnectedDevicesNotOpenAndNoneWhileConnecting() {
        val form = RegisterForm()
        form.devicesRead(ApiResult.Ok(listOf(device(1), device(2, DeviceState.CONNECTED), device(3), device(4))))
        assertEquals(listOf(1L, 3L, 4L), form.pickable.map { it.id })
        // The open device's pane connects it; the list does not show it again.
        form.opened(device(3))
        assertEquals(listOf(1L, 4L), form.pickable.map { it.id })
        // While one connects, no other can start.
        form.connect(3)
        assertEquals(emptyList<Device>(), form.pickable)
        form.bleFailed(BleProblem.BLUETOOTH_OFF)
        assertEquals(listOf(1L, 4L), form.pickable.map { it.id })
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
        assertEquals(listOf(plug.copy(on = true)), form.devices)
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

    @Test fun theFirstReadToFailSaysWhyUntilEveryReadSucceeds() {
        val form = RegisterForm()
        assertNull(form.readError)
        // Read side by side, they end in any order: the first failure shows at once.
        form.wifiRead(ApiResult.Failed(ApiError.CONNECT_TIMEOUT))
        assertEquals(ApiError.CONNECT_TIMEOUT, form.readError)
        form.devicesRead(ApiResult.Failed(ApiError.UNREACHABLE))
        form.labelsRead(ApiResult.Failed(ApiError.UNREACHABLE))
        assertEquals(ApiError.CONNECT_TIMEOUT, form.readError)
        form.wifiRead(ApiResult.Ok(emptyList()))
        assertEquals(ApiError.UNREACHABLE, form.readError)
        form.devicesRead(ApiResult.Ok(emptyList()))
        form.labelsRead(ApiResult.Ok(listOf(Label(1, "作業"))))
        assertNull(form.readError)
        assertEquals(listOf(Label(1, "作業")), form.labels)
        // A failed read keeps what was read.
        form.labelsRead(ApiResult.Failed(ApiError.TIMEOUT))
        assertEquals(listOf(Label(1, "作業")), form.labels)
    }

    @Test fun aDeviceJobRunsAloneSoNeitherConnectNorMoveIsOfferedDuringTheOther() {
        val form = RegisterForm()
        form.wifiRead(ApiResult.Ok(listOf(SavedNetwork(1, "iot-2g", forDevices = true))))
        form.opened(device(1))
        form.moving = 2
        assertFalse(form.canConnect())
        form.moving = null
        assertTrue(form.canConnect())
        form.opened(Device(2, "", emptySet(), DeviceState.CONNECTED, null, "home-5g"))
        assertTrue(form.canMoveWifi())
        form.connect(1)
        assertFalse(form.canMoveWifi())
    }

    private val started = 1_000_000L

    @Test fun aRegistrationAnEndedProcessLeftIsReadFromHomeLink() {
        val job = Unreported(DeviceJob.Commission(1), started)
        val now = started + 10_000
        val connected = device(1, DeviceState.CONNECTED, name = "押入れ1")
        assertEquals(Recovery.Settled(Status.Commissioned("押入れ1"), connected), recover(job, ApiResult.Ok(connected), null, now))
        assertEquals(Recovery.Settled(Status.Commissioned(""), device(1, DeviceState.NO_RESPONSE)),
            recover(job, ApiResult.Ok(device(1, DeviceState.NO_RESPONSE)), null, now))
        // Still unconnected: the proxy closed with the process, so the registration cannot have gone through.
        assertEquals(Recovery.Settled(Status.Interrupted(job.job, null), device(1)), recover(job, ApiResult.Ok(device(1)), null, now))
        // Gone from the ledger: home-link kept the row that already held the device.
        assertEquals(Recovery.Settled(Status.Commissioned(""), null), recover(job, ApiResult.Failed(ApiError.DEVICE_NOT_FOUND), null, now))
        // Not read: home-link, or matterjs-server behind it, is asked again later.
        assertEquals(Recovery.Later(Status.Failed(ApiError.UNREACHABLE)), recover(job, ApiResult.Failed(ApiError.UNREACHABLE), null, now))
        assertEquals(Recovery.Later(Status.Recovering(job.job)), recover(job, ApiResult.Ok(device(1, state = null)), null, now))
    }

    @Test fun aWifiMoveAnEndedProcessLeftWaitsForHomeLinkToFinishIt() {
        val job = Unreported(DeviceJob.MoveWifi(2), started)
        val moved = Device(2, "押入れ1", emptySet(), DeviceState.CONNECTED, null, "iot-2g")
        val stayed = moved.copy(wifiSsid = "home-5g")
        assertEquals(Recovery.Settled(Status.WifiMoved("iot-2g"), moved), recover(job, ApiResult.Ok(moved), "iot-2g", started + 10_000))
        // home-link moves the device alone, for up to the six minutes the request may take.
        assertEquals(Recovery.Later(Status.Recovering(job.job)), recover(job, ApiResult.Ok(stayed), "iot-2g", started + 10_000))
        assertEquals(Recovery.Later(Status.Recovering(job.job)), recover(job, ApiResult.Ok(moved), null, started + 10_000))
        assertEquals(Recovery.Settled(Status.Interrupted(job.job, "home-5g"), stayed),
            recover(job, ApiResult.Ok(stayed), "iot-2g", started + DEVICE_JOB_MS))
        assertEquals(Recovery.Later(Status.Failed(ApiError.TIMEOUT)), recover(job, ApiResult.Failed(ApiError.TIMEOUT), "iot-2g", started + 10_000))
    }
}
