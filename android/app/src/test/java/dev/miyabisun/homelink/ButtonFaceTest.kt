package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
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
        assertEquals(LightsMessage("接続不可", failed = true), failureLabel(ApiError.MATTER_UNREACHABLE))
        assertEquals(LightsMessage("未設定", failed = true), failureLabel(ApiError.MATTER_NOT_CONFIGURED))
        assertEquals(LightsMessage("接続先未設定", failed = true), failureLabel(ApiError.NOT_CONFIGURED))
        assertEquals(LightsMessage("エラー", failed = true), failureLabel(ApiError.SERVER))
    }

    @Test fun aButtonInEffectReadsAsSo() {
        assertEquals(ButtonFace("仕事中", R.drawable.ic_boosted, Tone.ACTIVE), buttonFace("仕事", R.drawable.ic_power, boosted = true, label = null))
        assertEquals(ButtonFace("仕事", R.drawable.ic_power, Tone.PLAIN), buttonFace("仕事", R.drawable.ic_power, boosted = false, label = null))
        assertEquals(ButtonFace("仕事", R.drawable.ic_power, Tone.PLAIN), buttonFace("仕事", R.drawable.ic_power, boosted = null, label = null))
    }

    @Test fun aPressShowsItsProgressThenItsShortResult() {
        val sending = LightsMessage("送信中…", failed = false)
        assertEquals(ButtonFace("送信中…", R.drawable.ic_power, Tone.PLAIN),
            buttonFace("仕事", R.drawable.ic_power, boosted = true, label = sending, sending = true))
        assertEquals(ButtonFace("機器なし", R.drawable.ic_error, Tone.FAILED),
            buttonFace("仕事", R.drawable.ic_power, boosted = true, label = LightsMessage("機器なし", failed = true)))
        assertEquals(ButtonFace("受付済み", R.drawable.ic_success, Tone.PLAIN),
            buttonFace("解錠", R.drawable.ic_unlock, boosted = null, label = LightsMessage("受付済み", failed = false)))
    }
}
