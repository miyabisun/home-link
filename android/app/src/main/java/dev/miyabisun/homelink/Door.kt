package dev.miyabisun.homelink

/** What a door widget has the SESAME do: the API path's last segment and the word for it. */
enum class DoorCommand(val path: String, val verb: String) {
    UNLOCK("unlock", "解錠"),
    LOCK("lock", "施錠"),
}

/** Words a door `command`'s result; acceptance by CANDY HOUSE, not a door seen moving. */
fun doorMessage(command: DoorCommand, result: ApiResult<Unit>, host: String): LightsMessage = when (result) {
    is ApiResult.Ok -> LightsMessage("SESAMEが${command.verb}を受け付けました", failed = false)
    is ApiResult.Failed -> when (result.error) {
        ApiError.TIMEOUT -> LightsMessage("home-link（$host）から時間内に応答がありませんでした。$CHECK_ENDPOINT", failed = true)
        ApiError.SESAME_NOT_CONFIGURED -> LightsMessage("home-linkにSESAMEが設定されていません", failed = true)
        ApiError.SESAME_REJECTED -> LightsMessage("SESAMEのAPIが${command.verb}を受け付けませんでした", failed = true)
        ApiError.SESAME_UNREACHABLE -> LightsMessage("SESAMEのAPIに接続できませんでした", failed = true)
        else -> failure(result.error, host)
    }
}

/** The short label a door widget shows on its button; `failed` as in [doorMessage]. */
fun doorLabel(result: ApiResult<Unit>): LightsMessage = when (result) {
    is ApiResult.Ok -> LightsMessage("受付済み", failed = false)
    is ApiResult.Failed -> LightsMessage(when (result.error) {
        ApiError.NOT_CONFIGURED -> NOT_CONFIGURED_LABEL
        ApiError.UNREACHABLE -> "接続不可"
        ApiError.TIMEOUT -> "時間切れ"
        ApiError.SESAME_NOT_CONFIGURED -> "未設定"
        ApiError.SESAME_REJECTED -> "拒否"
        ApiError.SESAME_UNREACHABLE -> "API不通"
        else -> "エラー"
    }, failed = true)
}
