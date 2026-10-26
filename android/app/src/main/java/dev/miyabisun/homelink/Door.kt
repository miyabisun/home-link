package dev.miyabisun.homelink

/** What a door widget has the SESAME do: the API path's last segment and the word for it. */
enum class DoorCommand(val path: String, val verb: String) {
    UNLOCK("unlock", "解錠"),
    LOCK("lock", "施錠"),
}

/** Words a door `command`'s result; acceptance by CANDY HOUSE, not a door seen moving. */
fun doorMessage(command: DoorCommand, result: ApiResult<Unit>, host: String): LightsMessage = when (result) {
    is ApiResult.Ok -> LightsMessage("SESAMEが${command.verb}を受け付けました", failed = false)
    is ApiResult.Failed ->
        if (result.error == ApiError.SESAME_REJECTED) LightsMessage("SESAMEのAPIが${command.verb}を受け付けませんでした", failed = true)
        else failure(result.error, host)
}

/** The short label a door widget shows on its button; `failed` as in [doorMessage]. */
fun doorLabel(result: ApiResult<Unit>): LightsMessage = when (result) {
    is ApiResult.Ok -> LightsMessage("受付済み", failed = false)
    is ApiResult.Failed -> failureLabel(result.error)
}
