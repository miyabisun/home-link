package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Test

class DoorMessageTest {
    private val failures = listOf(
        ApiError.UNREACHABLE, ApiError.NOT_TAILNET, ApiError.SESAME_NOT_CONFIGURED,
        ApiError.SESAME_REJECTED, ApiError.SESAME_UNREACHABLE, ApiError.SERVER,
    )

    @Test fun anAcceptedUnlockIsASuccessWithoutClaimingTheDoorIsOpen() {
        assertEquals(LightsMessage("受付済み", failed = false), doorLabel(ApiResult.Ok(Unit)))
        assertEquals(LightsMessage("SESAMEが解錠を受け付けました", failed = false),
            doorMessage(ApiResult.Ok(Unit), "homeserver:5011"))
    }

    @Test fun eachFailureHasItsOwnShortLabel() {
        assertEquals(
            listOf("接続不可", "tailnet外", "未設定", "拒否", "API不通", "エラー"),
            failures.map { doorLabel(ApiResult.Failed(it)).text },
        )
        failures.forEach { assert(doorLabel(ApiResult.Failed(it)).failed) }
    }

    @Test fun eachFailureIsWordedWithWhatToCheck() {
        assertEquals(
            listOf(
                "home-link（homeserver:5011）に接続できません。Tailscaleの接続を確認してください",
                "解錠はtailnetからだけ受け付けます。Tailscaleの接続を確認してください",
                "home-linkにSESAMEが設定されていません",
                "SESAMEのAPIが解錠を受け付けませんでした",
                "SESAMEのAPIに接続できませんでした",
                "home-linkでエラーが発生しました。時間をおいてお試しください",
            ),
            failures.map { doorMessage(ApiResult.Failed(it), "homeserver:5011").text },
        )
        failures.forEach { assert(doorMessage(ApiResult.Failed(it), "h").failed) }
    }
}
