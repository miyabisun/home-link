package dev.miyabisun.homelink

/** What switching every light did: `missing` names ledger devices matterjs-server does not serve. */
data class LightsResult(val switched: Int, val noResponse: Int, val failed: Int, val missing: List<String>)

/** The result line of a switch; `failed` when any light was left behind or the request failed. */
data class LightsMessage(val text: String, val failed: Boolean)

/**
 * Words the result of switching every light `on` or off. With `detail` it counts the lights and names
 * the missing devices; without, as on the widget, it only tells success from lights left behind.
 */
fun lightsMessage(on: Boolean, result: ApiResult<LightsResult>, host: String, detail: Boolean): LightsMessage {
    val verb = if (on) "オン" else "オフ"
    val lights = when (result) {
        is ApiResult.Failed -> return LightsMessage(when (result.error) {
            ApiError.UNREACHABLE -> "home-link（$host）に接続できません。Tailscaleの接続を確認してください"
            ApiError.MATTER_UNREACHABLE -> "照明のサーバー（matterjs-server）に接続できません"
            ApiError.MATTER_NOT_CONFIGURED -> "home-linkに照明のサーバーが設定されていません"
            else -> "home-linkでエラーが発生しました。時間をおいてお試しください"
        }, failed = true)
        is ApiResult.Ok -> result.value
    }
    if (!detail) {
        val left = lights.noResponse + lights.failed + lights.missing.size
        return when {
            lights.switched + left == 0 -> LightsMessage("照明が見つかりません", failed = true)
            left == 0 -> LightsMessage("照明を${verb}にしました", failed = false)
            lights.switched > 0 -> LightsMessage("一部の照明を${verb}にできませんでした", failed = true)
            else -> LightsMessage("照明を${verb}にできませんでした", failed = true)
        }
    }
    val missing = lights.missing.size.takeIf { it > 0 }?.let { count ->
        "見つからない${count}台" + lights.missing.joinToString("、", "（", "）")
    }
    val left = listOfNotNull(
        lights.noResponse.takeIf { it > 0 }?.let { "応答なし${it}台" },
        lights.failed.takeIf { it > 0 }?.let { "失敗${it}台" },
        missing,
    ).joinToString("・")
    val done = when {
        lights.switched > 0 -> "照明を${lights.switched}台${verb}にしました"
        left.isEmpty() -> return LightsMessage("照明が見つかりません", failed = true)
        else -> "照明を${verb}にできませんでした"
    }
    return if (left.isEmpty()) LightsMessage(done, failed = false) else LightsMessage("$done。$left", failed = true)
}

/** The short label a one-row widget shows on the pressed button; `failed` as in [lightsMessage]. */
fun lightsLabel(on: Boolean, result: ApiResult<LightsResult>): LightsMessage {
    val lights = when (result) {
        is ApiResult.Failed -> return LightsMessage(when (result.error) {
            ApiError.UNREACHABLE, ApiError.MATTER_UNREACHABLE -> "接続不可"
            ApiError.MATTER_NOT_CONFIGURED -> "未設定"
            else -> "エラー"
        }, failed = true)
        is ApiResult.Ok -> result.value
    }
    val total = lights.switched + lights.noResponse + lights.failed + lights.missing.size
    return when {
        total == 0 -> LightsMessage("照明なし", failed = true)
        lights.switched == total -> LightsMessage("${if (on) "オン" else "オフ"}完了", failed = false)
        lights.switched > 0 -> LightsMessage("一部失敗", failed = true)
        else -> LightsMessage("失敗", failed = true)
    }
}
