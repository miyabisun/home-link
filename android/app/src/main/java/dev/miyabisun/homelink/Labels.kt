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

/** One field of a label to change. */
sealed interface LabelUpdate {
    data class Name(val name: String) : LabelUpdate
    /** Auto and an override exclude each other; neither leaves the lights alone. */
    data class Dimming(val auto: Boolean, val override: Override?) : LabelUpdate
    /** Replaces the whole schedule. */
    data class Schedule(val entries: List<ScheduleEntry>) : LabelUpdate
}

/** What a widget or the app sends to a label: the API path's last segment. */
enum class LabelOp(val path: String) { ON("on"), OFF("off"), TOGGLE("toggle") }

/** What switching a label did: [action] is on, off or release (an override ended); [on] the label's state after. */
data class LabelResult(val action: String, val on: Boolean?, val devices: SwitchResult)

/** Matter's CurrentLevel for `percent` of full brightness, the way home-link counts 254 as 100%. */
fun levelOf(percent: Int): Int = Math.round(percent * 254 / 100.0).toInt().coerceIn(1, 254)

/** The percent `level` shows as; any lit level shows as at least 1%. */
fun percentOf(level: Int): Int = Math.round(level * 100 / 254.0).toInt().coerceIn(1, 100)

/** Which of a typed fixed brightness (%) and colour temperature (K) cannot be sent. */
enum class OverrideError { LEVEL, KELVIN }

fun overrideErrors(percent: String, kelvin: String): Set<OverrideError> = setOfNotNull(
    OverrideError.LEVEL.takeIf { percent.trim().toIntOrNull() !in 1..100 },
    OverrideError.KELVIN.takeIf { kelvin.trim().toIntOrNull() !in 1000..10000 },
)

/** The fixed values typed, or null while either cannot be sent. */
fun overrideOf(percent: String, kelvin: String): Override? =
    if (overrideErrors(percent, kelvin).isEmpty()) Override(levelOf(percent.trim().toInt()), kelvin.trim().toInt()) else null

/** `HH:MM` for a minute of the day, as home-link writes a schedule's time. */
fun clock(minute: Int): String = String.format(java.util.Locale.ROOT, "%02d:%02d", minute / 60, minute % 60)

/** The minute of the day home-link's `HH:MM` names, or null for anything else. */
fun minuteOf(clock: String): Int? {
    val (hour, minute) = Regex("""(\d{2}):(\d{2})""").matchEntire(clock)?.destructured ?: return null
    return (hour.toInt() * 60 + minute.toInt()).takeIf { hour.toInt() < 24 && minute.toInt() < 60 }
}

/**
 * The schedule with `entry` in place of the one at `index`, or added when none, in time order;
 * null when another switch is already at its time, which home-link refuses.
 */
fun scheduleWith(entries: List<ScheduleEntry>, entry: ScheduleEntry, index: Int? = null): List<ScheduleEntry>? {
    val others = entries.filterIndexed { i, _ -> i != index }
    if (others.any { it.minute == entry.minute }) return null
    return (others + entry).sortedBy { it.minute }
}

/** The widget's name once `after` is picked: the label's name, unless the name typed is not `before`'s. */
fun nameAfterPick(current: String, before: Label?, after: Label): String =
    if (current.isBlank() || current == before?.name) after.name else current

/** The name a widget shows: the one typed, else its label's. */
fun widgetName(typed: String, label: Label): String = typed.trim().ifEmpty { label.name }

/**
 * How the widget button sending `op` to the label named `name` reads: "<name>オン", "<name>オフ",
 * or the name, as "<name>中" while the label is `on`, for a toggle. A press's short result shows instead.
 */
fun labelFace(op: LabelOp, name: String, on: Boolean?, press: LightsMessage?, sending: Boolean = false): ButtonFace = when (op) {
    LabelOp.ON -> buttonFace("${name}オン", R.drawable.ic_light_on, null, press, sending)
    LabelOp.OFF -> buttonFace("${name}オフ", R.drawable.ic_light_off, null, press, sending)
    LabelOp.TOGGLE -> buttonFace(name, R.drawable.ic_power, on, press, sending)
}

/** What a press shows on its button; none for a toggle that switched every device, as the button then shows the new state. */
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
    val undone = when (done.action) {
        "on" -> "オンにできませんでした"
        "off" -> "オフにできませんでした"
        else -> "元に戻せませんでした"
    }
    val some = if (counts.switched > 0) "一部の" else ""
    return LightsMessage("${name}の${some}機器を$undone。$left", failed = true)
}
