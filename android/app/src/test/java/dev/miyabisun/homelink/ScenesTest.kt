package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ScenesTest {
    private val host = "home-link.example:5011"
    private fun waiting(reason: String, distance: Double? = null) = SceneRun.Waiting(1, 30, reason, distance)

    @Test fun aRunThatEndedShowsWhetherEveryStepWent() {
        assertEquals(LightsMessage("完了", failed = false), scenePress(ApiResult.Ok(SceneRun.Done(0, null))))
        assertEquals(LightsMessage("一部失敗", failed = true), scenePress(ApiResult.Ok(SceneRun.Done(2, "on: room"))))
        assertEquals(listOf("操作なし", "接続不可", "結果不明"),
            listOf(ApiError.SCENE_NOT_FOUND, ApiError.UNREACHABLE, ApiError.TIMEOUT).map { scenePress(ApiResult.Failed(it)).text })
    }

    @Test fun aWaitThePhoneCannotEndSaysWhatIsNotRegistered() {
        assertEquals(LightsMessage("電話未登録", failed = true), scenePress(ApiResult.Ok(waiting("phone_not_registered"))))
        assertEquals(LightsMessage("自宅未登録", failed = true), scenePress(ApiResult.Ok(waiting("home_not_set"))))
        // A wait a fix may still end is no result: the button shows it waiting instead.
        for (reason in listOf("no_location", "stale", "vague", "far")) {
            assertTrue(reason, waiting(reason).watchable)
            assertEquals(reason, LightsMessage("未到着", failed = true), scenePress(ApiResult.Ok(waiting(reason))))
        }
        for (reason in listOf("phone_not_registered", "home_not_set", "no_phone", "later")) assertFalse(reason, waiting(reason).watchable)
    }

    @Test fun aWatchThatEndedWithoutARunSaysWhy() {
        assertEquals(listOf("未到着" to true, "取り消し" to false, "位置未許可" to true, "位置オフ" to true),
            WatchEnd.entries.map { it.label.text to it.label.failed })
    }

    @Test fun whatIsReadAloudNamesTheRoutineAndWhatToDo() {
        assertEquals("帰宅を実行しました", sceneMessage("帰宅", ApiResult.Ok(SceneRun.Done(0, null)), host))
        assertEquals("帰宅を実行しましたが、2つのステップが失敗しました。最初は「on: room」です",
            sceneMessage("帰宅", ApiResult.Ok(SceneRun.Done(2, "on: room")), host))
        assertEquals("この電話がhome-linkに登録されていません。アプリの「この電話の位置」で登録してください",
            sceneMessage("帰宅", ApiResult.Ok(waiting("phone_not_registered")), host))
        assertEquals("自宅の位置が登録されていません。自宅でアプリの「この電話の位置」から登録してください",
            sceneMessage("帰宅", ApiResult.Ok(waiting("home_not_set")), host))
        assertEquals("自宅の近くに着かなかったため、帰宅の続きは実行していません",
            sceneMessage("帰宅", ApiResult.Ok(waiting("far", 812.0)), host))
        assertEquals("home-linkに「帰宅」という操作がありません。ウィジェットを設定し直してください",
            sceneMessage("帰宅", ApiResult.Failed(ApiError.SCENE_NOT_FOUND), host))
        assertEquals(failure(ApiError.UNREACHABLE, host).text, sceneMessage("帰宅", ApiResult.Failed(ApiError.UNREACHABLE), host))
    }

    @Test fun theButtonReadsAsItsNameWaitingOrAPressesResult() {
        assertEquals(ButtonFace("帰宅", R.drawable.ic_power, Tone.PLAIN), sceneFace("帰宅", null))
        assertEquals(ButtonFace("送信中…", R.drawable.ic_power, Tone.PLAIN), sceneFace("帰宅", SENDING, sending = true))
        assertEquals(ButtonFace("待機中", R.drawable.ic_near, Tone.ACTIVE, "帰宅：自宅の近くに着くのを待っています。押すと取り消します"),
            sceneFace("帰宅", null, waiting = true))
        assertEquals(ButtonFace("完了", R.drawable.ic_success, Tone.PLAIN), sceneFace("帰宅", LightsMessage("完了", failed = false)))
        assertEquals(ButtonFace("未到着", R.drawable.ic_error, Tone.FAILED), sceneFace("帰宅", WatchEnd.EXPIRED.label))
    }

    @Test fun aFixTellsHowLongAgoItWasMeasured() {
        assertEquals(1_500L, fixAgeMs(nowNanos = 9_500_000_000, measuredNanos = 8_000_000_000))
        // A clock that reads behind the fix is no age at all.
        assertEquals(0L, fixAgeMs(nowNanos = 1, measuredNanos = 2))
    }

    @Test fun farFromHomeTheNextFixIsHeldBackForAsLongAsACarTakes() {
        assertEquals(listOf(0L, 0L, 0L, 30_000L, 300_000L),
            listOf(null, 99.0, 1_000.0, 1_900.0, 10_000.0).map { resendAfterMs(it) })
    }

    @Test fun theHomeIsPinnedOnlyFromAFixSureEnough() {
        assertNull(pinRefusal(Fix(34.0, 132.0, 50.0, 0)))
        assertEquals("位置の精度が51mです。屋外か窓の近くでやり直してください", pinRefusal(Fix(34.0, 132.0, 50.6, 0)))
    }
}
