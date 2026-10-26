package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Test

class DoorMessageTest {
    private val failures = listOf(
        ApiError.UNREACHABLE, ApiError.TIMEOUT, ApiError.NOT_CONFIGURED, ApiError.SESAME_NOT_CONFIGURED,
        ApiError.SESAME_REJECTED, ApiError.SESAME_UNREACHABLE, ApiError.SERVER, ApiError.NO_NETWORK, ApiError.BLOCKED,
    )

    @Test fun anAcceptedCommandIsASuccessWithoutClaimingTheDoorMoved() {
        assertEquals(LightsMessage("受付済み", failed = false), doorLabel(ApiResult.Ok(Unit)))
        assertEquals(LightsMessage("SESAMEが解錠を受け付けました", failed = false),
            doorMessage(DoorCommand.UNLOCK, ApiResult.Ok(Unit), "home-link.example:5011"))
        assertEquals(LightsMessage("SESAMEが施錠を受け付けました", failed = false),
            doorMessage(DoorCommand.LOCK, ApiResult.Ok(Unit), "home-link.example:5011"))
    }

    @Test fun eachFailureHasItsOwnShortLabel() {
        assertEquals(
            listOf("接続不可", "結果不明", "接続先なし", "SESAME未設定", "SESAME拒否", "SESAME不通", "エラー", "回線なし", "通信制限"),
            failures.map { doorLabel(ApiResult.Failed(it)).text },
        )
        failures.forEach { assert(doorLabel(ApiResult.Failed(it)).failed) }
    }

    @Test fun eachFailureIsWordedWithWhatToCheck() {
        assertEquals(
            listOf(
                "home-link（home-link.example:5011）に接続できません。接続先の設定と通信を確かめてください",
                "home-link（home-link.example:5011）に送りましたが、時間内に応答がありませんでした。届いたかは分かりません",
                "接続先が未設定です。アプリの「接続先の設定」でhome-linkのURLを入れてください",
                "home-linkにSESAMEが設定されていません",
                "SESAMEのAPIが施錠を受け付けませんでした",
                "SESAMEのAPIに接続できませんでした",
                "home-linkでエラーが発生しました。時間をおいてお試しください",
                "電話に使える回線が無く、送れませんでした",
                "電話の通信制限（データセーバーなど）で送れませんでした",
            ),
            failures.map { doorMessage(DoorCommand.LOCK, ApiResult.Failed(it), "home-link.example:5011").text },
        )
        failures.forEach { assert(doorMessage(DoorCommand.UNLOCK, ApiResult.Failed(it), "h").failed) }
    }
}
