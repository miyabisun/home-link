package dev.miyabisun.homelink

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
import java.io.FileInputStream
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

    @Before fun install() {
        MainActivity.apiFactory = { api }
        MainActivity.scannerFactory = { scanner }
        MainActivity.linkFactory = { link }
        MainActivity.wifiStoreFactory = { wifi }
        MainActivity.wifiReaderFactory = { reader }
    }

    @After fun uninstall() {
        MainActivity.apiFactory = null
        MainActivity.scannerFactory = null
        MainActivity.linkFactory = null
        MainActivity.wifiStoreFactory = null
        MainActivity.wifiReaderFactory = null
    }

    /** The ledger-only registration the earlier tests exercise. */
    private fun recordOnly(screen: ActivityScenario<MainActivity>) = screen.onActivity { activity ->
        views(activity).filterIsInstance<RadioButton>().first { it.text == "記録だけ（つながっている機器）" }.performClick()
        assertFalse(hasLabel(activity, "電球に渡すWi-Fi"))
    }

    @Test fun createSelectAndRegister() {
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            recordOnly(screen)
            eventually(screen) { hasLabel(it, "部屋がありません。「部屋を追加」から作成してください") }
            screen.onActivity { assertFalse(button(it, "登録").isEnabled) }
            capture(screen, "empty")

            for (room in listOf("寝室", "リビングと続きの和室（南側・大きな窓のある部屋）")) {
                screen.onActivity { activity ->
                    button(activity, "部屋を追加").performClick()
                    field(activity, "新しい部屋の名前").setText(room)
                }
                if (room == "寝室") capture(screen, "add-room")
                screen.onActivity { button(it, "追加").performClick() }
                eventually(screen) { hasLabel(it, "部屋「$room」を追加しました") }
            }
            screen.onActivity { activity ->
                val spinner = spinner(activity, "部屋")
                assertEquals("リビングと続きの和室（南側・大きな窓のある部屋）", spinner.selectedItem)
                spinner.setSelection(0)
            }
            eventually(screen) { spinner(it, "部屋").selectedItem == "寝室" }

            scanner.next = ScanResult.Read(qr)
            screen.onActivity { activity ->
                button(activity, "QRを読み取る").performClick()
                assertTrue(hasLabel(activity, "読み取り済み：MT:Y.K90…"))
                assertFalse(views(activity).filterIsInstance<TextView>().any { qr in it.text })
                field(activity, "機器名（任意）").setText("天井灯")
                assertTrue(button(activity, "登録").isEnabled)
            }
            capture(screen, "ready")
            screen.onActivity { button(it, "登録").performClick() }
            eventually(screen) { hasLabel(it, "寝室に「天井灯」を登録しました") }
            assertEquals(listOf(Registration(1, qr, "天井灯")), api.registrations)
            screen.onActivity { activity ->
                assertTrue(hasLabel(activity, "まだ読み取っていません"))
                assertEquals("", field(activity, "機器名（任意）").text.toString())
                assertEquals("寝室", spinner(activity, "部屋").selectedItem)
                assertFalse(button(activity, "登録").isEnabled)
            }
            capture(screen, "registered")

            // The name is optional: a second bulb in the same room needs only a scan.
            scanner.next = ScanResult.Read("MT:Y.K9042C00KA0648G00*Y.K9042C00KA0648G00")
            screen.onActivity { activity ->
                button(activity, "QRを読み取る").performClick()
                button(activity, "登録").performClick()
            }
            eventually(screen) { hasLabel(it, "寝室に機器を登録しました") }
            assertEquals("", api.registrations.last().name)
        }
    }

    @Test fun failuresKeepTheInputs() {
        api.rooms += Room(1, "寝室")
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            recordOnly(screen)
            eventually(screen) { spinner(it, "部屋").selectedItem == "寝室" }
            scanner.next = ScanResult.Read(qr)
            screen.onActivity { activity ->
                button(activity, "QRを読み取る").performClick()
                field(activity, "機器名（任意）").setText("天井灯")
            }

            api.failure = ApiError.DUPLICATE_QR
            screen.onActivity { button(it, "登録").performClick() }
            eventually(screen) { hasLabel(it, "この機器は登録済みです") }
            assertInputsKept(screen)
            capture(screen, "duplicate")

            api.failure = ApiError.UNREACHABLE
            screen.onActivity { button(it, "登録").performClick() }
            eventually(screen) { hasLabel(it, "home-link（homeserver:5011）に接続できません。Tailscaleの接続を確認してください") }
            assertInputsKept(screen)
            capture(screen, "unreachable")
            screen.onActivity { button(it, "再接続").performClick() }
            eventually(screen) { activity -> views(activity).none { it is Button && it.text == "再接続" && it.isShown } }

            api.failure = ApiError.ROOM_NOT_FOUND
            api.rooms.clear()
            api.rooms += Room(2, "居間")
            screen.onActivity { button(it, "登録").performClick() }
            eventually(screen) { activity ->
                hasLabel(activity, "選んだ部屋が見つかりません。部屋を選び直してください") &&
                    spinner(activity, "部屋").selectedItem == "居間"
            }
            screen.onActivity { activity ->
                assertTrue(hasLabel(activity, "読み取り済み：MT:Y.K90…"))
                assertEquals("天井灯", field(activity, "機器名（任意）").text.toString())
            }
            capture(screen, "room-missing")

            scanner.next = ScanResult.Read("https://example.com/")
            screen.onActivity { activity ->
                button(activity, "読み取り直す").performClick()
                assertTrue(hasLabel(activity, "MatterのQRコードではありません。QRコードが無い機器は「数字で入力」を使ってください"))
                assertTrue(hasLabel(activity, "読み取り済み：MT:Y.K90…"))
            }
            capture(screen, "not-matter")

            api.failure = null
            screen.onActivity { button(it, "登録").performClick() }
            eventually(screen) { hasLabel(it, "居間に「天井灯」を登録しました") }
            assertEquals(Registration(2, qr, "天井灯"), api.registrations.last())
        }
    }

    @Test fun manualCodeIsTypedCheckedAndRegistered() {
        api.rooms += Room(1, "寝室")
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            recordOnly(screen)
            eventually(screen) { spinner(it, "部屋").selectedItem == "寝室" }
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
                assertFalse(button(activity, "登録").isEnabled)
            }
            capture(screen, "manual-length")

            screen.onActivity { field(it, "11桁の数字").requestFocus() }
            instrumentation.waitForIdleSync()
            instrumentation.sendStringSync("2331")
            eventually(screen) { hasLabel(it, "数字が正しくありません。機器に印字された数字と見比べてください") }
            screen.onActivity { activity ->
                assertEquals("3497 011 2331", field(activity, "11桁の数字").text.toString())
                assertFalse(button(activity, "登録").isEnabled)
            }
            capture(screen, "manual-check-digit")

            screen.onActivity { activity ->
                field(activity, "11桁の数字").setText("3497-011-233")
                assertEquals("3497 011 233", field(activity, "11桁の数字").text.toString())
                assertFalse(hasLabel(activity, "数字が正しくありません。機器に印字された数字と見比べてください"))
                field(activity, "11桁の数字").requestFocus()
                field(activity, "11桁の数字").setSelection(12)
            }
            instrumentation.waitForIdleSync()
            instrumentation.sendStringSync("2")
            eventually(screen) { hasLabel(it, "入力済み：3497 …") }
            screen.onActivity { activity ->
                assertFalse(field(activity, "11桁の数字").isShown)
                assertTrue(button(activity, "数字を入力し直す").isShown)
                assertFalse(views(activity).filterIsInstance<TextView>().any { "2332" in it.text })
                field(activity, "機器名（任意）").setText("電球")
            }
            capture(screen, "manual-ready")
            screen.onActivity { button(it, "登録").performClick() }
            eventually(screen) { hasLabel(it, "寝室に「電球」を登録しました") }
            assertEquals(Registration(1, "34970112332", "電球"), api.registrations.single())
            screen.onActivity { activity ->
                assertTrue(hasLabel(activity, "まだ読み取っていません"))
                assertTrue(button(activity, "数字で入力").isShown)
            }
            capture(screen, "manual-registered")

            api.failure = ApiError.DUPLICATE_QR
            screen.onActivity { activity ->
                button(activity, "数字で入力").performClick()
                field(activity, "11桁の数字").setText("34970112332")
                button(activity, "登録").performClick()
            }
            eventually(screen) { hasLabel(it, "この機器は登録済みです") }
            screen.onActivity { assertTrue(hasLabel(it, "入力済み：3497 …")) }
        }
    }

    @Test fun unreachableServerOnLaunchOffersReconnect() {
        api.roomsFailure = ApiError.UNREACHABLE
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            recordOnly(screen)
            eventually(screen) { hasLabel(it, "部屋を読み込めませんでした") }
            scanner.next = ScanResult.Failed
            screen.onActivity { activity ->
                button(activity, "QRを読み取る").performClick()
                assertTrue(hasLabel(activity, "QRコードを読み取れませんでした。もう一度お試しください"))
                assertTrue(hasLabel(activity, "部屋を読み込めませんでした"))
                assertTrue(button(activity, "再接続").isShown)
            }
            capture(screen, "offline")
            api.roomsFailure = null
            api.rooms += Room(1, "寝室")
            screen.onActivity { button(it, "再接続").performClick() }
            eventually(screen) { activity ->
                spinner(activity, "部屋").selectedItem == "寝室" &&
                    !hasLabel(activity, "部屋を読み込めませんでした")
            }
        }
    }

    @Test fun recreationKeepsTheEnteredValues() {
        api.rooms += listOf(Room(1, "寝室"), Room(2, "居間"))
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            recordOnly(screen)
            eventually(screen) { spinner(it, "部屋").count == 2 }
            scanner.next = ScanResult.Read(qr)
            screen.onActivity { spinner(it, "部屋").setSelection(1) }
            InstrumentationRegistry.getInstrumentation().waitForIdleSync()
            screen.onActivity { activity ->
                button(activity, "QRを読み取る").performClick()
                field(activity, "機器名（任意）").setText("天井灯")
            }
            screen.recreate()
            eventually(screen) { spinner(it, "部屋").selectedItem == "居間" }
            screen.onActivity { activity ->
                assertTrue(hasLabel(activity, "読み取り済み：MT:Y.K90…"))
                assertEquals("天井灯", field(activity, "機器名（任意）").text.toString())
                button(activity, "登録").performClick()
            }
            eventually(screen) { hasLabel(it, "居間に「天井灯」を登録しました") }
        }
    }

    @Test fun switchAllLightsAndShowTheResult() {
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { hasLabel(it, "部屋がありません。「部屋を追加」から作成してください") }
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
            eventually(screen) { hasLabel(it, "home-link（homeserver:5011）に接続できません。Tailscaleの接続を確認してください") }
            capture(screen, "lights-unreachable")
        }
    }


    @Test fun commissionOverBluetoothShowsEachStage() {
        api.rooms += Room(1, "押入れ")
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { spinner(it, "部屋").selectedItem == "押入れ" }
            scanner.next = ScanResult.Read(qr)
            screen.onActivity { activity ->
                assertTrue(views(activity).filterIsInstance<RadioButton>().first { it.text.startsWith("新しいWi-Fi電球") }.isChecked)
                assertTrue(hasLabel(activity, "まだ保存していません"))
                button(activity, "QRを読み取る").performClick()
                field(activity, "機器名（任意）").setText("押入れ1")
                // No Wi-Fi to hand over yet.
                assertFalse(button(activity, "登録").isEnabled)
                button(activity, "Wi-Fiを追加").performClick()
                // The network the phone is on fills in the name; only the password is left.
                assertEquals("home-2g", field(activity, "Wi-Fiの名前（SSID）").text.toString())
                assertTrue(hasLabel(activity, "今つながっているWi-Fiの名前を入れました。パスワードを入力してください"))
                assertTrue(field(activity, "Wi-Fiのパスワード").isFocused)
                field(activity, "Wi-Fiのパスワード").setText("x".repeat(65))
                button(activity, "保存").performClick()
                assertTrue(hasLabel(activity, "Wi-Fiの名前（32バイトまで）とパスワード（64文字まで）を入力してください"))
                field(activity, "Wi-Fiのパスワード").setText("kakushi")
            }
            capture(screen, "bt-wifi")
            screen.onActivity { activity ->
                button(activity, "保存").performClick()
                assertEquals("home-2g", spinner(activity, "電球に渡すWi-Fi").selectedItem)
                assertFalse(field(activity, "Wi-Fiのパスワード").isShown)
                assertTrue(button(activity, "登録").isEnabled)
            }
            assertEquals(SavedWifi(listOf(WifiNetwork("home-2g", "kakushi")), "home-2g"), wifi.saved)
            capture(screen, "bt-ready")

            link.script = listOf("start_scan", "stop_scan", "connect", "discover_services", "write_and_subscribe", "disconnect")
            val gate = CountDownLatch(1)
            api.gate = gate
            api.commissioned = ApiResult.Ok(Commissioned(true, "押入れ", "押入れ1"))
            screen.onActivity { button(it, "登録").performClick() }
            for ((text, name) in listOf("電球を探しています…" to "bt-searching", "電球に接続しています…" to "bt-connecting",
                    "電球にコードとWi-Fiの設定を送っています…" to "bt-sending")) {
                eventually(screen) { hasLabel(it, text) }
                screen.onActivity { activity ->
                    assertFalse(button(activity, "登録中…").isEnabled)
                    assertFalse(views(activity).filterIsInstance<RadioButton>().any { it.isEnabled })
                }
                capture(screen, name)
                if (name == "bt-sending") {
                    // A recreated screen follows the same registration.
                    screen.recreate()
                    eventually(screen) { hasLabel(it, text) }
                }
                link.step()
                link.step()
            }
            eventually(screen) { hasLabel(it, "電球がWi-Fiにつながるのを待っています…") }
            capture(screen, "bt-joining")
            link.step()
            gate.countDown()
            eventually(screen) { hasLabel(it, "押入れに「押入れ1」をつなぎ、登録しました") }
            assertEquals(listOf(Commissioning(1, qr, "押入れ1", WifiNetwork("home-2g", "kakushi"))), api.commissionings)
            assertTrue(link.closed)
            screen.onActivity { activity ->
                assertTrue(hasLabel(activity, "まだ読み取っていません"))
                assertEquals("", field(activity, "機器名（任意）").text.toString())
                assertFalse(button(activity, "登録").isEnabled)
            }
            capture(screen, "bt-registered")
        }
    }

    @Test fun commissioningFailuresSayWhatToDoAndKeepTheInputs() {
        api.rooms += Room(1, "押入れ")
        wifi.saved = SavedWifi().added(WifiNetwork("home-2g", "kakushi"))
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { spinner(it, "部屋").selectedItem == "押入れ" }
            scanner.next = ScanResult.Read(qr)
            screen.onActivity { activity ->
                assertEquals("home-2g", spinner(activity, "電球に渡すWi-Fi").selectedItem)
                button(activity, "QRを読み取る").performClick()
                field(activity, "機器名（任意）").setText("押入れ1")
            }
            val cases = listOf(
                ApiError.DEVICE_NOT_FOUND to "電球が見つかりませんでした。電球をペアリング待ちにし、電話を近づけてから「登録」を押し直してください",
                ApiError.WRONG_CODE to "電球がコードを受け付けませんでした。電球のコードを読み取り直すか、入力し直してください",
                ApiError.WIFI_FAILED to "電球がWi-Fi「home-2g」につながりませんでした。Wi-Fiの名前とパスワードを確かめてください。2.4GHzにしか対応しない電球もあります",
                ApiError.COMMISSION_TIMEOUT to "時間内に登録が終わりませんでした。電球をペアリング待ちにし直して、もう一度お試しください",
                ApiError.COMMISSION_FAILED to "電球を登録できませんでした。電球を初期化してから、もう一度お試しください",
                ApiError.BLUETOOTH_UNAVAILABLE to "matterjs-serverでBluetoothの中継が有効になっていません。サーバーの設定を確かめてください",
            )
            for ((error, text) in cases) {
                api.commissioned = ApiResult.Failed(error)
                screen.onActivity { button(it, "登録").performClick() }
                eventually(screen) { hasLabel(it, text) }
                assertInputsKept(screen, "押入れ1", "押入れ")
                if (error == ApiError.DEVICE_NOT_FOUND || error == ApiError.WIFI_FAILED) capture(screen, "bt-${error.name.lowercase()}")
            }

            link.opens = ProxyOpen.UNREACHABLE
            screen.onActivity { button(it, "登録").performClick() }
            eventually(screen) {
                hasLabel(it, "matterjs-server（192.168.1.100:5580）に接続できません。家のWi-Fiにつないでから、もう一度お試しください")
            }
            assertInputsKept(screen, "押入れ1", "押入れ")
            capture(screen, "bt-proxy-unreachable")

            // A device the ledger already holds keeps its entry.
            link.opens = ProxyOpen.OK
            api.commissioned = ApiResult.Ok(Commissioned(false, "寝室", "読書灯"))
            screen.onActivity { button(it, "登録").performClick() }
            eventually(screen) { hasLabel(it, "Wi-Fiにつなぎました。台帳の「読書灯」（寝室）として登録済みです") }
            capture(screen, "bt-already")
            // An unreachable proxy never reaches the API.
            assertEquals(7, api.commissionings.size)
        }
    }

    @Test fun savedNetworksAreChosenFromTheListAddedAndDeleted() {
        api.rooms += Room(1, "押入れ")
        val home = WifiNetwork("home-2g", "kakushi")
        val annex = WifiNetwork("離れの2.4GHz（物置と作業部屋）", "hanare")
        wifi.saved = SavedWifi(listOf(home, annex), last = annex.ssid)
        api.commissioned = ApiResult.Ok(Commissioned(true, "押入れ", ""))
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { spinner(it, "部屋").selectedItem == "押入れ" }
            // The network used last is chosen already.
            screen.onActivity { activity ->
                val choice = spinner(activity, "電球に渡すWi-Fi")
                assertEquals(annex.ssid, choice.selectedItem)
                assertEquals(listOf(home.ssid, annex.ssid), (0 until choice.count).map { choice.getItemAtPosition(it) })
            }
            capture(screen, "wifi-list")
            scanner.next = ScanResult.Read(qr)
            screen.onActivity { activity ->
                button(activity, "QRを読み取る").performClick()
                button(activity, "登録").performClick()
            }
            eventually(screen) { hasLabel(it, "押入れに機器をつなぎ、登録しました") }
            assertEquals(annex, api.commissionings.last().wifi)

            // Choosing another network is remembered, also across a recreated screen.
            screen.onActivity { spinner(it, "電球に渡すWi-Fi").setSelection(0) }
            eventually(screen) { wifi.saved.last == home.ssid }
            screen.recreate()
            screen.onActivity { assertEquals(home.ssid, spinner(it, "電球に渡すWi-Fi").selectedItem) }
            scanner.next = ScanResult.Read(qr)
            screen.onActivity { activity ->
                button(activity, "QRを読み取る").performClick()
                button(activity, "登録").performClick()
            }
            eventually(screen) { api.commissionings.size == 2 }
            assertEquals(home, api.commissionings.last().wifi)

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
            screen.onActivity { activity ->
                assertEquals("home-2g", field(activity, "Wi-Fiの名前（SSID）").text.toString())
                assertEquals("", field(activity, "Wi-Fiのパスワード").text.toString())
                // The same name replaces the saved password instead of adding a second entry.
                field(activity, "Wi-Fiのパスワード").setText("new-pass")
                button(activity, "保存").performClick()
                assertEquals(home.ssid, spinner(activity, "電球に渡すWi-Fi").selectedItem)
            }
            assertEquals(SavedWifi(listOf(WifiNetwork("home-2g", "new-pass"), annex), home.ssid), wifi.saved)

            // Without the name of the network, the adder says why and leaves the name to type.
            for ((current, text) in listOf(
                    null to "Wi-Fiにつながっていません。電球に渡すWi-Fiの名前を入力してください",
                    CurrentWifi(null, 2437) to "今つながっているWi-Fiの名前を読めませんでした。位置情報がオンか確かめるか、名前を入力してください")) {
                reader.next = current
                screen.onActivity { activity ->
                    button(activity, "Wi-Fiを追加").performClick()
                    assertTrue(hasLabel(activity, text))
                    assertEquals("", field(activity, "Wi-Fiの名前（SSID）").text.toString())
                    assertFalse(button(activity, "保存").isEnabled)
                    button(activity, "やめる").performClick()
                    assertTrue(spinner(activity, "電球に渡すWi-Fi").isShown)
                }
            }

            // Deleting the chosen network falls back to the one left, then to none.
            screen.onActivity { activity ->
                button(activity, "このWi-Fiを削除").performClick()
                assertTrue(hasLabel(activity, "「home-2g」を削除しました"))
                assertEquals(annex.ssid, spinner(activity, "電球に渡すWi-Fi").selectedItem)
                assertEquals(1, spinner(activity, "電球に渡すWi-Fi").count)
            }
            capture(screen, "wifi-deleted")
            screen.onActivity { activity ->
                button(activity, "このWi-Fiを削除").performClick()
                assertTrue(hasLabel(activity, "まだ保存していません"))
                assertFalse(spinner(activity, "電球に渡すWi-Fi").isShown)
                assertFalse(views(activity).filterIsInstance<Button>().any { it.isShown && it.text == "このWi-Fiを削除" })
                scanner.next = ScanResult.Read(qr)
                button(activity, "QRを読み取る").performClick()
                assertFalse(button(activity, "登録").isEnabled)
            }
            assertEquals(SavedWifi(), wifi.saved)
            capture(screen, "wifi-empty")
        }
    }

    private val threadTab = "Threadの電球\n（T2など）"

    private fun tab(activity: MainActivity, text: String) =
        views(activity).filterIsInstance<RadioButton>().first { it.text == text }

    @Test fun tabsSplitWifiAndThreadBulbs() {
        api.rooms += Room(1, "寝室")
        wifi.saved = SavedWifi().added(WifiNetwork("home-2g", "kakushi"))
        api.thread = ApiResult.Ok(true)
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { spinner(it, "部屋").selectedItem == "寝室" }
            screen.onActivity { activity ->
                assertTrue(tab(activity, "Wi-Fiの電球").isChecked)
                assertFalse(tab(activity, threadTab).isChecked)
                // The underlines line up although the Thread tab takes two lines.
                assertEquals(tab(activity, "Wi-Fiの電球").height, tab(activity, threadTab).height)
                assertTrue(spinner(activity, "電球に渡すWi-Fi").isShown)
                assertFalse(hasLabel(activity, "Thread網"))
                assertTrue(tab(activity, "新しいWi-Fi電球をBluetoothでつなぐ").isChecked)
            }
            assertEquals(0, api.threadChecks)
            capture(screen, "tab-wifi")

            screen.onActivity { tab(it, threadTab).performClick() }
            eventually(screen) { hasLabel(it, "準備できています。電球にはmatterjs-serverが持つThread網の設定を渡します") }
            screen.onActivity { activity ->
                assertTrue(tab(activity, threadTab).isChecked)
                assertFalse(tab(activity, "Wi-Fiの電球").isChecked)
                // No Wi-Fi to choose: Thread bulbs take the network matterjs-server holds.
                assertFalse(hasLabel(activity, "電球に渡すWi-Fi"))
                assertFalse(spinner(activity, "電球に渡すWi-Fi").isShown)
                assertTrue(hasLabel(activity, "Thread網"))
                assertTrue(tab(activity, "新しいThread電球をBluetoothでつなぐ").isChecked)
                assertTrue(views(activity).filterIsInstance<TextView>().any { it.isShown && "電源のオフ・オンを1秒間隔で10回" in it.text })
                assertFalse(views(activity).filterIsInstance<TextView>().any { it.isShown && "BEAMTEC" in it.text })
            }
            capture(screen, "tab-thread")

            // The chosen tab survives a recreated screen, and switching back restores the Wi-Fi flow.
            screen.recreate()
            eventually(screen) { tab(it, threadTab).isChecked && hasLabel(it, "Thread網") }
            screen.onActivity { tab(it, "Wi-Fiの電球").performClick() }
            screen.onActivity { activity ->
                assertEquals("home-2g", spinner(activity, "電球に渡すWi-Fi").selectedItem)
                assertTrue(spinner(activity, "電球に渡すWi-Fi").isShown)
                assertFalse(hasLabel(activity, "Thread網"))
            }

            // Recording only works from either tab and needs no network.
            screen.onActivity { activity ->
                tab(activity, threadTab).performClick()
                tab(activity, "記録だけ（つながっている機器）").performClick()
                assertFalse(hasLabel(activity, "Thread網"))
                assertFalse(hasLabel(activity, "電球に渡すWi-Fi"))
            }
        }
    }

    @Test fun threadBulbsCommissionWithoutWifi() {
        api.rooms += Room(1, "寝室")
        api.thread = ApiResult.Ok(true)
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { spinner(it, "部屋").selectedItem == "寝室" }
            screen.onActivity { tab(it, threadTab).performClick() }
            eventually(screen) { hasLabel(it, "準備できています。電球にはmatterjs-serverが持つThread網の設定を渡します") }
            scanner.next = ScanResult.Read(qr)
            screen.onActivity { activity ->
                button(activity, "QRを読み取る").performClick()
                field(activity, "機器名（任意）").setText("寝室T2")
                // Nothing saved for Wi-Fi, yet Thread can register.
                assertTrue(button(activity, "登録").isEnabled)
            }
            capture(screen, "thread-ready")

            link.script = listOf("start_scan", "connect", "write_and_subscribe", "disconnect")
            val gate = CountDownLatch(1)
            api.gate = gate
            api.commissioned = ApiResult.Ok(Commissioned(true, "寝室", "寝室T2"))
            screen.onActivity { button(it, "登録").performClick() }
            for ((text, name) in listOf("電球を探しています…" to null, "電球に接続しています…" to null,
                    "電球にコードとThread網の設定を送っています…" to "thread-sending",
                    "電球がThread網につながるのを待っています…" to "thread-joining")) {
                eventually(screen) { hasLabel(it, text) }
                screen.onActivity { activity ->
                    assertFalse(views(activity).filterIsInstance<RadioButton>().any { it.isEnabled })
                }
                name?.let { capture(screen, it) }
                link.step()
            }
            gate.countDown()
            eventually(screen) { hasLabel(it, "寝室に「寝室T2」をつなぎ、登録しました") }
            assertEquals(listOf(Commissioning(1, qr, "寝室T2", null)), api.commissionings)
            capture(screen, "thread-registered")

            // A device already in the ledger says it joined Thread.
            api.gate = null
            api.commissioned = ApiResult.Ok(Commissioned(false, "寝室", "読書灯"))
            screen.onActivity { activity ->
                button(activity, "QRを読み取る").performClick()
                button(activity, "登録").performClick()
            }
            eventually(screen) { hasLabel(it, "Thread網につなぎました。台帳の「読書灯」（寝室）として登録済みです") }
        }
    }

    @Test fun threadRegistrationWaitsForTheThreadNetwork() {
        api.rooms += Room(1, "寝室")
        api.thread = ApiResult.Ok(false)
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { spinner(it, "部屋").selectedItem == "寝室" }
            scanner.next = ScanResult.Read(qr)
            screen.onActivity { activity ->
                tab(activity, threadTab).performClick()
                button(activity, "QRを読み取る").performClick()
                field(activity, "機器名（任意）").setText("寝室T2")
            }
            val notReady = "Thread網が未準備です。homeserverでThread網（OTBR）を用意するまで、Threadの電球は登録できません"
            eventually(screen) { hasLabel(it, notReady) }
            screen.onActivity { activity ->
                assertFalse(button(activity, "登録").isEnabled)
                assertTrue(button(activity, "もう一度確かめる").isShown)
            }
            capture(screen, "thread-not-ready")

            api.thread = ApiResult.Failed(ApiError.UNREACHABLE)
            screen.onActivity { button(it, "もう一度確かめる").performClick() }
            eventually(screen) {
                hasLabel(it, "Thread網の状態を確かめられませんでした。home-linkにつながるか確かめてから、もう一度確かめてください")
            }
            screen.onActivity { assertFalse(button(it, "登録").isEnabled) }
            capture(screen, "thread-check-failed")

            // Once matterjs-server holds the network, registering opens without other changes.
            api.thread = ApiResult.Ok(true)
            screen.onActivity { button(it, "もう一度確かめる").performClick() }
            eventually(screen) { button(it, "登録").isEnabled }
            screen.onActivity { assertFalse(views(it).filterIsInstance<Button>().any { b -> b.isShown && b.text == "もう一度確かめる" }) }

            // home-link refusing it marks the network not ready again and keeps the inputs.
            api.commissioned = ApiResult.Failed(ApiError.THREAD_NOT_READY)
            screen.onActivity { button(it, "登録").performClick() }
            eventually(screen) { hasLabel(it, "Thread網が未準備です。homeserverでThread網を用意してから登録してください") }
            screen.onActivity { activity ->
                assertTrue(hasLabel(activity, notReady))
                assertFalse(button(activity, "登録").isEnabled)
                assertTrue(hasLabel(activity, "読み取り済み：MT:Y.K90…"))
                assertEquals("寝室T2", field(activity, "機器名（任意）").text.toString())
            }
            capture(screen, "thread-refused")

            api.thread = ApiResult.Ok(true)
            api.commissioned = ApiResult.Failed(ApiError.THREAD_FAILED)
            screen.onActivity { button(it, "もう一度確かめる").performClick() }
            eventually(screen) { button(it, "登録").isEnabled }
            screen.onActivity { button(it, "登録").performClick() }
            eventually(screen) {
                hasLabel(it, "電球がThread網につながりませんでした。Thread網（OTBR）が動いているか確かめてから、もう一度お試しください")
            }
            assertInputsKept(screen, "寝室T2", "寝室")
        }
    }

    @Test fun theWifiPasswordIsKeptEncryptedInTheKeystore() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val store = KeystoreWifiStore(context)
        val saved = SavedWifi().added(WifiNetwork("home-2g", "kakushi-pass")).added(WifiNetwork("guest", "welcome-pass"))
        store.save(saved)
        assertEquals(saved, KeystoreWifiStore(context).load())
        val prefs = context.getSharedPreferences("wifi", android.content.Context.MODE_PRIVATE).all.values.joinToString()
        assertFalse(listOf("kakushi-pass", "home-2g", "welcome-pass", "guest").any { it in prefs })
        context.getSharedPreferences("wifi", android.content.Context.MODE_PRIVATE).edit().clear().commit()
    }

    private fun assertInputsKept(screen: ActivityScenario<MainActivity>, name: String = "天井灯", room: String = "寝室") =
        screen.onActivity { activity ->
            assertTrue(hasLabel(activity, "読み取り済み：MT:Y.K90…"))
            assertEquals(name, field(activity, "機器名（任意）").text.toString())
            assertEquals(room, spinner(activity, "部屋").selectedItem)
            assertTrue(button(activity, "登録").isEnabled)
        }

    private fun eventually(screen: ActivityScenario<MainActivity>, condition: (MainActivity) -> Boolean) {
        val deadline = System.currentTimeMillis() + 5_000
        while (true) {
            var met = false
            screen.onActivity { met = condition(it) }
            if (met) return
            assertTrue("condition not met in time", System.currentTimeMillis() < deadline)
            Thread.sleep(50)
        }
    }

    /** Saves a screenshot and the text layout, asserting no clipped text and 48dp targets. */
    private fun capture(screen: ActivityScenario<MainActivity>, name: String) {
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
            val register = button(activity, views(activity).filterIsInstance<Button>().last().text.toString())
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

    private fun views(activity: MainActivity) = descendants(activity.window.decorView)
    private fun descendants(view: View): List<View> = listOf(view) +
        if (view is ViewGroup) (0 until view.childCount).flatMap { descendants(view.getChildAt(it)) } else emptyList()
    private fun hasLabel(activity: MainActivity, text: String) =
        views(activity).filterIsInstance<TextView>().any { it.isShown && it.text.toString() == text }
    private fun button(activity: MainActivity, text: String) =
        views(activity).filterIsInstance<Button>().first { it.isShown && it.text.toString() == text }
    private fun spinner(activity: MainActivity, description: String) =
        views(activity).filterIsInstance<Spinner>().first { it.contentDescription == description }
    private fun field(activity: MainActivity, description: String) =
        views(activity).filterIsInstance<EditText>().first { it.contentDescription == description }
}

private data class Registration(val roomId: Long, val payload: String, val name: String)
private data class Commissioning(val roomId: Long, val payload: String, val name: String, val wifi: WifiNetwork?)

private class FakeApi : HomeLinkApi {
    val rooms: MutableList<Room> = Collections.synchronizedList(mutableListOf())
    val registrations: MutableList<Registration> = Collections.synchronizedList(mutableListOf())
    @Volatile var failure: ApiError? = null
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

    override fun register(roomId: Long, payload: String, name: String): ApiResult<Unit> {
        failure?.let { return ApiResult.Failed(it) }
        registrations += Registration(roomId, payload, name)
        return ApiResult.Ok(Unit)
    }

    override fun switchLights(on: Boolean): ApiResult<LightsResult> {
        gate?.await()
        switches += on
        return lights
    }

    val commissionings: MutableList<Commissioning> = Collections.synchronizedList(mutableListOf())
    @Volatile var commissioned: ApiResult<Commissioned> = ApiResult.Failed(ApiError.SERVER)

    override fun commission(roomId: Long, payload: String, name: String, wifi: WifiNetwork?): ApiResult<Commissioned> {
        gate?.await()
        commissionings += Commissioning(roomId, payload, name, wifi)
        return commissioned
    }

    @Volatile var thread: ApiResult<Boolean> = ApiResult.Ok(false)
    @Volatile var threadChecks = 0

    override fun threadReady(): ApiResult<Boolean> {
        threadChecks++
        return thread
    }
}

/** Plays proxy commands one [step] at a time, as matterjs-server would send them. */
private class FakeLink : BleLink {
    @Volatile var script: List<String> = emptyList()
    @Volatile var opens = ProxyOpen.OK
    @Volatile var closed = false
    private val steps = Semaphore(0)

    fun step() = steps.release()

    override fun open(onCommand: (String) -> Unit): ProxyOpen {
        closed = false
        for (command in script) {
            onCommand(command)
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
    override fun save(saved: SavedWifi) { this.saved = saved }
}

private class FakeWifiReader : WifiReader {
    @Volatile var next: CurrentWifi? = CurrentWifi("home-2g", 2437)
    override fun read(done: (CurrentWifi?) -> Unit) = done(next)
}

private class FakeScanner : QrScanner {
    var next: ScanResult = ScanResult.Cancelled
    override fun scan(done: (ScanResult) -> Unit) = done(next)
}
