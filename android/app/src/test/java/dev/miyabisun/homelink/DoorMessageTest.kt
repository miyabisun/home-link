package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Test

class DoorMessageTest {
    private val failures = listOf(
        ApiError.UNREACHABLE, ApiError.TIMEOUT, ApiError.NOT_TAILNET, ApiError.SESAME_NOT_CONFIGURED,
        ApiError.SESAME_REJECTED, ApiError.SESAME_UNREACHABLE, ApiError.SERVER,
    )

    @Test fun anAcceptedCommandIsASuccessWithoutClaimingTheDoorMoved() {
        assertEquals(LightsMessage("受付済み", failed = false), doorLabel(ApiResult.Ok(Unit)))
        assertEquals(LightsMessage("SESAMEが解錠を受け付けました", failed = false),
            doorMessage(DoorCommand.UNLOCK, ApiResult.Ok(Unit), "homeserver:5011"))
        assertEquals(LightsMessage("SESAMEが施錠を受け付けました", failed = false),
            doorMessage(DoorCommand.LOCK, ApiResult.Ok(Unit), "homeserver:5011"))
    }

    @Test fun eachFailureHasItsOwnShortLabel() {
        assertEquals(
            listOf("接続不可", "時間切れ", "tailnet外", "未設定", "拒否", "API不通", "エラー"),
            failures.map { doorLabel(ApiResult.Failed(it)).text },
        )
        failures.forEach { assert(doorLabel(ApiResult.Failed(it)).failed) }
    }

    @Test fun eachFailureIsWordedWithWhatToCheck() {
        assertEquals(
            listOf(
                "home-link（homeserver:5011）に接続できません。Tailscaleの接続を確認してください",
                "home-link（homeserver:5011）から時間内に応答がありませんでした。Tailscaleの接続を確認してください",
                "施錠はtailnetからだけ受け付けます。Tailscaleの接続を確認してください",
                "home-linkにSESAMEが設定されていません",
                "SESAMEのAPIが施錠を受け付けませんでした",
                "SESAMEのAPIに接続できませんでした",
                "home-linkでエラーが発生しました。時間をおいてお試しください",
            ),
            failures.map { doorMessage(DoorCommand.LOCK, ApiResult.Failed(it), "homeserver:5011").text },
        )
        assertEquals("解錠はtailnetからだけ受け付けます。Tailscaleの接続を確認してください",
            doorMessage(DoorCommand.UNLOCK, ApiResult.Failed(ApiError.NOT_TAILNET), "h").text)
        failures.forEach { assert(doorMessage(DoorCommand.UNLOCK, ApiResult.Failed(it), "h").failed) }
    }
}
