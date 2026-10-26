package dev.miyabisun.homelink

/** What switching a label's devices did: `missing` names ledger devices matterjs-server does not serve. */
data class SwitchResult(val switched: Int, val noResponse: Int, val failed: Int, val missing: List<String>)

/** A result line; `failed` when any device was left behind or the request failed. */
data class LightsMessage(val text: String, val failed: Boolean)

internal const val CHECK_ENDPOINT = "接続先の設定と通信を確かめてください"
internal const val NOT_CONFIGURED = "接続先が未設定です。アプリの「接続先の設定」でhome-linkのURLを入れてください"
// Short labels fit five full-width characters, the narrowest button's width.
internal const val NOT_CONFIGURED_LABEL = "接続先なし"

/** What a pressed button shows until the answer. */
internal val SENDING = LightsMessage("送信中…", failed = false)

/**
 * What the app says when a request failed, the one wording for every screen and widget. Where it stopped reads
 * apart: on the phone; on the way to home-link (no connection, or none in time); sent with no answer, so whether
 * it acted is unknown; at something answering in home-link's place; or behind home-link.
 */
internal fun failure(error: ApiError, host: String) = LightsMessage(when (error) {
    ApiError.NOT_CONFIGURED -> NOT_CONFIGURED
    ApiError.SETTINGS_UNREADABLE -> "接続先の設定ファイルを読めませんでした。アプリの「接続先の設定」で保存し直してください"
    ApiError.SETTINGS_MALFORMED -> "接続先の設定ファイルの書き方が正しくありません。アプリの「接続先の設定」で直してください"
    ApiError.LOCAL_NETWORK_DENIED -> "LANへの接続が許可されていません。アプリを開いて「LANへの接続を許可」を押してください"
    ApiError.NO_NETWORK -> "電話に使える回線が無く、送れませんでした"
    ApiError.BLOCKED -> "電話の通信制限（データセーバーなど）で送れませんでした"
    ApiError.UNREACHABLE -> "home-link（$host）に接続できません。$CHECK_ENDPOINT"
    ApiError.CONNECT_TIMEOUT -> "home-link（$host）に時間内に接続できませんでした。接続先の設定と通信（VPNなど）を確かめてください"
    ApiError.TIMEOUT -> "home-link（$host）に送りましたが、時間内に応答がありませんでした。届いたかは分かりません"
    ApiError.HTTP_DENIED -> "home-link（$host）への要求が拒否されました。間にあるプロキシや認証の設定を確かめてください"
    ApiError.HTTP_NOT_FOUND -> "home-link（$host）にこの操作の窓口がありません。接続先のURLとhome-linkの版を確かめてください"
    ApiError.HTTP_GATEWAY -> "home-link（$host）の手前の中継がエラーを返しました。home-linkが動いているか確かめてください"
    ApiError.BAD_ANSWER -> "home-link（$host）から読めない応答が返りました。接続先がhome-linkか確かめてください"
    ApiError.MATTER_UNREACHABLE -> "照明のサーバー（matterjs-server）に接続できません"
    ApiError.MATTER_NOT_CONFIGURED -> "home-linkに照明のサーバーが設定されていません"
    ApiError.SESAME_NOT_CONFIGURED -> "home-linkにSESAMEが設定されていません"
    ApiError.SESAME_REJECTED -> "SESAMEのAPIが受け付けませんでした"
    ApiError.SESAME_UNREACHABLE -> "SESAMEのAPIに接続できませんでした"
    ApiError.LABEL_NOT_FOUND -> "ラベルがありません"
    ApiError.LABEL_HAS_NO_DEVICES -> "ラベルに機器がありません"
    else -> "home-linkでエラーが発生しました。時間をおいてお試しください"
}, failed = true)

/** The short label a widget shows for a failed press, one for each wording of [failure]. */
internal fun failureLabel(error: ApiError) = LightsMessage(when (error) {
    ApiError.NOT_CONFIGURED -> NOT_CONFIGURED_LABEL
    ApiError.SETTINGS_UNREADABLE -> "設定読めず"
    ApiError.SETTINGS_MALFORMED -> "設定不正"
    ApiError.LOCAL_NETWORK_DENIED -> "LAN未許可"
    ApiError.NO_NETWORK -> "回線なし"
    ApiError.BLOCKED -> "通信制限"
    ApiError.UNREACHABLE -> "接続不可"
    ApiError.CONNECT_TIMEOUT -> "時間切れ"
    ApiError.TIMEOUT -> "結果不明"
    ApiError.HTTP_DENIED -> "要求拒否"
    ApiError.HTTP_NOT_FOUND -> "URL違い"
    ApiError.HTTP_GATEWAY -> "中継エラー"
    ApiError.BAD_ANSWER -> "応答不正"
    ApiError.MATTER_UNREACHABLE -> "Matter不通"
    ApiError.MATTER_NOT_CONFIGURED -> "Matter未設定"
    ApiError.SESAME_NOT_CONFIGURED -> "SESAME未設定"
    ApiError.SESAME_REJECTED -> "SESAME拒否"
    ApiError.SESAME_UNREACHABLE -> "SESAME不通"
    ApiError.LABEL_NOT_FOUND -> "ラベルなし"
    ApiError.LABEL_HAS_NO_DEVICES -> "機器なし"
    else -> "エラー"
}, failed = true)

/** How a widget button reads: [Tone.ACTIVE] fills it with the accent, [Tone.FAILED] words it in the failure colour. */
enum class Tone { PLAIN, ACTIVE, FAILED }

/** The words, icon and tone a widget button shows, and what is read aloud for it. */
data class ButtonFace(val text: String, val icon: Int, val tone: Tone, val description: String = text)

/** A switch's state as last known: [UNKNOWN] when it could not be read or a press may have changed it unseen. */
enum class Toggle {
    ON, OFF, UNKNOWN;

    companion object {
        fun of(on: Boolean?) = when (on) {
            true -> ON
            false -> OFF
            null -> UNKNOWN
        }
    }
}

/**
 * The face of the widget button `name` with `icon`: a press's short `label` (keeping `icon` while `sending`),
 * else, for a switch with a `state`, "<name>中" with `activeIcon` while on and a question mark while unknown,
 * so the widget never shows a state it does not know as off. A button with no state (null) reads as its name.
 */
fun buttonFace(name: String, icon: Int, state: Toggle?, label: LightsMessage?, sending: Boolean = false,
               activeIcon: Int = R.drawable.ic_boosted): ButtonFace = when {
    label == null -> when (state) {
        Toggle.ON -> ButtonFace("${name}中", activeIcon, Tone.ACTIVE)
        Toggle.UNKNOWN -> ButtonFace(name, R.drawable.ic_unknown, Tone.PLAIN, "${name}（状態不明）")
        else -> ButtonFace(name, icon, Tone.PLAIN)
    }
    sending -> ButtonFace(label.text, icon, Tone.PLAIN)
    label.failed -> ButtonFace(label.text, R.drawable.ic_error, Tone.FAILED)
    else -> ButtonFace(label.text, R.drawable.ic_success, Tone.PLAIN)
}
