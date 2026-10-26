package dev.miyabisun.homelink

/** The level and colour temperature an override label holds its lights at: CurrentLevel 1–254 and kelvin. */
data class Override(val level: Int, val kelvin: Int)

/** A label's daily switch at `minute` of the day, Asia/Tokyo. */
data class ScheduleEntry(val minute: Int, val on: Boolean)

/**
 * A label: [auto] follows the brightness schedule, [override] holds its lights while on (never both);
 * [on] is its last switch, null until it was switched.
 */
data class Label(val id: Long, val name: String, val auto: Boolean = false, val override: Override? = null,
                 val on: Boolean? = null, val schedule: List<ScheduleEntry> = emptyList())

/** What a widget sends to a label: the API path's last segment. */
enum class LabelOp(val path: String) { ON("on"), OFF("off"), TOGGLE("toggle") }

/** What switching a label did: [action] is on, off or release (an override ended); [on] the label's state after. */
data class LabelResult(val action: String, val on: Boolean?, val devices: SwitchResult)

/** The minute of the day home-link's `HH:MM` names, or null for anything else. */
fun minuteOf(clock: String): Int? {
    val (hour, minute) = Regex("""(\d{2}):(\d{2})""").matchEntire(clock)?.destructured ?: return null
    return (hour.toInt() * 60 + minute.toInt()).takeIf { hour.toInt() < 24 && minute.toInt() < 60 }
}

/** The widget's name once `after` is picked: the label's name, unless the name typed is not `before`'s. */
fun nameAfterPick(current: String, before: Label?, after: Label): String =
    if (current.isBlank() || current == before?.name) after.name else current

/** The name a widget shows: the one typed, else its label's. */
fun widgetName(typed: String, label: Label): String = typed.trim().ifEmpty { label.name }

/**
 * How the widget button sending `op` to the label named `name` reads: "<name>オン", "<name>オフ",
 * or the name for a toggle, as "<name>中" while the label is `on` and unknown while null. A press's short result shows instead.
 */
fun labelFace(op: LabelOp, name: String, on: Boolean?, press: LightsMessage?, sending: Boolean = false): ButtonFace = when (op) {
    LabelOp.ON -> buttonFace("${name}オン", R.drawable.ic_light_on, null, press, sending)
    LabelOp.OFF -> buttonFace("${name}オフ", R.drawable.ic_light_off, null, press, sending)
    LabelOp.TOGGLE -> buttonFace(name, R.drawable.ic_power, Toggle.of(on), press, sending)
}

/**
 * What a press shows on its button; none for a toggle that switched every device, as the button then shows the new state.
 * Devices that did not answer read apart from ones that failed: home-link took the press, the lights did not.
 */
fun pressLabel(op: LabelOp, result: ApiResult<LabelResult>): LightsMessage? {
    val counts = when (result) {
        is ApiResult.Failed -> return failureLabel(result.error)
        is ApiResult.Ok -> result.value.devices
    }
    val left = counts.noResponse + counts.failed + counts.missing.size
    return when {
        counts.switched + left == 0 -> LightsMessage("機器なし", failed = true)
        left == 0 -> when (op) {
            LabelOp.ON -> LightsMessage("オン完了", failed = false)
            LabelOp.OFF -> LightsMessage("オフ完了", failed = false)
            LabelOp.TOGGLE -> null
        }
        left == counts.noResponse -> LightsMessage(if (counts.switched > 0) "一部無応答" else "照明無応答", failed = true)
        counts.switched > 0 -> LightsMessage("一部失敗", failed = true)
        else -> LightsMessage("失敗", failed = true)
    }
}

/** What the app says after switching the label `name`: nothing when every device switched, else what was left behind. */
fun labelMessage(name: String, result: ApiResult<LabelResult>, host: String): LightsMessage? {
    val done = when (result) {
        is ApiResult.Failed -> return when (result.error) {
            ApiError.LABEL_NOT_FOUND -> LightsMessage("ラベル「$name」がありません", failed = true)
            ApiError.LABEL_HAS_NO_DEVICES -> LightsMessage("ラベル「$name」に機器がありません", failed = true)
            else -> failure(result.error, host)
        }
        is ApiResult.Ok -> result.value
    }
    val counts = done.devices
    val missing = counts.missing.size.takeIf { it > 0 }?.let { count -> "見つからない${count}台" + counts.missing.joinToString("、", "（", "）") }
    val left = listOfNotNull(
        counts.noResponse.takeIf { it > 0 }?.let { "応答なし${it}台" },
        counts.failed.takeIf { it > 0 }?.let { "失敗${it}台" },
        missing,
    ).joinToString("・")
    if (left.isEmpty()) return null
    val some = if (counts.switched > 0) "一部の" else ""
    if (counts.failed == 0 && counts.missing.isEmpty()) return LightsMessage("${name}の${some}照明が応答しませんでした。$left", failed = true)
    val undone = when (done.action) {
        "on" -> "オンにできませんでした"
        "off" -> "オフにできませんでした"
        else -> "元に戻せませんでした"
    }
    return LightsMessage("${name}の${some}機器を$undone。$left", failed = true)
}

/** Each label's state after a read: the answer's, where it has one; a failed read changes nothing known. */
fun readStates(known: Map<Long, Boolean>, read: ApiResult<List<Label>>): Map<Long, Boolean> = when (read) {
    is ApiResult.Ok -> read.value.mapNotNull { label -> label.on?.let { label.id to it } }.toMap()
    is ApiResult.Failed -> known
}

/** A state after reading it: the answer, else what was known. */
fun <T> readState(known: T?, read: ApiResult<T>): T? = if (read is ApiResult.Ok) read.value else known

/**
 * A state after a press: the answer's; unknown when the press was sent and nothing came back, as it may have
 * switched; else, never sent or refused, what was known.
 */
fun <T> pressedState(known: T?, result: ApiResult<T?>): T? = when (result) {
    is ApiResult.Ok -> result.value
    is ApiResult.Failed -> if (result.error == ApiError.TIMEOUT) null else known
}
