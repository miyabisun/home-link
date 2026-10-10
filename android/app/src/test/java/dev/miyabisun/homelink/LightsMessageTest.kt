package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Test

class LightsMessageTest {
    private fun message(on: Boolean, result: ApiResult<LightsResult>, detail: Boolean = true) =
        lightsMessage(on, result, "homeserver:5011", detail)

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
        assertEquals(LightsMessage("照明をオンにできませんでした。応答なし3台", failed = true),
            message(true, ApiResult.Ok(LightsResult(0, 3, 0, emptyList()))))
        assertEquals(LightsMessage("照明が見つかりません", failed = true),
            message(true, ApiResult.Ok(LightsResult(0, 0, 0, emptyList()))))
    }

    @Test fun theWidgetLineCountsNothing() {
        assertEquals(LightsMessage("照明をオンにしました", failed = false),
            message(true, ApiResult.Ok(LightsResult(12, 0, 0, emptyList())), detail = false))
        assertEquals(LightsMessage("照明をオフにしました", failed = false),
            message(false, ApiResult.Ok(LightsResult(12, 0, 0, emptyList())), detail = false))
        assertEquals(LightsMessage("一部の照明をオフにできませんでした", failed = true),
            message(false, ApiResult.Ok(LightsResult(8, 2, 1, listOf("台所", "通路"))), detail = false))
        assertEquals(LightsMessage("一部の照明をオンにできませんでした", failed = true),
            message(true, ApiResult.Ok(LightsResult(3, 0, 0, listOf("通路"))), detail = false))
        assertEquals(LightsMessage("照明をオンにできませんでした", failed = true),
            message(true, ApiResult.Ok(LightsResult(0, 3, 0, emptyList())), detail = false))
        assertEquals(LightsMessage("照明が見つかりません", failed = true),
            message(true, ApiResult.Ok(LightsResult(0, 0, 0, emptyList())), detail = false))
        assertEquals(LightsMessage("照明のサーバー（matterjs-server）に接続できません", failed = true),
            message(true, ApiResult.Failed(ApiError.MATTER_UNREACHABLE), detail = false))
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

    @Test fun theOneRowLabelIsShortCountsNothingAndKeepsLeftoversAFailure() {
        assertEquals(LightsMessage("オン完了", failed = false), lightsLabel(true, ApiResult.Ok(LightsResult(12, 0, 0, emptyList()))))
        assertEquals(LightsMessage("オフ完了", failed = false), lightsLabel(false, ApiResult.Ok(LightsResult(12, 0, 0, emptyList()))))
        assertEquals(LightsMessage("一部失敗", failed = true),
            lightsLabel(false, ApiResult.Ok(LightsResult(8, 2, 1, listOf("台所", "通路")))))
        assertEquals(LightsMessage("一部失敗", failed = true), lightsLabel(true, ApiResult.Ok(LightsResult(3, 0, 0, listOf("通路")))))
        assertEquals(LightsMessage("失敗", failed = true), lightsLabel(true, ApiResult.Ok(LightsResult(0, 3, 0, emptyList()))))
        assertEquals(LightsMessage("照明なし", failed = true), lightsLabel(true, ApiResult.Ok(LightsResult(0, 0, 0, emptyList()))))
    }

    @Test fun theOneRowLabelNamesTheFailure() {
        assertEquals(LightsMessage("接続不可", failed = true), lightsLabel(true, ApiResult.Failed(ApiError.UNREACHABLE)))
        assertEquals(LightsMessage("接続不可", failed = true), lightsLabel(true, ApiResult.Failed(ApiError.MATTER_UNREACHABLE)))
        assertEquals(LightsMessage("未設定", failed = true), lightsLabel(false, ApiResult.Failed(ApiError.MATTER_NOT_CONFIGURED)))
        assertEquals(LightsMessage("エラー", failed = true), lightsLabel(false, ApiResult.Failed(ApiError.SERVER)))
    }

    private val all = LightsResult(2, 0, 0, emptyList())
    private fun boost(result: ApiResult<BoostResult>) = boostMessage("作業", result, "homeserver:5011")

    @Test fun aBoostIsToldFromARelease() {
        assertEquals(LightsMessage("作業の照明を明るくしました", failed = false), boost(ApiResult.Ok(BoostResult(true, all))))
        assertEquals(LightsMessage("作業の照明を元に戻しました", failed = false), boost(ApiResult.Ok(BoostResult(false, all))))
        assertEquals(LightsMessage("一部の作業の照明を明るくできませんでした", failed = true),
            boost(ApiResult.Ok(BoostResult(true, LightsResult(1, 1, 0, emptyList())))))
        assertEquals(LightsMessage("一部の作業の照明を元に戻せませんでした", failed = true),
            boost(ApiResult.Ok(BoostResult(false, LightsResult(1, 0, 0, listOf("デスク"))))))
        assertEquals(LightsMessage("作業の照明を明るくできませんでした", failed = true),
            boost(ApiResult.Ok(BoostResult(true, LightsResult(0, 2, 0, emptyList())))))
        assertEquals(LightsMessage("作業の照明を元に戻せませんでした", failed = true),
            boost(ApiResult.Ok(BoostResult(false, LightsResult(0, 0, 2, emptyList())))))
    }

    @Test fun aBoostNamesAMissingLabelAndOtherFailures() {
        assertEquals(LightsMessage("label「作業」がありません", failed = true), boost(ApiResult.Failed(ApiError.LABEL_NOT_FOUND)))
        assertEquals(LightsMessage("label「作業」に照明がありません", failed = true),
            boost(ApiResult.Failed(ApiError.LABEL_HAS_NO_LIGHTS)))
        assertEquals(LightsMessage("home-link（homeserver:5011）に接続できません。Tailscaleの接続を確認してください", failed = true),
            boost(ApiResult.Failed(ApiError.UNREACHABLE)))
        assertEquals(LightsMessage("照明のサーバー（matterjs-server）に接続できません", failed = true),
            boost(ApiResult.Failed(ApiError.MATTER_UNREACHABLE)))
    }

    @Test fun theOneRowBoostLabelTellsOnFromReleased() {
        assertEquals(LightsMessage("オン完了", failed = false), boostLabel(ApiResult.Ok(BoostResult(true, all))))
        assertEquals(LightsMessage("解除完了", failed = false), boostLabel(ApiResult.Ok(BoostResult(false, all))))
        assertEquals(LightsMessage("一部失敗", failed = true),
            boostLabel(ApiResult.Ok(BoostResult(true, LightsResult(1, 0, 0, listOf("デスク"))))))
        assertEquals(LightsMessage("失敗", failed = true), boostLabel(ApiResult.Ok(BoostResult(true, LightsResult(0, 2, 0, emptyList())))))
        assertEquals(LightsMessage("ラベルなし", failed = true), boostLabel(ApiResult.Failed(ApiError.LABEL_NOT_FOUND)))
        assertEquals(LightsMessage("照明なし", failed = true), boostLabel(ApiResult.Failed(ApiError.LABEL_HAS_NO_LIGHTS)))
        assertEquals(LightsMessage("接続不可", failed = true), boostLabel(ApiResult.Failed(ApiError.UNREACHABLE)))
        assertEquals(LightsMessage("エラー", failed = true), boostLabel(ApiResult.Failed(ApiError.SERVER)))
    }
}
