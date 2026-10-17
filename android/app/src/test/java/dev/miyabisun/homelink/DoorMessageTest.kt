package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Test

class DoorMessageTest {
    private val failures = listOf(
        ApiError.UNREACHABLE, ApiError.TIMEOUT, ApiError.NOT_CONFIGURED, ApiError.SESAME_NOT_CONFIGURED,
        ApiError.SESAME_REJECTED, ApiError.SESAME_UNREACHABLE, ApiError.SERVER,
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
            listOf("接続不可", "時間切れ", "接続先未設定", "未設定", "拒否", "API不通", "エラー"),
            failures.map { doorLabel(ApiResult.Failed(it)).text },
        )
        failures.forEach { assert(doorLabel(ApiResult.Failed(it)).failed) }
    }

    @Test fun eachFailureIsWordedWithWhatToCheck() {
        assertEquals(
            listOf(
                "home-link（home-link.example:5011）に接続できません。接続先の設定と通信を確かめてください",
                "home-link（home-link.example:5011）から時間内に応答がありませんでした。接続先の設定と通信を確かめてください",
                "接続先が未設定です。アプリの「接続先の設定」でhome-linkのURLを入れてください",
                "home-linkにSESAMEが設定されていません",
                "SESAMEのAPIが施錠を受け付けませんでした",
                "SESAMEのAPIに接続できませんでした",
                "home-linkでエラーが発生しました。時間をおいてお試しください",
            ),
            failures.map { doorMessage(DoorCommand.LOCK, ApiResult.Failed(it), "home-link.example:5011").text },
        )
        failures.forEach { assert(doorMessage(DoorCommand.UNLOCK, ApiResult.Failed(it), "h").failed) }
    }
}
