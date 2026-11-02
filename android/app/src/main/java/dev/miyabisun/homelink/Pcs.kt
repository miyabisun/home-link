package dev.miyabisun.homelink

/** A PC connected to home-link by its name; [sleeping] is null while the PC does not say. */
data class Pc(val name: String, val sleeping: Boolean?)

/** What a PC widget sends: the API path's last segment and the word for it. */
enum class PcAction(val path: String, val verb: String) {
    SLEEP("sleep", "スリープ"),
    WAKE("wake", "復帰"),
}

private const val SLEEPING = "スリープ中"

/**
 * How a PC's button reads: "スリープ中" on the accent with its own icon while the PC says it sleeps, its name
 * while awake, unknown while not connected or never read, and a press's short result instead.
 */
fun pcFace(name: String, sleeping: Boolean?, press: LightsMessage?, sending: Boolean = false): ButtonFace =
    if (press == null && sleeping == true) ButtonFace(SLEEPING, R.drawable.ic_sleep, Tone.ACTIVE, "${name}（$SLEEPING）")
    else buttonFace(name, R.drawable.ic_power, Toggle.of(sleeping), press, sending)

/** What a press sends: a sleeping PC wakes; any other, unknown included, sleeps. */
fun pcAction(sleeping: Boolean?): PcAction = if (sleeping == true) PcAction.WAKE else PcAction.SLEEP

/** What a press shows on the button: none when the PC says its new state, as the button then shows it. */
fun pcPress(result: ApiResult<Boolean?>): LightsMessage? = when (result) {
    is ApiResult.Ok -> if (result.value == null) LightsMessage("受付済み", failed = false) else null
    is ApiResult.Failed -> failureLabel(result.error)
}

/** What is read aloud for a press's result; none when the button shows the new state. */
fun pcMessage(name: String, action: PcAction, result: ApiResult<Boolean?>, host: String): String? = when (result) {
    is ApiResult.Ok -> if (result.value == null) "${name}が${action.verb}を受け付けました" else null
    is ApiResult.Failed -> when (result.error) {
        ApiError.PC_NOT_CONNECTED -> "${name}はhome-linkにつながっていません。PCでwindows-linkが動いているか確かめてください"
        ApiError.PC_REJECTED -> "${name}が${action.verb}を受け付けませんでした"
        ApiError.PC_NO_ANSWER -> "${name}が${action.verb}に答えませんでした"
        ApiError.PC_DISCONNECTED -> "${name}への接続が${action.verb}の途中で切れました"
        else -> failure(result.error, host).text
    }
}

/** A PC's state after reading the list: its own, unknown when it is not connected; a failed read keeps what was known. */
fun pcState(name: String, known: Boolean?, read: ApiResult<List<Pc>>): Boolean? = when (read) {
    is ApiResult.Ok -> read.value.firstOrNull { it.name == name }?.sleeping
    is ApiResult.Failed -> known
}

/** The name typed once `after` is picked: the picked PC's, unless the name typed is not `before`'s. */
fun pcNameAfterPick(current: String, before: String?, after: String): String =
    if (current.isBlank() || current == before) after else current
