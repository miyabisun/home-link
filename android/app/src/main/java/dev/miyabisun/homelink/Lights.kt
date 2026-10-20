package dev.miyabisun.homelink

/** What switching a label's devices did: `missing` names ledger devices matterjs-server does not serve. */
data class SwitchResult(val switched: Int, val noResponse: Int, val failed: Int, val missing: List<String>)

/** A result line; `failed` when any device was left behind or the request failed. */
data class LightsMessage(val text: String, val failed: Boolean)

internal const val CHECK_ENDPOINT = "接続先の設定と通信を確かめてください"
internal const val NOT_CONFIGURED = "接続先が未設定です。アプリの「接続先の設定」でhome-linkのURLを入れてください"
internal const val NOT_CONFIGURED_LABEL = "接続先未設定"

internal fun failure(error: ApiError, host: String) = LightsMessage(when (error) {
    ApiError.NOT_CONFIGURED -> NOT_CONFIGURED
    ApiError.UNREACHABLE -> "home-link（$host）に接続できません。$CHECK_ENDPOINT"
    ApiError.MATTER_UNREACHABLE -> "照明のサーバー（matterjs-server）に接続できません"
    ApiError.MATTER_NOT_CONFIGURED -> "home-linkに照明のサーバーが設定されていません"
    else -> "home-linkでエラーが発生しました。時間をおいてお試しください"
}, failed = true)

internal fun failureLabel(error: ApiError) = LightsMessage(when (error) {
    ApiError.NOT_CONFIGURED -> NOT_CONFIGURED_LABEL
    ApiError.UNREACHABLE, ApiError.MATTER_UNREACHABLE -> "接続不可"
    ApiError.MATTER_NOT_CONFIGURED -> "未設定"
    ApiError.LABEL_NOT_FOUND -> "ラベルなし"
    ApiError.LABEL_HAS_NO_DEVICES -> "機器なし"
    else -> "エラー"
}, failed = true)

/** How a widget button reads: [Tone.ACTIVE] fills it with the accent, [Tone.FAILED] words it in the failure colour. */
enum class Tone { PLAIN, ACTIVE, FAILED }

/** The words, icon and tone a widget button shows. */
data class ButtonFace(val text: String, val icon: Int, val tone: Tone)

/**
 * The face of the widget button `name` with `icon`: a press's short `label` (keeping `icon` while `sending`),
 * else "<name>中" with its own icon while the server says its label is `boosted`. An unknown (null)
 * state reads as off, so the widget never claims a state it could not read.
 */
fun buttonFace(name: String, icon: Int, boosted: Boolean?, label: LightsMessage?, sending: Boolean = false): ButtonFace = when {
    label == null -> if (boosted == true) ButtonFace("${name}中", R.drawable.ic_boosted, Tone.ACTIVE) else ButtonFace(name, icon, Tone.PLAIN)
    sending -> ButtonFace(label.text, icon, Tone.PLAIN)
    label.failed -> ButtonFace(label.text, R.drawable.ic_error, Tone.FAILED)
    else -> ButtonFace(label.text, R.drawable.ic_success, Tone.PLAIN)
}
