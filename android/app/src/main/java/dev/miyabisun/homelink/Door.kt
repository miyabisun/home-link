package dev.miyabisun.homelink

/** Words an unlock of the front door; acceptance by CANDY HOUSE, not a door seen open. */
fun doorMessage(result: ApiResult<Unit>, host: String): LightsMessage = when (result) {
    is ApiResult.Ok -> LightsMessage("SESAMEが解錠を受け付けました", failed = false)
    is ApiResult.Failed -> when (result.error) {
        ApiError.NOT_TAILNET -> LightsMessage("解錠はtailnetからだけ受け付けます。Tailscaleの接続を確認してください", failed = true)
        ApiError.SESAME_NOT_CONFIGURED -> LightsMessage("home-linkにSESAMEが設定されていません", failed = true)
        ApiError.SESAME_REJECTED -> LightsMessage("SESAMEのAPIが解錠を受け付けませんでした", failed = true)
        ApiError.SESAME_UNREACHABLE -> LightsMessage("SESAMEのAPIに接続できませんでした", failed = true)
        else -> failure(result.error, host)
    }
}

/** The short label the unlock widget shows on its button; `failed` as in [doorMessage]. */
fun doorLabel(result: ApiResult<Unit>): LightsMessage = when (result) {
    is ApiResult.Ok -> LightsMessage("受付済み", failed = false)
    is ApiResult.Failed -> LightsMessage(when (result.error) {
        ApiError.UNREACHABLE -> "接続不可"
        ApiError.NOT_TAILNET -> "tailnet外"
        ApiError.SESAME_NOT_CONFIGURED -> "未設定"
        ApiError.SESAME_REJECTED -> "拒否"
        ApiError.SESAME_UNREACHABLE -> "API不通"
        else -> "エラー"
    }, failed = true)
}
