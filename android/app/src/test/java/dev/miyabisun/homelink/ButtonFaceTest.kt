package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class ButtonFaceTest {
    @Test fun failuresNameWhatToCheck() {
        val host = "home-link.example:5011"
        assertEquals(LightsMessage("home-link（home-link.example:5011）に接続できません。接続先の設定と通信を確かめてください", failed = true),
            failure(ApiError.UNREACHABLE, host))
        assertEquals(LightsMessage("接続先が未設定です。アプリの「接続先の設定」でhome-linkのURLを入れてください", failed = true),
            failure(ApiError.NOT_CONFIGURED, host))
        assertEquals(LightsMessage("照明のサーバー（matterjs-server）に接続できません", failed = true), failure(ApiError.MATTER_UNREACHABLE, host))
        assertEquals(LightsMessage("home-linkに照明のサーバーが設定されていません", failed = true), failure(ApiError.MATTER_NOT_CONFIGURED, host))
        assertEquals(LightsMessage("home-linkでエラーが発生しました。時間をおいてお試しください", failed = true), failure(ApiError.SERVER, host))
    }

    @Test fun theShortLabelNamesTheFailure() {
        assertEquals(LightsMessage("接続不可", failed = true), failureLabel(ApiError.UNREACHABLE))
        assertEquals(LightsMessage("SESAME不通", failed = true), failureLabel(ApiError.SESAME_UNREACHABLE))
        assertEquals(LightsMessage("Matter不通", failed = true), failureLabel(ApiError.MATTER_UNREACHABLE))
        assertEquals(LightsMessage("Matter未設定", failed = true), failureLabel(ApiError.MATTER_NOT_CONFIGURED))
        assertEquals(LightsMessage("接続先なし", failed = true), failureLabel(ApiError.NOT_CONFIGURED))
        assertEquals(LightsMessage("エラー", failed = true), failureLabel(ApiError.SERVER))
    }

    @Test fun eachStretchWhereAPressStoppedReadsDifferently() {
        val host = "home-link.example:5011"
        // The phone could not send; home-link could not be reached; it was sent but nothing came back;
        // something other than home-link answered; home-link answered but the lights did not.
        val cases = listOf(
            ApiError.NO_NETWORK to ("回線なし" to "電話に使える回線が無く、送れませんでした"),
            ApiError.BLOCKED to ("通信制限" to "電話の通信制限（データセーバーなど）で送れませんでした"),
            ApiError.UNREACHABLE to ("接続不可" to "home-link（home-link.example:5011）に接続できません。接続先の設定と通信を確かめてください"),
            ApiError.CONNECT_TIMEOUT to ("時間切れ" to "home-link（home-link.example:5011）に時間内に接続できませんでした。接続先の設定と通信（VPNなど）を確かめてください"),
            ApiError.TIMEOUT to ("結果不明" to "home-link（home-link.example:5011）に送りましたが、時間内に応答がありませんでした。届いたかは分かりません"),
            ApiError.HTTP_DENIED to ("要求拒否" to "home-link（home-link.example:5011）への要求が拒否されました。間にあるプロキシや認証の設定を確かめてください"),
            ApiError.HTTP_NOT_FOUND to ("URL違い" to "home-link（home-link.example:5011）にこの操作の窓口がありません。接続先のURLとhome-linkの版を確かめてください"),
            ApiError.HTTP_GATEWAY to ("中継エラー" to "home-link（home-link.example:5011）の手前の中継がエラーを返しました。home-linkが動いているか確かめてください"),
            ApiError.BAD_ANSWER to ("応答不正" to "home-link（home-link.example:5011）から読めない応答が返りました。接続先がhome-linkか確かめてください"),
        )
        for ((error, words) in cases) {
            assertEquals("$error", LightsMessage(words.first, failed = true), failureLabel(error))
            assertEquals("$error", LightsMessage(words.second, failed = true), failure(error, host))
        }
        val silent = pressLabel(LabelOp.ON, ApiResult.Ok(LabelResult("on", true, SwitchResult(0, 3, 0, emptyList()))))
        val labels = cases.map { it.second.first } + listOfNotNull(silent?.text) + failureLabel(ApiError.SERVER).text
        assertEquals(labels.distinct(), labels)
    }

    @Test fun aButtonInEffectReadsAsSo() {
        assertEquals(ButtonFace("仕事中", R.drawable.ic_boosted, Tone.ACTIVE), buttonFace("仕事", R.drawable.ic_power, Toggle.ON, label = null))
        assertEquals(ButtonFace("仕事", R.drawable.ic_power, Tone.PLAIN), buttonFace("仕事", R.drawable.ic_power, Toggle.OFF, label = null))
        // A state that could not be read reads as unknown, never as off.
        assertEquals(ButtonFace("仕事", R.drawable.ic_unknown, Tone.PLAIN, "仕事（状態不明）"),
            buttonFace("仕事", R.drawable.ic_power, Toggle.UNKNOWN, label = null))
        // A button without a state reads as its name.
        assertEquals(ButtonFace("解錠", R.drawable.ic_unlock, Tone.PLAIN), buttonFace("解錠", R.drawable.ic_unlock, null, label = null))
    }

    @Test fun theAirconReadsOnWithItsOwnIcon() {
        assertEquals(ButtonFace("エアコン中", R.drawable.ic_aircon_on, Tone.ACTIVE), airconFace(on = true, press = null))
        assertEquals(ButtonFace("エアコン", R.drawable.ic_power, Tone.PLAIN), airconFace(on = false, press = null))
        assertEquals(ButtonFace("エアコン", R.drawable.ic_unknown, Tone.PLAIN, "エアコン（状態不明）"), airconFace(on = null, press = null))
        assertEquals(ButtonFace("送信中…", R.drawable.ic_power, Tone.PLAIN),
            airconFace(on = true, press = LightsMessage("送信中…", failed = false), sending = true))
        assertEquals(ButtonFace("接続不可", R.drawable.ic_error, Tone.FAILED),
            airconFace(on = true, press = failureLabel(ApiError.UNREACHABLE)))
    }

    @Test fun anAirconPressThatDidNotReachItReadsAsAFailure() {
        val host = "home-link.example:5011"
        val reached = ApiResult.Ok(AirconToggle(on = true, delivered = true, result = "sent"))
        val unreached = ApiResult.Ok(AirconToggle(on = true, delivered = false, result = "no_response", reason = "子機が応答しませんでした"))
        // Reached: the button shows the new state at once, as before.
        assertEquals(null, airconPress(reached))
        assertEquals(null, airconMessage(reached, host))
        // Kept but not reached: a failure for a while, said apart from every failure of the request itself.
        assertEquals(LightsMessage("届かず", failed = true), airconPress(unreached))
        assertEquals(ButtonFace("届かず", R.drawable.ic_error, Tone.FAILED), airconFace(on = true, press = airconPress(unreached)))
        assertFalse(ApiError.entries.any { failureLabel(it).text == "届かず" })
        assertEquals("子機が応答しませんでした。入・切は保存済みで、10分ごとにエアコンへ送り直します", airconMessage(unreached, host))
        assertEquals("エアコンへ届いていません。入・切は保存済みで、10分ごとにエアコンへ送り直します",
            airconMessage(ApiResult.Ok(AirconToggle(on = false, delivered = false, result = "pending")), host))
        // The request itself failed: the one wording for every widget.
        val failed = ApiResult.Failed(ApiError.UNREACHABLE)
        assertEquals(failureLabel(ApiError.UNREACHABLE), airconPress(failed))
        assertEquals(failure(ApiError.UNREACHABLE, host).text, airconMessage(failed, host))
    }

    @Test fun aPressShowsItsProgressThenItsShortResult() {
        val sending = LightsMessage("送信中…", failed = false)
        assertEquals(ButtonFace("送信中…", R.drawable.ic_power, Tone.PLAIN),
            buttonFace("仕事", R.drawable.ic_power, Toggle.ON, label = sending, sending = true))
        assertEquals(ButtonFace("機器なし", R.drawable.ic_error, Tone.FAILED),
            buttonFace("仕事", R.drawable.ic_power, Toggle.ON, label = LightsMessage("機器なし", failed = true)))
        assertEquals(ButtonFace("受付済み", R.drawable.ic_success, Tone.PLAIN),
            buttonFace("解錠", R.drawable.ic_unlock, null, label = LightsMessage("受付済み", failed = false)))
    }
}
