package dev.miyabisun.homelink

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.os.ParcelFileDescriptor
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import android.widget.Button
import android.widget.EditText
import android.widget.RadioButton
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.io.FileInputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Semaphore
import java.util.concurrent.TimeUnit

@RunWith(AndroidJUnit4::class)
class RegisterScreenTest {
    private val qr = "MT:Y.K9042C00KA0648G00"
    private val api = FakeApi()
    private val scanner = FakeScanner()
    private val link = FakeLink()
    private val wifi = MemoryWifiStore()
    private val reader = FakeWifiReader()

    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val endpointsFile get() = File(Endpoints.path(context))

    @Before fun install() {
        // Messages name the host from the endpoints file, as on a phone set up with `adb push`.
        assertTrue(Endpoints.save(context, Endpoints("http://home-link.example:5011", "ws://192.0.2.10:5580/ble")))
        MainActivity.apiFactory = { api }
        MainActivity.scannerFactory = { scanner }
        MainActivity.linkFactory = { link }
        MainActivity.wifiStoreFactory = { wifi }
        SettingsActivity.wifiReaderFactory = { reader }
    }

    @After fun uninstall() {
        MainActivity.apiFactory = null
        MainActivity.scannerFactory = null
        MainActivity.linkFactory = null
        MainActivity.wifiStoreFactory = null
        SettingsActivity.wifiReaderFactory = null
        // A connection a failed test left running would carry over to the next screen.
        CommissionSession.cancel()
        endpointsFile.delete()
    }

    private val homeWifi = SavedNetwork(1, "home-2g", forDevices = true)

    private val qrs = listOf("MT:Y.K9042C00084-0Z800", "MT:Y.K9042C00SIWN1WH00", "MT:Y.K9042C00ITMB2TQ00")

    @Test fun aReadCodeEntersTheLedgerAndOpensTheDevice() {
        api.rooms += Room(1, "寝室")
        api.wifi += homeWifi
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { hasLabel(it, "まだ読み取っていません") }
            screen.onActivity { activity ->
                assertNoGuides(activity)
                assertNoWifiInputs(activity)
                // One way in: no Wi-Fi or Thread tabs, no device pane until a code is read.
                assertFalse(views(activity).any { it is RadioButton })
                assertFalse(field(activity, "機器名（任意）").isShown)
                assertFalse(views(activity).any { it is Button && it.isShown && it.text == "接続" })
            }
            capture(screen, "empty")

            scanner.next = ScanResult.Read(qr)
            screen.onActivity { button(it, "QRを読み取る").performClick() }
            eventually(screen) { hasLabel(it, "読み取り済み：3497-011-2332") && hasLabel(it, "未接続") }
            assertEquals(listOf(qr), api.adds.toList())
            assertEquals(1, api.ledger.size)
            screen.onActivity { activity ->
                assertEquals("", field(activity, "機器名（任意）").text.toString())
                assertEquals("未設定", spinner(activity, "部屋").selectedItem)
                assertTrue(button(activity, "接続").isEnabled)
            }
            capture(screen, "opened")

            // The same code again opens the same device; home-link keeps one row.
            screen.onActivity { activity ->
                field(activity, "機器名（任意）").setText("天井1")
                field(activity, "機器名（任意）").onEditorAction(EditorInfo.IME_ACTION_DONE)
            }
            eventually(screen) { api.ledger.single().name == "天井1" }
            screen.onActivity { button(it, "QRを読み取る").performClick() }
            eventually(screen) { api.adds.size == 2 }
            InstrumentationRegistry.getInstrumentation().waitForIdleSync()
            assertEquals(1, api.ledger.size)
            screen.onActivity { activity ->
                assertTrue(hasLabel(activity, "読み取り済み：3497-011-2332"))
                assertEquals("天井1", field(activity, "機器名（任意）").text.toString())
            }

            // A payload for several devices has no single printed code, so it shows whole.
            scanner.next = ScanResult.Read("MT:Y.K9042C00KA0648G00*Y.K9042C00KA0648G00")
            screen.onActivity { button(it, "QRを読み取る").performClick() }
            eventually(screen) {
                hasLabel(it, "読み取り済み（ラベルの数字に直せないコード）：MT:Y.K9042C00KA0648G00*Y.K9042C00KA0648G00")
            }
            screen.onActivity { assertEquals("", field(it, "機器名（任意）").text.toString()) }
            capture(screen, "qr-undecodable")

            // Anything but a Matter code is not sent and keeps the open device.
            scanner.next = ScanResult.Read("https://example.com/")
            screen.onActivity { activity ->
                button(activity, "QRを読み取る").performClick()
                assertTrue(hasLabel(activity, "MatterのQRコードではありません。QRコードが無い機器は「数字で入力」を使ってください"))
            }
            assertEquals(3, api.adds.size)
            capture(screen, "not-matter")
        }
    }

    @Test fun manualCodeIsTypedCheckedAndEntered() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { hasLabel(it, "まだ読み取っていません") }
            screen.onActivity { button(it, "数字で入力").performClick() }
            eventually(screen) { field(it, "11桁の数字").hasFocus() }
            instrumentation.waitForIdleSync()
            instrumentation.sendStringSync("3497011")
            eventually(screen) { field(it, "11桁の数字").text.toString() == "3497 011" }
            capture(screen, "manual-typing")

            screen.recreate()
            eventually(screen) { field(it, "11桁の数字").isShown }
            screen.onActivity { activity ->
                assertEquals("3497 011", field(activity, "11桁の数字").text.toString())
                field(activity, "11桁の数字").onEditorAction(EditorInfo.IME_ACTION_DONE)
                assertTrue(hasLabel(activity, "11桁の数字を入力してください（あと4桁）"))
            }
            capture(screen, "manual-length")

            screen.onActivity { field(it, "11桁の数字").requestFocus() }
            instrumentation.waitForIdleSync()
            instrumentation.sendStringSync("2331")
            eventually(screen) { hasLabel(it, "数字が正しくありません。機器に印字された数字と見比べてください") }
            screen.onActivity { assertEquals("3497 011 2331", field(it, "11桁の数字").text.toString()) }
            capture(screen, "manual-check-digit")
            assertEquals(emptyList<String>(), api.adds.toList())

            screen.onActivity { activity ->
                field(activity, "11桁の数字").setText("3497-011-233")
                assertEquals("3497 011 233", field(activity, "11桁の数字").text.toString())
                field(activity, "11桁の数字").requestFocus()
                field(activity, "11桁の数字").setSelection(12)
            }
            instrumentation.waitForIdleSync()
            instrumentation.sendStringSync("2")
            eventually(screen) { hasLabel(it, "入力済み：3497-011-2332") && hasLabel(it, "未接続") }
            assertEquals(listOf("34970112332"), api.adds.toList())
            screen.onActivity { activity ->
                assertFalse(field(activity, "11桁の数字").isShown)
                assertTrue(button(activity, "数字で入力").isShown)
            }
            capture(screen, "manual-entered")
        }
    }

    @Test fun threeReadDevicesAreListedOnTheNextLaunchAndConnectOneByOne() {
        api.wifi += homeWifi
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            for ((i, code) in qrs.withIndex()) {
                scanner.next = ScanResult.Read(code)
                screen.onActivity { button(it, "QRを読み取る").performClick() }
                eventually(screen) { api.ledger.size == i + 1 && hasLabel(it, "読み取り済み：${ManualCode.label(code)}") }
            }
        }
        val titles = qrs.map { ManualCode.label(it)!! }
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { listed(it) == titles }
            screen.onActivity { activity ->
                assertTrue(hasLabel(activity, "未接続"))
                assertTrue(hasLabel(activity, "まだ読み取っていません"))
            }
            capture(screen, "unconnected-three")

            for ((i, title) in titles.withIndex()) {
                link.script = listOf("start_scan", "connect", "write_and_subscribe")
                screen.onActivity { button(it, title).performClick() }
                eventually(screen) { hasLabel(it, "機器を探しています…") }
                screen.onActivity { activity ->
                    // The picked device opens; nothing else connects meanwhile.
                    assertTrue(hasLabel(activity, "読み取り済み：$title"))
                    assertTrue(button(activity, "キャンセル").isEnabled)
                    assertFalse(views(activity).any { it is Button && it.isShown && it.text == "接続" })
                    assertTrue(listed(activity).isNotEmpty())
                    assertTrue(views(activity).filterIsInstance<Button>().filter { it.isShown && it.text in titles }.none { it.isEnabled })
                }
                if (i == 0) capture(screen, "unconnected-searching")
                repeat(3) { link.step() }
                eventually(screen) { hasLabel(it, "接続しました") && listed(it) == titles.drop(i + 1) }
                screen.onActivity { assertTrue(hasLabel(it, "接続済み")) }
                if (i == 0) capture(screen, "unconnected-connected")
            }
            assertEquals(api.ledger.map { it.id }, api.commissionings.toList())
            screen.onActivity { assertFalse(hasLabel(it, "未接続")) }
            capture(screen, "unconnected-none")
        }
    }

    @Test fun aFailedDeviceStaysListedAndConnectsOnTheNextPress() {
        api.wifi += homeWifi
        api.ledger += listOf(
            Device(1, "天井1", null, emptySet(), DeviceState.UNCONNECTED, qrs[0]),
            Device(2, "読書灯", 1, emptySet(), DeviceState.CONNECTED, qr),
        )
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { listed(it) == listOf("天井1") }
            api.failure = ApiError.DEVICE_NOT_FOUND
            screen.onActivity { button(it, "天井1").performClick() }
            eventually(screen) {
                hasLabel(it, "機器が見つかりませんでした。機器をペアリング待ちにし、電話を近づけてから「接続」を押し直してください")
            }
            screen.onActivity { activity ->
                assertEquals(listOf("天井1"), listed(activity))
                assertTrue(hasLabel(activity, "未接続"))
                assertTrue(button(activity, "接続").isEnabled)
            }
            capture(screen, "connect-failed")

            // The device was powered again: the same row connects without a restart or another read.
            api.failure = null
            screen.onActivity { button(it, "天井1").performClick() }
            eventually(screen) { hasLabel(it, "「天井1」を接続しました") && listed(it).isEmpty() }
            assertEquals(listOf(1L, 1L), api.commissionings.toList())
            assertEquals(listOf(qrs[0]), api.ledger.filter { it.state == DeviceState.CONNECTED && it.id == 1L }.map { it.code })
        }
    }

    @Test fun theNameTheRoomAndTheLabelsReachHomeLink() {
        api.rooms += listOf(Room(1, "寝室"), Room(2, "リビングと続きの和室（南側・大きな窓のある部屋）"))
        api.labels += listOf(Label(1, "作業"), Label(2, "夜"))
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            scanner.next = ScanResult.Read(qr)
            eventually(screen) { hasLabel(it, "まだ読み取っていません") }
            screen.onActivity { button(it, "QRを読み取る").performClick() }
            eventually(screen) { spinner(it, "部屋").isShown && spinner(it, "部屋").count == 3 }
            val id = api.ledger.single().id
            screen.onActivity { activity ->
                field(activity, "機器名（任意）").setText("押入れ1")
                field(activity, "機器名（任意）").onEditorAction(EditorInfo.IME_ACTION_DONE)
                spinner(activity, "部屋").setSelection(2)
            }
            eventually(screen) { api.ledger.single().roomId == 2L }
            screen.onActivity { activity ->
                checkBox(activity, "夜").performClick()
            }
            eventually(screen) { api.ledger.single().labelIds == setOf(2L) }
            screen.onActivity { checkBox(it, "作業").performClick() }
            eventually(screen) { api.ledger.single().labelIds == setOf(1L, 2L) }
            screen.onActivity { checkBox(it, "夜").performClick() }
            eventually(screen) { api.ledger.single().labelIds == setOf(1L) }
            assertEquals("押入れ1", api.ledger.single().name)
            assertEquals(listOf(DeviceUpdate.Name("押入れ1"), DeviceUpdate.Room(2), DeviceUpdate.Labels(setOf(2)),
                DeviceUpdate.Labels(setOf(1, 2)), DeviceUpdate.Labels(setOf(1))), api.updates.filter { it.first == id }.map { it.second })
            capture(screen, "edited")

            // A new room is made in the pane and given to the device; no room takes it out again.
            screen.onActivity { activity ->
                button(activity, "部屋を追加").performClick()
                field(activity, "新しい部屋の名前").setText("書斎")
            }
            capture(screen, "add-room")
            screen.onActivity { button(it, "追加").performClick() }
            eventually(screen) { hasLabel(it, "部屋「書斎」を追加しました") && spinner(it, "部屋").selectedItem == "書斎" }
            eventually(screen) { api.ledger.single().roomId == 3L }
            screen.onActivity { spinner(it, "部屋").setSelection(0) }
            eventually(screen) { api.ledger.single().roomId == null }

            // A recreated screen reopens the device with its values.
            screen.recreate()
            eventually(screen) { field(it, "機器名（任意）").isShown && checkBox(it, "作業").isChecked }
            screen.onActivity { activity ->
                assertEquals("押入れ1", field(activity, "機器名（任意）").text.toString())
                assertEquals("未設定", spinner(activity, "部屋").selectedItem)
                assertFalse(checkBox(activity, "夜").isChecked)
                assertTrue(hasLabel(activity, "読み取り済み：3497-011-2332"))
            }
            // Drawing the reopened device sends nothing.
            assertEquals(listOf(DeviceUpdate.Room(3), DeviceUpdate.Room(null)), api.updates.drop(5).map { it.second })
        }
    }

    @Test fun unreachableServerOnLaunchOffersReconnect() {
        api.devicesFailure = ApiError.UNREACHABLE
        api.roomsFailure = ApiError.UNREACHABLE
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { button(it, "再接続").isShown }
            scanner.next = ScanResult.Failed
            screen.onActivity { activity ->
                button(activity, "QRを読み取る").performClick()
                assertTrue(hasLabel(activity, "QRコードを読み取れませんでした。もう一度お試しください"))
            }
            capture(screen, "offline")
            api.devicesFailure = null
            api.roomsFailure = null
            api.ledger += Device(1, "天井1", null, emptySet(), DeviceState.UNCONNECTED, qr)
            screen.onActivity { button(it, "再接続").performClick() }
            eventually(screen) { activity ->
                listed(activity) == listOf("天井1") && views(activity).none { it is Button && it.text == "再接続" && it.isShown }
            }
        }
    }

    @Test fun connectingShowsEachStageAndSurvivesRecreation() {
        api.ledger += Device(1, "押入れ1", null, emptySet(), DeviceState.UNCONNECTED, qr)
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { listed(it) == listOf("押入れ1") }
            // No Wi-Fi on home-link for devices yet: it is saved once in the settings, and Thread devices need none.
            val settings = InstrumentationRegistry.getInstrumentation().addMonitor(SettingsActivity::class.java.name, null, true)
            screen.onActivity { activity ->
                assertTrue(hasLabel(activity, "機器に渡すWi-Fi"))
                assertNoWifiInputs(activity)
                button(activity, "Wi-Fiを設定").performClick()
            }
            assertEquals(1, settings.hits)
            InstrumentationRegistry.getInstrumentation().removeMonitor(settings)
            capture(screen, "bt-wifi")
            api.wifi += homeWifi
            screen.recreate()
            eventually(screen) { !hasLabel(it, "機器に渡すWi-Fi") && listed(it) == listOf("押入れ1") }

            link.script = listOf("start_scan", "stop_scan", "connect", "discover_services", "write_and_subscribe", "disconnect")
            val gate = CountDownLatch(1)
            api.gate = gate
            screen.onActivity { button(it, "押入れ1").performClick() }
            for ((text, name) in listOf("機器を探しています…" to "bt-searching", "機器に接続しています…" to "bt-connecting",
                    "機器にコードと設定を送っています…" to "bt-sending")) {
                eventually(screen) { hasLabel(it, text) }
                screen.onActivity { activity ->
                    assertFalse(views(activity).any { it is Button && it.isShown && it.text == "接続" })
                    assertTrue(button(activity, "キャンセル").isEnabled)
                }
                capture(screen, name)
                if (name == "bt-sending") {
                    // A recreated screen follows the same connection.
                    screen.recreate()
                    eventually(screen) { hasLabel(it, text) && hasLabel(it, "読み取り済み：3497-011-2332") }
                }
                link.step()
                link.step()
            }
            // matterjs-server also disconnects after a failed handshake, so the stage stays.
            eventually(screen) { link.played == 6 }
            InstrumentationRegistry.getInstrumentation().waitForIdleSync()
            screen.onActivity { assertTrue(hasLabel(it, "機器にコードと設定を送っています…")) }
            gate.countDown()
            eventually(screen) { hasLabel(it, "「押入れ1」を接続しました") && hasLabel(it, "接続済み") }
            assertEquals(listOf(1L), api.commissionings.toList())
            assertTrue(link.closed)
            capture(screen, "bt-connected")
        }
    }

    @Test fun cancellingReturnsToConnectingAtOnceAndDropsTheLateAnswer() {
        api.wifi += homeWifi
        api.ledger += Device(1, "押入れ1", null, emptySet(), DeviceState.UNCONNECTED, qr)
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { listed(it) == listOf("押入れ1") }
            val gate = CountDownLatch(1)
            api.gate = gate
            screen.onActivity { button(it, "押入れ1").performClick() }
            eventually(screen) { hasLabel(it, "matterjs-serverに接続しています…") }
            capture(screen, "bt-cancel")
            screen.onActivity { activity ->
                button(activity, "キャンセル").performClick()
                assertFalse(hasLabel(activity, "matterjs-serverに接続しています…"))
                assertFalse(views(activity).any { it is Button && it.isShown && it.text == "キャンセル" })
                assertTrue(button(activity, "接続").isEnabled)
                assertTrue(button(activity, "押入れ1").isEnabled)
            }
            assertTrue(link.closed)
            capture(screen, "bt-cancelled")

            // home-link answering later changes nothing on the screen.
            api.commissioned = ApiResult.Failed(ApiError.COMMISSION_FAILED)
            gate.countDown()
            eventually(screen) { api.commissionings.size == 1 }
            Thread.sleep(500)
            InstrumentationRegistry.getInstrumentation().waitForIdleSync()
            screen.onActivity { activity ->
                assertFalse(views(activity).filterIsInstance<TextView>().any { it.isShown && it.text.startsWith("接続が途中で止まりました") })
                assertTrue(button(activity, "接続").isEnabled)
            }

            // Connecting again runs as usual.
            api.gate = null
            api.commissioned = null
            screen.onActivity { button(it, "接続").performClick() }
            eventually(screen) { hasLabel(it, "「押入れ1」を接続しました") }
        }
    }

    @Test fun connectionFailuresSayWhatToDoAndKeepTheDevice() {
        api.wifi += homeWifi
        api.ledger += Device(1, "押入れ1", null, emptySet(), DeviceState.UNCONNECTED, qr)
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { listed(it) == listOf("押入れ1") }
            screen.onActivity { button(it, "押入れ1").performClick() }
            eventually(screen) { hasLabel(it, "「押入れ1」を接続しました") }
            api.ledger[0] = api.ledger[0].copy(state = DeviceState.UNCONNECTED)
            screen.recreate()
            eventually(screen) { button(it, "接続").isShown }
            val cases = listOf(
                ApiError.DEVICE_NOT_FOUND to "機器が見つかりませんでした。機器をペアリング待ちにし、電話を近づけてから「接続」を押し直してください",
                ApiError.WRONG_CODE to "機器がコードを受け付けませんでした。機器のコードを読み取り直すか、入力し直してください",
                ApiError.WIFI_FAILED to "機器がWi-Fi「home-2g」につながりませんでした。Wi-Fiの名前とパスワードを確かめてください。2.4GHzにしか対応しない機器もあります",
                ApiError.THREAD_FAILED to "機器がThread網につながりませんでした。Thread網（OTBR）が動いているか確かめてから、もう一度お試しください",
                ApiError.COMMISSION_TIMEOUT to "時間内に接続が終わりませんでした。機器をペアリング待ちにし直して、もう一度お試しください",
                ApiError.COMMISSION_FAILED to "接続が途中で止まりました。電話を機器の近くに置いて、「接続」を押し直してください。" +
                    "機器の接続待ちが終わっていたら、機器の電源を1回入れ直してから押し直してください",
                ApiError.BLUETOOTH_UNAVAILABLE to "matterjs-serverでBluetoothの中継が有効になっていません。サーバーの設定を確かめてください",
            )
            for ((error, text) in cases) {
                api.failure = error
                screen.onActivity { button(it, "接続").performClick() }
                eventually(screen) { hasLabel(it, text) && button(it, "接続").isEnabled }
                screen.onActivity { assertEquals(listOf("押入れ1"), listed(it)) }
                if (error in listOf(ApiError.DEVICE_NOT_FOUND, ApiError.WIFI_FAILED, ApiError.COMMISSION_FAILED)) capture(screen, "bt-${error.name.lowercase()}")
            }

            Endpoints.save(context, Endpoints("http://home-link.example:5011", ""))
            screen.onActivity { button(it, "接続").performClick() }
            eventually(screen) { hasLabel(it, "matterjs-serverのBLE Proxyの接続先が未設定です。「接続先の設定」で入れてください") }
            Endpoints.save(context, Endpoints("http://home-link.example:5011", "ws://192.0.2.10:5580/ble"))

            link.opens = ProxyOpen.UNREACHABLE
            screen.onActivity { button(it, "接続").performClick() }
            eventually(screen) {
                hasLabel(it, "matterjs-server（192.0.2.10:5580）に接続できません。家のWi-Fiにつないでから、もう一度お試しください")
            }
            capture(screen, "bt-proxy-unreachable")
            // An unreachable proxy never reaches the API.
            assertEquals(1 + cases.size, api.commissionings.size)
        }
    }

    @Test fun okHttpSendsThePatchOnAndroid() {
        if (android.os.Build.VERSION.SDK_INT >= 37) InstrumentationRegistry.getInstrumentation().uiAutomation
            .grantRuntimePermission(context.packageName, android.Manifest.permission.ACCESS_LOCAL_NETWORK)
        FakeHomeLink().use { server ->
            HttpHomeLinkApi(server.url).updateDevice(9, DeviceUpdate.Room(null))
            assertEquals(listOf("PATCH /api/devices/9"), server.requests.toList())
        }
    }

    @Test fun withoutEndpointsNothingIsSentAndTheSettingsAreOffered() {
        MainActivity.apiFactory = null
        endpointsFile.delete()
        val settings = InstrumentationRegistry.getInstrumentation().addMonitor(SettingsActivity::class.java.name, null, true)
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) {
                hasLabel(it, "接続先が未設定です。home-linkのURLを設定してください")
            }
            capture(screen, "no-endpoints")
            screen.onActivity { button(it, "照明オン").performClick() }
            eventually(screen) { hasLabel(it, "接続先が未設定です。アプリの「接続先の設定」でhome-linkのURLを入れてください") }
            screen.onActivity { button(it, "接続先を設定").performClick() }
            assertEquals(1, settings.hits)
        }
        InstrumentationRegistry.getInstrumentation().removeMonitor(settings)
    }

    @Test fun theSettingsWriteTheYamlAndTheNextRequestUsesIt() {
        MainActivity.apiFactory = null
        endpointsFile.delete()
        // Saving asks Android 17 for the local network; a dialog left open would pause the screen.
        if (android.os.Build.VERSION.SDK_INT >= 37) InstrumentationRegistry.getInstrumentation().uiAutomation
            .grantRuntimePermission(context.packageName, android.Manifest.permission.ACCESS_LOCAL_NETWORK)
        FakeHomeLink().use { server ->
            ActivityScenario.launch(SettingsActivity::class.java).use { screen ->
                eventually(screen) { hasLabel(it, "home-linkのURLを入れてください（例：http://home-link.example:5011）") }
                screen.onActivity { activity ->
                    field(activity, "home-linkのURL").setText("home-link.example:5011")
                    field(activity, "BLE ProxyのURL").setText("http://192.0.2.10:5580/ble")
                    button(activity, "保存").performClick()
                    assertTrue(hasLabel(activity, "http://かhttps://で始まるURLを入れてください（例：http://home-link.example:5011）"))
                    assertTrue(hasLabel(activity, "ws://かwss://で始まるURLを入れてください（例：ws://192.0.2.10:5580/ble）"))
                    assertTrue(hasLabel(activity, "URLを直してから保存してください"))
                }
                assertFalse(endpointsFile.exists())
                capture(screen, "settings-invalid")
                screen.onActivity { activity ->
                    field(activity, "home-linkのURL").setText(server.url)
                    field(activity, "BLE ProxyのURL").setText("ws://192.0.2.10:5580/ble")
                    button(activity, "保存").performClick()
                    assertTrue(hasLabel(activity, "保存しました。次の要求からこの接続先へ送ります"))
                }
                assertEquals(Endpoints(server.url, "ws://192.0.2.10:5580/ble"), Endpoints.parse(endpointsFile.readText()))
                screen.recreate()
                screen.onActivity { assertEquals(server.url, field(it, "home-linkのURL").text.toString()) }
                capture(screen, "settings")
            }
            ActivityScenario.launch(MainActivity::class.java).use { screen ->
                eventually(screen) { "GET /api/devices" in server.requests }
                screen.onActivity { assertFalse(hasLabel(it, "接続先が未設定です。home-linkのURLを設定してください")) }
            }
            assertTrue(server.requests.toString(), "GET /api/rooms" in server.requests)
        }
    }

    @Test fun switchAllLightsAndShowTheResult() {
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { hasLabel(it, "まだ読み取っていません") }
            screen.onActivity { activity ->
                // Offered until a widget is placed, when the launcher can pin one.
                val manager = AppWidgetManager.getInstance(activity)
                val placed = manager.getAppWidgetIds(ComponentName(activity, LightsWidget::class.java)).isNotEmpty()
                assertEquals(manager.isRequestPinAppWidgetSupported && !placed,
                    views(activity).any { it is Button && it.isShown && it.text == "ホーム画面にボタンを置く" })
            }
            capture(screen, "lights")

            val gate = CountDownLatch(1)
            api.gate = gate
            api.lights = ApiResult.Ok(LightsResult(11, 0, 0, emptyList()))
            screen.onActivity { button(it, "照明オン").performClick() }
            eventually(screen) { hasLabel(it, "照明をオンにしています…") }
            screen.onActivity { activity ->
                assertFalse(button(activity, "照明オン").isEnabled)
                assertFalse(button(activity, "照明オフ").isEnabled)
            }
            capture(screen, "lights-busy")
            gate.countDown()
            eventually(screen) { hasLabel(it, "照明を11台オンにしました") }
            screen.onActivity { assertTrue(button(it, "照明オフ").isEnabled) }
            assertEquals(listOf(true), api.switches)
            capture(screen, "lights-on")

            api.lights = ApiResult.Ok(LightsResult(8, 2, 1, listOf("キッチン側の天井灯（リビング・南の窓寄り）", "通路")))
            screen.onActivity { button(it, "照明オフ").performClick() }
            val partial = "照明を8台オフにしました。応答なし2台・失敗1台・見つからない2台（キッチン側の天井灯（リビング・南の窓寄り）、通路）"
            eventually(screen) { hasLabel(it, partial) }
            assertEquals(listOf(true, false), api.switches)
            capture(screen, "lights-partial")
            screen.recreate()
            eventually(screen) { hasLabel(it, partial) }

            api.lights = ApiResult.Failed(ApiError.UNREACHABLE)
            screen.onActivity { button(it, "照明オン").performClick() }
            eventually(screen) { hasLabel(it, "home-link（home-link.example:5011）に接続できません。接続先の設定と通信を確かめてください") }
            capture(screen, "lights-unreachable")
        }
    }


    @Test fun theWifiForDevicesIsSavedChosenAndDeletedInTheSettings() {
        val home = SavedNetwork(1, "home-2g", forDevices = false)
        val annex = SavedNetwork(2, "離れの2.4GHz（物置と作業部屋）", forDevices = true)
        api.wifi += listOf(home, annex)
        ActivityScenario.launch(SettingsActivity::class.java).use { screen ->
            // The network for devices is chosen already.
            eventually(screen) { spinner(it, "電球に渡すWi-Fi").isShown }
            screen.onActivity { activity ->
                val choice = spinner(activity, "電球に渡すWi-Fi")
                assertEquals(annex.ssid, choice.selectedItem)
                assertEquals(listOf(home.ssid, annex.ssid), (0 until choice.count).map { choice.getItemAtPosition(it) })
            }
            capture(screen, "wifi-list")

            // Choosing another network makes it the one for devices, also across a recreated screen.
            screen.onActivity { spinner(it, "電球に渡すWi-Fi").setSelection(0) }
            eventually(screen) { api.wifi.single { w -> w.forDevices }.ssid == home.ssid }
            screen.recreate()
            eventually(screen) { spinner(it, "電球に渡すWi-Fi").selectedItem == home.ssid }

            // The adder fills in the network the phone is on; only the password is left.
            reader.next = CurrentWifi("home-2g", 2437)
            screen.onActivity { activity ->
                button(activity, "Wi-Fiを追加").performClick()
                assertEquals("home-2g", field(activity, "Wi-Fiの名前（SSID）").text.toString())
                assertTrue(hasLabel(activity, "今つながっているWi-Fiの名前を入れました。パスワードを入力してください"))
                assertTrue(field(activity, "Wi-Fiのパスワード").isFocused)
                field(activity, "Wi-Fiのパスワード").setText("x".repeat(65))
                wifiSave(activity).performClick()
                assertTrue(hasLabel(activity, "Wi-Fiの名前（32バイトまで）とパスワード（64文字まで）を入力してください"))
                button(activity, "やめる").performClick()
            }

            // On 5GHz, the adder says the bulbs need 2.4GHz.
            reader.next = CurrentWifi("home-5g", 5180)
            screen.onActivity { button(it, "Wi-Fiを追加").performClick() }
            screen.onActivity { activity ->
                assertEquals("home-5g", field(activity, "Wi-Fiの名前（SSID）").text.toString())
                assertTrue(hasLabel(activity, "今つながっている「home-5g」は5GHzです。電球は2.4GHzにしかつながりません。" +
                    "ルーターが2.4GHzを別の名前で出している場合は、その名前に直してください"))
                assertFalse(spinner(activity, "電球に渡すWi-Fi").isShown)
            }
            capture(screen, "wifi-5ghz")
            // The adder survives a recreated screen with the typed name but never the password.
            screen.onActivity { activity ->
                field(activity, "Wi-Fiの名前（SSID）").setText("home-2g")
                field(activity, "Wi-Fiのパスワード").setText("new-pass")
            }
            screen.recreate()
            eventually(screen) { field(it, "Wi-Fiの名前（SSID）").isShown }
            screen.onActivity { activity ->
                assertEquals("home-2g", field(activity, "Wi-Fiの名前（SSID）").text.toString())
                assertEquals("", field(activity, "Wi-Fiのパスワード").text.toString())
                // home-link replaces the password of the same name instead of adding a second entry.
                field(activity, "Wi-Fiのパスワード").setText("new-pass")
                wifiSave(activity).performClick()
            }
            eventually(screen) { !field(it, "Wi-Fiのパスワード").isShown && spinner(it, "電球に渡すWi-Fi").isShown }
            assertEquals(WifiNetwork("home-2g", "new-pass"), api.wifiSaves.last())
            screen.onActivity { assertEquals(2, spinner(it, "電球に渡すWi-Fi").count) }

            // Without the name of the network, the adder says why and leaves the name to type.
            for ((current, text) in listOf(
                    null to "Wi-Fiにつながっていません。電球に渡すWi-Fiの名前を入力してください",
                    CurrentWifi(null, 2437) to "今つながっているWi-Fiの名前を読めませんでした。位置情報がオンか確かめるか、名前を入力してください")) {
                reader.next = current
                screen.onActivity { activity ->
                    button(activity, "Wi-Fiを追加").performClick()
                    assertTrue(hasLabel(activity, text))
                    assertEquals("", field(activity, "Wi-Fiの名前（SSID）").text.toString())
                    assertFalse(wifiSave(activity).isEnabled)
                    button(activity, "やめる").performClick()
                    assertTrue(spinner(activity, "電球に渡すWi-Fi").isShown)
                }
            }

            // A save home-link refuses keeps the adder open with the reason.
            api.wifiFailure = ApiError.UNREACHABLE
            screen.onActivity { activity ->
                button(activity, "Wi-Fiを追加").performClick()
                field(activity, "Wi-Fiの名前（SSID）").setText("guest-2g")
                field(activity, "Wi-Fiのパスワード").setText("guest-pass")
                wifiSave(activity).performClick()
            }
            val unreachable = "home-link（home-link.example:5011）に接続できません。接続先の設定と通信を確かめてください"
            eventually(screen) { hasLabel(it, unreachable) && field(it, "Wi-Fiのパスワード").isShown }
            capture(screen, "wifi-save-failed")
            api.wifiFailure = null
            screen.onActivity { button(it, "やめる").performClick() }

            // Deleting the network for devices hands devices the one left, then none.
            eventually(screen) { spinner(it, "電球に渡すWi-Fi").isShown }
            screen.onActivity { button(it, "このWi-Fiを削除").performClick() }
            eventually(screen) { hasLabel(it, "「home-2g」を削除しました") && spinner(it, "電球に渡すWi-Fi").count == 1 }
            screen.onActivity { assertEquals(annex.ssid, spinner(it, "電球に渡すWi-Fi").selectedItem) }
            capture(screen, "wifi-deleted")
            screen.onActivity { button(it, "このWi-Fiを削除").performClick() }
            eventually(screen) { hasLabel(it, "まだ保存していません") }
            screen.onActivity { activity ->
                assertFalse(spinner(activity, "電球に渡すWi-Fi").isShown)
                assertFalse(views(activity).filterIsInstance<Button>().any { it.isShown && it.text == "このWi-Fiを削除" })
            }
            assertEquals(emptyList<SavedNetwork>(), api.wifi.toList())
            capture(screen, "wifi-empty")

            // A failed read says why and offers to read again.
            api.wifiFailure = ApiError.UNREACHABLE
            screen.recreate()
            eventually(screen) { hasLabel(it, "Wi-Fiを読み込めませんでした") && hasLabel(it, unreachable) }
            screen.onActivity { assertFalse(views(it).filterIsInstance<Button>().any { b -> b.isShown && b.text == "Wi-Fiを追加" }) }
            capture(screen, "wifi-offline")
            api.wifiFailure = null
            screen.onActivity { button(it, "再接続").performClick() }
            eventually(screen) { hasLabel(it, "まだ保存していません") && !hasLabel(it, unreachable) }
        }
    }

    @Test fun wifiSavedOnThePhoneMovesToHomeLinkOnce() {
        val home = WifiNetwork("home-2g", "kakushi")
        val annex = WifiNetwork("離れの2.4GHz", "hanare")
        val saved = SavedWifi(listOf(home, annex), last = annex.ssid)
        wifi.saved = saved
        // home-link cannot be reached at first: the phone keeps its networks for the next launch.
        api.wifiFailure = ApiError.UNREACHABLE
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { hasLabel(it, "まだ読み取っていません") }
            InstrumentationRegistry.getInstrumentation().waitForIdleSync()
            assertEquals(saved, wifi.saved)
            assertEquals(emptyList<WifiNetwork>(), api.wifiSaves.toList())
        }
        api.wifiFailure = null
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            // The chosen network goes first and becomes the one for devices.
            eventually(screen) { wifi.saved == SavedWifi() }
            assertEquals(listOf(annex, home), api.wifiSaves.toList())
            assertEquals(annex.ssid, api.wifi.single { it.forDevices }.ssid)
            eventually(screen) { !hasLabel(it, "機器に渡すWi-Fi") }
            screen.onActivity { assertNoWifiInputs(it) }
            capture(screen, "wifi-moved")
        }
        // Moved once: nothing is sent again.
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { hasLabel(it, "まだ読み取っていません") }
            InstrumentationRegistry.getInstrumentation().waitForIdleSync()
            assertEquals(2, api.wifiSaves.size)
        }
    }

    @Test fun aConnectedDeviceMovesOntoTheWifiForDevicesWithOneButton() {
        api.wifi += listOf(SavedNetwork(1, "home-2g", forDevices = false), SavedNetwork(2, "iot-2g", forDevices = true))
        api.ledger += Device(1, "押入れ1", null, emptySet(), DeviceState.CONNECTED, qrs[0], "home-2g")
        api.ledger += Device(2, "天井1", null, emptySet(), DeviceState.CONNECTED, qrs[1], "iot-2g")
        val move = "「iot-2g」へ移す"
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { hasLabel(it, "まだ読み取っていません") }
            // A device already on the Wi-Fi for devices has nothing to move.
            scanner.next = ScanResult.Read(qrs[1])
            screen.onActivity { button(it, "QRを読み取る").performClick() }
            eventually(screen) { hasLabel(it, "接続済み") && field(it, "機器名（任意）").text.toString() == "天井1" }
            screen.onActivity { assertFalse(views(it).any { v -> v is Button && v.isShown && v.text == move }) }

            scanner.next = ScanResult.Read(qrs[0])
            screen.onActivity { button(it, "QRを読み取る").performClick() }
            eventually(screen) { field(it, "機器名（任意）").text.toString() == "押入れ1" && button(it, move).isEnabled }
            screen.onActivity { activity ->
                assertNoWifiInputs(activity)
                assertFalse(views(activity).any { it is Button && it.isShown && it.text == "接続" })
            }
            capture(screen, "move-ready")

            // The move runs on across a recreated screen and cannot be pressed twice.
            val gate = CountDownLatch(1)
            api.moveGate = gate
            screen.onActivity { button(it, move).performClick() }
            eventually(screen) { hasLabel(it, "Wi-Fiを移しています…") }
            screen.onActivity { assertFalse(views(it).any { v -> v is Button && v.isShown && v.text == move }) }
            capture(screen, "move-busy")
            screen.recreate()
            eventually(screen) { hasLabel(it, "Wi-Fiを移しています…") && field(it, "機器名（任意）").text.toString() == "押入れ1" }
            gate.countDown()
            eventually(screen) { hasLabel(it, "「iot-2g」へ移しました") }
            screen.onActivity { assertFalse(views(it).any { v -> v is Button && v.isShown && v.text == move }) }
            assertEquals(listOf(1L), api.moves.toList())
            capture(screen, "move-done")
        }
        api.moveGate = null
        for ((failure, text, state) in listOf(
            Triple(ApiError.WIFI_REVERTED, "「iot-2g」につながらず、元のWi-Fiに戻りました", "接続済み"),
            Triple(ApiError.WIFI_REVERTING, "「iot-2g」につながらず、応答がありません。元のWi-Fiへ戻ります", "応答なし"),
        )) {
            api.ledger[0] = api.ledger[0].copy(wifiSsid = "home-2g", state = DeviceState.CONNECTED)
            api.moveFailure = failure
            ActivityScenario.launch(MainActivity::class.java).use { screen ->
                scanner.next = ScanResult.Read(qrs[0])
                eventually(screen) { hasLabel(it, "まだ読み取っていません") }
                screen.onActivity { button(it, "QRを読み取る").performClick() }
                eventually(screen) { button(it, move).isEnabled }
                screen.onActivity { button(it, move).performClick() }
                // The device's state, read again, tells whether it is still usable; a usable one may be moved again.
                eventually(screen) {
                    hasLabel(it, text) && hasLabel(it, state) &&
                        views(it).any { v -> v is Button && v.isShown && v.text == move } == (state == "接続済み")
                }
                capture(screen, "move-${failure.name.lowercase()}")
            }
        }
    }

    /** Registering neither chooses nor types a Wi-Fi. */
    private fun assertNoWifiInputs(activity: Activity) {
        assertFalse(views(activity).any { it.isShown && it is Spinner && it.contentDescription == "電球に渡すWi-Fi" })
        assertFalse(views(activity).any { it.isShown && it is EditText && it.contentDescription.startsWith("Wi-Fi") })
    }

    /** The adder's save button, below the settings' own. */
    private fun wifiSave(activity: Activity) = views(activity).filterIsInstance<Button>().last { it.isShown && it.text == "保存" }

    /** The removed ledger-only choice and its guide texts are gone from the screen. */
    private fun assertNoGuides(activity: Activity) {
        val texts = views(activity).filterIsInstance<TextView>().map { it.text.toString() }
        for (gone in listOf("つなぎ方", "記録だけ", "T2など", "BEAMTEC", "Zigbee", "メーカーの手順")) {
            assertFalse(gone, texts.any { gone in it })
        }
    }

    private fun <T : Activity> eventually(screen: ActivityScenario<T>, condition: (T) -> Boolean) {
        val deadline = System.currentTimeMillis() + 5_000
        while (true) {
            var met = false
            screen.onActivity { met = condition(it) }
            if (met) return
            if (System.currentTimeMillis() >= deadline) {
                var shown = ""
                screen.onActivity { activity ->
                    shown = views(activity).filterIsInstance<TextView>().filter { it.isShown }.joinToString(" | ") { it.text }
                }
                throw AssertionError("condition not met in time; shown: $shown")
            }
            Thread.sleep(50)
        }
    }

    /** Saves a screenshot and the text layout, asserting no clipped text and 48dp targets. */
    private fun <T : Activity> capture(screen: ActivityScenario<T>, name: String) {
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        val prefix = InstrumentationRegistry.getArguments().getString("capturePrefix", "")
        screen.onActivity { activity ->
            val metrics = JSONArray()
            val minimum = (48 * activity.resources.displayMetrics.density).toInt()
            for (view in views(activity).filterIsInstance<TextView>().filter { it.isShown }) {
                val layout = view.layout ?: continue
                for (line in 0 until layout.lineCount) {
                    assertEquals("Ellipsis: ${view.text}", 0, layout.getEllipsisCount(line))
                    assertTrue("Clipped: ${view.text}", layout.getLineWidth(line) <=
                        view.width - view.compoundPaddingLeft - view.compoundPaddingRight + 1)
                }
                if (view is Button || view is EditText) assertTrue("Small target: ${view.text}", view.height >= minimum)
                val location = IntArray(2).also(view::getLocationOnScreen)
                metrics.put(JSONObject().put("text", view.text.toString()).put("x", location[0])
                    .put("y", location[1]).put("width", view.width).put("height", view.height))
            }
            val root = activity.window.decorView.width
            assertTrue("No horizontal overflow", views(activity).filter { it.isShown }.all {
                val location = IntArray(2).also(it::getLocationOnScreen)
                location[0] >= 0 && location[0] + it.width <= root
            })
            write("/data/local/tmp/$prefix$name.json", metrics.toString())
        }
        shell("screencap -p /data/local/tmp/$prefix$name.png")
        screen.onActivity { activity ->
            val scroll = views(activity).filterIsInstance<ScrollView>().single()
            scroll.scrollTo(0, scroll.getChildAt(0).height)
        }
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        screen.onActivity { activity ->
            val register = views(activity).filterIsInstance<Button>().last { it.isShown }
            val bounds = android.graphics.Rect()
            assertTrue("Register reachable", register.getGlobalVisibleRect(bounds))
            assertEquals(register.height, bounds.height())
        }
        shell("screencap -p /data/local/tmp/$prefix$name-bottom.png")
        screen.onActivity { views(it).filterIsInstance<ScrollView>().single().scrollTo(0, 0) }
    }

    private fun write(path: String, text: String) {
        val pipes = InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommandRw("tee $path")
        ParcelFileDescriptor.AutoCloseOutputStream(pipes[1]).use { it.write(text.toByteArray()) }
        ParcelFileDescriptor.AutoCloseInputStream(pipes[0]).use { it.readBytes() }
    }

    private fun shell(command: String) {
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand(command)
            .use { FileInputStream(it.fileDescriptor).readBytes() }
    }

    private fun views(activity: Activity) = descendants(activity.window.decorView)
    private fun descendants(view: View): List<View> = listOf(view) +
        if (view is ViewGroup) (0 until view.childCount).flatMap { descendants(view.getChildAt(it)) } else emptyList()
    private fun hasLabel(activity: Activity, text: String) =
        views(activity).filterIsInstance<TextView>().any { it.isShown && it.text.toString() == text }
    private fun button(activity: Activity, text: String) =
        views(activity).filterIsInstance<Button>().first { it.isShown && it.text.toString() == text }
    private fun spinner(activity: Activity, description: String) =
        views(activity).filterIsInstance<Spinner>().first { it.contentDescription == description }
    private fun field(activity: Activity, description: String) =
        views(activity).filterIsInstance<EditText>().first { it.contentDescription == description }
    /** The titles of the devices listed to connect, top to bottom. */
    private fun listed(activity: Activity): List<String> {
        // The list's heading leads its panel; the open device's state reads the same.
        val heading = views(activity).filterIsInstance<TextView>().firstOrNull {
            it.text == "未接続" && it.isShown && (it.parent as ViewGroup).getChildAt(0) == it
        } ?: return emptyList()
        val list = (heading.parent as ViewGroup).getChildAt(1) as ViewGroup
        return (0 until list.childCount).map { (list.getChildAt(it) as Button).text.toString() }
    }
    private fun checkBox(activity: Activity, text: String) =
        views(activity).filterIsInstance<android.widget.CheckBox>().first { it.isShown && it.text.toString() == text }
}

private class FakeApi : HomeLinkApi {
    val rooms: MutableList<Room> = Collections.synchronizedList(mutableListOf())
    @Volatile var roomsFailure: ApiError? = null
    @Volatile var lights: ApiResult<LightsResult> = ApiResult.Ok(LightsResult(0, 0, 0, emptyList()))
    @Volatile var gate: CountDownLatch? = null
    val switches: MutableList<Boolean> = Collections.synchronizedList(mutableListOf())

    override fun rooms(): ApiResult<List<Room>> =
        roomsFailure?.let { ApiResult.Failed(it) } ?: ApiResult.Ok(rooms.toList())

    override fun createRoom(name: String): ApiResult<Room> {
        if (rooms.any { it.name == name }) return ApiResult.Failed(ApiError.DUPLICATE_ROOM)
        return Room(rooms.size + 1L, name).also { rooms += it }.let { ApiResult.Ok(it) }
    }

    override fun switchLights(on: Boolean): ApiResult<LightsResult> {
        gate?.await()
        switches += on
        return lights
    }

    override fun toggleBoost(label: String): ApiResult<BoostResult> = ApiResult.Failed(ApiError.SERVER)
    override fun boostedLabels(): ApiResult<Set<String>> = ApiResult.Failed(ApiError.SERVER)
    override fun door(command: DoorCommand): ApiResult<Unit> = ApiResult.Failed(ApiError.SERVER)

    /** home-link's ledger, codes included, kept the way home-link keeps it. */
    val ledger: MutableList<Device> = Collections.synchronizedList(mutableListOf())
    val adds: MutableList<String> = Collections.synchronizedList(mutableListOf())
    val updates: MutableList<Pair<Long, DeviceUpdate>> = Collections.synchronizedList(mutableListOf())
    val labels: MutableList<Label> = Collections.synchronizedList(mutableListOf())
    @Volatile var devicesFailure: ApiError? = null

    override fun devices(): ApiResult<List<Device>> =
        devicesFailure?.let { ApiResult.Failed(it) } ?: ApiResult.Ok(synchronized(ledger) { ledger.map { it.copy(code = null) } })

    override fun device(id: Long): ApiResult<Device> =
        ledger.toList().firstOrNull { it.id == id }?.let { ApiResult.Ok(it) } ?: ApiResult.Failed(ApiError.DEVICE_NOT_FOUND)

    override fun addDevice(payload: String): ApiResult<Device> = synchronized(ledger) {
        adds += payload
        val device = ledger.firstOrNull { it.code == payload }
            ?: Device((ledger.maxOfOrNull { it.id } ?: 0) + 1, "", null, emptySet(), DeviceState.UNCONNECTED, payload).also { ledger += it }
        ApiResult.Ok(device.copy(code = null))
    }

    override fun updateDevice(id: Long, update: DeviceUpdate): ApiResult<Device> = synchronized(ledger) {
        updates += id to update
        val index = ledger.indexOfFirst { it.id == id }
        val device = ledger[index]
        ledger[index] = when (update) {
            is DeviceUpdate.Name -> device.copy(name = update.name)
            is DeviceUpdate.Room -> device.copy(roomId = update.id)
            is DeviceUpdate.Labels -> device.copy(labelIds = update.ids)
        }
        ApiResult.Ok(ledger[index].copy(code = null))
    }

    override fun labels(): ApiResult<List<Label>> = ApiResult.Ok(labels.toList())

    val commissionings: MutableList<Long> = Collections.synchronizedList(mutableListOf())
    @Volatile var failure: ApiError? = null
    /** Null connects the device. */
    @Volatile var commissioned: ApiResult<Device>? = null

    override fun commission(id: Long): ApiResult<Device> {
        gate?.await()
        commissionings += id
        failure?.let { return ApiResult.Failed(it) }
        commissioned?.let { return it }
        return synchronized(ledger) {
            val index = ledger.indexOfFirst { it.id == id }
            ledger[index] = ledger[index].copy(state = DeviceState.CONNECTED)
            ApiResult.Ok(ledger[index].copy(code = null))
        }
    }

    val moves: MutableList<Long> = Collections.synchronizedList(mutableListOf())
    @Volatile var moveGate: CountDownLatch? = null
    @Volatile var moveFailure: ApiError? = null

    override fun moveWifi(id: Long): ApiResult<Device> {
        moveGate?.await()
        moves += id
        return synchronized(ledger) {
            val index = ledger.indexOfFirst { it.id == id }
            when (moveFailure) {
                null -> ledger[index] = ledger[index].copy(wifiSsid = wifi.first { it.forDevices }.ssid)
                ApiError.WIFI_REVERTING -> ledger[index] = ledger[index].copy(state = DeviceState.NO_RESPONSE)
                else -> Unit
            }
            moveFailure?.let { ApiResult.Failed(it) } ?: ApiResult.Ok(ledger[index].copy(code = null))
        }
    }

    /** home-link's saved networks, kept the way home-link keeps them. */
    val wifi: MutableList<SavedNetwork> = Collections.synchronizedList(mutableListOf())
    val wifiSaves: MutableList<WifiNetwork> = Collections.synchronizedList(mutableListOf())
    @Volatile var wifiFailure: ApiError? = null

    override fun wifiNetworks(): ApiResult<List<SavedNetwork>> =
        wifiFailure?.let { ApiResult.Failed(it) } ?: ApiResult.Ok(wifi.toList())

    override fun saveWifi(network: WifiNetwork): ApiResult<SavedNetwork> = synchronized(wifi) {
        wifiFailure?.let { return ApiResult.Failed(it) }
        wifiSaves += network
        val saved = wifi.firstOrNull { it.ssid == network.ssid }
            ?: SavedNetwork((wifi.maxOfOrNull { it.id } ?: 0) + 1, network.ssid, wifi.none { it.forDevices }).also { wifi += it }
        ApiResult.Ok(saved)
    }

    override fun chooseWifi(id: Long): ApiResult<SavedNetwork> = synchronized(wifi) {
        wifiFailure?.let { return ApiResult.Failed(it) }
        wifi.replaceAll { it.copy(forDevices = it.id == id) }
        ApiResult.Ok(wifi.first { it.id == id })
    }

    override fun deleteWifi(id: Long): ApiResult<Unit> = synchronized(wifi) {
        wifiFailure?.let { return ApiResult.Failed(it) }
        val gone = wifi.first { it.id == id }
        wifi.remove(gone)
        if (gone.forDevices && wifi.isNotEmpty()) wifi[0] = wifi[0].copy(forDevices = true)
        ApiResult.Ok(Unit)
    }
}

/** Plays proxy commands one [step] at a time, as matterjs-server would send them. */
private class FakeLink : BleLink {
    @Volatile var script: List<String> = emptyList()
    @Volatile var opens = ProxyOpen.OK
    @Volatile var closed = false
    /** How many commands the screen has heard. */
    @Volatile var played = 0
    private val steps = Semaphore(0)

    fun step() = steps.release()

    override fun open(onCommand: (String) -> Unit): ProxyOpen {
        closed = false
        played = 0
        for (command in script) {
            onCommand(command)
            played++
            if (!steps.tryAcquire(10, TimeUnit.SECONDS)) error("test did not step past $command")
        }
        script = emptyList()
        return opens
    }

    override fun close() { closed = true }
}

private class MemoryWifiStore : WifiStore {
    @Volatile var saved = SavedWifi()
    override fun load() = saved
    override fun clear() { saved = SavedWifi() }
}

private class FakeWifiReader : WifiReader {
    @Volatile var next: CurrentWifi? = CurrentWifi("home-2g", 2437)
    override fun read(done: (CurrentWifi?) -> Unit) = done(next)
}

private class FakeScanner : QrScanner {
    var next: ScanResult = ScanResult.Cancelled
    override fun scan(done: (ScanResult) -> Unit) = done(next)
}

/** A home-link on loopback that answers every request with an empty list and keeps each request line. */
private class FakeHomeLink : AutoCloseable {
    private val socket = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
    val url = "http://127.0.0.1:${socket.localPort}"
    val requests: MutableList<String> = Collections.synchronizedList(mutableListOf())

    init {
        Thread {
            while (!socket.isClosed) {
                val client = try { socket.accept() } catch (_: java.io.IOException) { break }
                client.use {
                    val reader = it.getInputStream().bufferedReader()
                    val line = reader.readLine().orEmpty()
                    while (reader.readLine()?.isNotEmpty() == true) Unit
                    requests += line.substringBeforeLast(" ")
                    it.getOutputStream().write(("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n" +
                        "content-length: 2\r\nconnection: close\r\n\r\n[]").toByteArray())
                }
            }
        }.start()
    }

    override fun close() = socket.close()
}
