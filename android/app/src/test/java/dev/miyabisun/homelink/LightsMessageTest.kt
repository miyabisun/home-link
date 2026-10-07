package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Test

class LightsMessageTest {
    private fun message(on: Boolean, result: ApiResult<LightsResult>, names: Boolean = true) =
        lightsMessage(on, result, "homeserver:5011", names)

    @Test fun allSwitchedIsASuccess() {
        assertEquals(LightsMessage("照明を11台オンにしました", failed = false),
            message(true, ApiResult.Ok(LightsResult(11, 0, 0, emptyList()))))
        assertEquals(LightsMessage("照明を11台オフにしました", failed = false),
            message(false, ApiResult.Ok(LightsResult(11, 0, 0, emptyList()))))
    }

    @Test fun lightsLeftBehindAreCountedAndNotShownAsSuccess() {
        val partial = LightsResult(8, 2, 1, listOf("台所", "通路"))
        assertEquals(LightsMessage("照明を8台オフにしました。応答なし2台・失敗1台・見つからない2台（台所、通路）", failed = true),
            message(false, ApiResult.Ok(partial)))
        assertEquals(LightsMessage("照明を8台オフにしました。応答なし2台・失敗1台・見つからない2台", failed = true),
            message(false, ApiResult.Ok(partial), names = false))
        assertEquals(LightsMessage("照明をオンにできませんでした。応答なし3台", failed = true),
            message(true, ApiResult.Ok(LightsResult(0, 3, 0, emptyList()))))
        assertEquals(LightsMessage("照明が見つかりません", failed = true),
            message(true, ApiResult.Ok(LightsResult(0, 0, 0, emptyList()))))
    }

    @Test fun failuresNameWhatToCheck() {
        assertEquals(LightsMessage("home-link（homeserver:5011）に接続できません。Tailscaleの接続を確認してください", failed = true),
            message(true, ApiResult.Failed(ApiError.UNREACHABLE)))
        assertEquals(LightsMessage("照明のサーバー（matterjs-server）に接続できません", failed = true),
            message(true, ApiResult.Failed(ApiError.MATTER_UNREACHABLE)))
        assertEquals(LightsMessage("home-linkに照明のサーバーが設定されていません", failed = true),
            message(false, ApiResult.Failed(ApiError.MATTER_NOT_CONFIGURED)))
        assertEquals(LightsMessage("home-linkでエラーが発生しました。時間をおいてお試しください", failed = true),
            message(false, ApiResult.Failed(ApiError.SERVER)))
    }

    @Test fun theOneRowLabelIsShortAndKeepsLeftoversAFailure() {
        assertEquals(LightsMessage("11台オン", failed = false), lightsLabel(true, ApiResult.Ok(LightsResult(11, 0, 0, emptyList()))))
        assertEquals(LightsMessage("11台オフ", failed = false), lightsLabel(false, ApiResult.Ok(LightsResult(11, 0, 0, emptyList()))))
        assertEquals(LightsMessage("8/13台", failed = true),
            lightsLabel(false, ApiResult.Ok(LightsResult(8, 2, 1, listOf("台所", "通路")))))
        assertEquals(LightsMessage("0/3台", failed = true), lightsLabel(true, ApiResult.Ok(LightsResult(0, 3, 0, emptyList()))))
        assertEquals(LightsMessage("照明なし", failed = true), lightsLabel(true, ApiResult.Ok(LightsResult(0, 0, 0, emptyList()))))
    }

    @Test fun theOneRowLabelNamesTheFailure() {
        assertEquals(LightsMessage("接続不可", failed = true), lightsLabel(true, ApiResult.Failed(ApiError.UNREACHABLE)))
        assertEquals(LightsMessage("接続不可", failed = true), lightsLabel(true, ApiResult.Failed(ApiError.MATTER_UNREACHABLE)))
        assertEquals(LightsMessage("未設定", failed = true), lightsLabel(false, ApiResult.Failed(ApiError.MATTER_NOT_CONFIGURED)))
        assertEquals(LightsMessage("エラー", failed = true), lightsLabel(false, ApiResult.Failed(ApiError.SERVER)))
    }
}
