package dev.miyabisun.homelink

import android.os.ParcelFileDescriptor
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
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

@RunWith(AndroidJUnit4::class)
class RegisterScreenTest {
    private val qr = "MT:Y.K9042C00KA0648G00"
    private val api = FakeApi()
    private val scanner = FakeScanner()

    @Before fun install() {
        MainActivity.apiFactory = { api }
        MainActivity.scannerFactory = { scanner }
    }

    @After fun uninstall() {
        MainActivity.apiFactory = null
        MainActivity.scannerFactory = null
    }

    @Test fun createSelectAndRegister() {
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
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
                val spinner = views(activity).filterIsInstance<Spinner>().single()
                assertEquals("リビングと続きの和室（南側・大きな窓のある部屋）", spinner.selectedItem)
                spinner.setSelection(0)
            }
            eventually(screen) { views(it).filterIsInstance<Spinner>().single().selectedItem == "寝室" }

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
                assertEquals("寝室", views(activity).filterIsInstance<Spinner>().single().selectedItem)
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
            eventually(screen) { views(it).filterIsInstance<Spinner>().single().selectedItem == "寝室" }
            scanner.next = ScanResult.Read(qr)
            screen.onActivity { activity ->
                button(activity, "QRを読み取る").performClick()
                field(activity, "機器名（任意）").setText("天井灯")
            }

            api.failure = ApiError.DUPLICATE_QR
            screen.onActivity { button(it, "登録").performClick() }
            eventually(screen) { hasLabel(it, "このQRコードの機器は登録済みです") }
            assertInputsKept(screen)
            capture(screen, "duplicate")

            api.failure = ApiError.UNREACHABLE
            screen.onActivity { button(it, "登録").performClick() }
            eventually(screen) { hasLabel(it, "home-link（homeserver:5009）に接続できません。Tailscaleの接続を確認してください") }
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
                    views(activity).filterIsInstance<Spinner>().single().selectedItem == "居間"
            }
            screen.onActivity { activity ->
                assertTrue(hasLabel(activity, "読み取り済み：MT:Y.K90…"))
                assertEquals("天井灯", field(activity, "機器名（任意）").text.toString())
            }
            capture(screen, "room-missing")

            scanner.next = ScanResult.Read("https://example.com/")
            screen.onActivity { activity ->
                button(activity, "読み取り直す").performClick()
                assertTrue(hasLabel(activity, "MatterのQRコードではありません。電球に印刷されたQRコードを読み取ってください"))
                assertTrue(hasLabel(activity, "読み取り済み：MT:Y.K90…"))
            }
            capture(screen, "not-matter")

            api.failure = null
            screen.onActivity { button(it, "登録").performClick() }
            eventually(screen) { hasLabel(it, "居間に「天井灯」を登録しました") }
            assertEquals(Registration(2, qr, "天井灯"), api.registrations.last())
        }
    }

    @Test fun unreachableServerOnLaunchOffersReconnect() {
        api.roomsFailure = ApiError.UNREACHABLE
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
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
                views(activity).filterIsInstance<Spinner>().single().selectedItem == "寝室" &&
                    !hasLabel(activity, "部屋を読み込めませんでした")
            }
        }
    }

    @Test fun recreationKeepsTheEnteredValues() {
        api.rooms += listOf(Room(1, "寝室"), Room(2, "居間"))
        ActivityScenario.launch(MainActivity::class.java).use { screen ->
            eventually(screen) { views(it).filterIsInstance<Spinner>().single().count == 2 }
            scanner.next = ScanResult.Read(qr)
            screen.onActivity { views(it).filterIsInstance<Spinner>().single().setSelection(1) }
            InstrumentationRegistry.getInstrumentation().waitForIdleSync()
            screen.onActivity { activity ->
                button(activity, "QRを読み取る").performClick()
                field(activity, "機器名（任意）").setText("天井灯")
            }
            screen.recreate()
            eventually(screen) { views(it).filterIsInstance<Spinner>().single().selectedItem == "居間" }
            screen.onActivity { activity ->
                assertTrue(hasLabel(activity, "読み取り済み：MT:Y.K90…"))
                assertEquals("天井灯", field(activity, "機器名（任意）").text.toString())
                button(activity, "登録").performClick()
            }
            eventually(screen) { hasLabel(it, "居間に「天井灯」を登録しました") }
        }
    }

    private fun assertInputsKept(screen: ActivityScenario<MainActivity>) = screen.onActivity { activity ->
        assertTrue(hasLabel(activity, "読み取り済み：MT:Y.K90…"))
        assertEquals("天井灯", field(activity, "機器名（任意）").text.toString())
        assertEquals("寝室", views(activity).filterIsInstance<Spinner>().single().selectedItem)
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
    private fun field(activity: MainActivity, description: String) =
        views(activity).filterIsInstance<EditText>().first { it.contentDescription == description }
}

private data class Registration(val roomId: Long, val payload: String, val name: String)

private class FakeApi : HomeLinkApi {
    val rooms: MutableList<Room> = Collections.synchronizedList(mutableListOf())
    val registrations: MutableList<Registration> = Collections.synchronizedList(mutableListOf())
    @Volatile var failure: ApiError? = null
    @Volatile var roomsFailure: ApiError? = null

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
}

private class FakeScanner : QrScanner {
    var next: ScanResult = ScanResult.Cancelled
    override fun scan(done: (ScanResult) -> Unit) = done(next)
}
