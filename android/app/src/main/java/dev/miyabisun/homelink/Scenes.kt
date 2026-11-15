package dev.miyabisun.homelink

/** One routine as home-link lists it, such as "帰宅": its name and its steps, a line each. */
data class Scene(val name: String, val steps: List<String>)

/** Where the phone is: degrees, the radius in metres it vouches for, and how long ago it measured. */
data class Fix(val latitude: Double, val longitude: Double, val accuracy: Double, val ageMs: Long)

/** What home-link answered a run of a routine. */
sealed interface SceneRun {
    /** Every step ran; `failed` of them failed, the first of which is written `failure`. */
    data class Done(val failed: Int, val failure: String?) : SceneRun

    /**
     * Stopped at step `next` until this phone is near home, for `waitMinutes` at most; `reason` is home-link's word
     * for why it is not there yet, and `distance` the metres left when it knows them.
     */
    data class Waiting(val next: Int, val waitMinutes: Int, val reason: String, val distance: Double?) : SceneRun
}

/** Whether a fix may still end the wait: not while this phone or the home's place is not registered. */
val SceneRun.Waiting.watchable: Boolean get() = reason in setOf("no_location", "stale", "vague", "far")

/** How a watch for the phone to come near home ended without the routine going on. */
enum class WatchEnd(val label: LightsMessage, val message: String) {
    EXPIRED(LightsMessage("未到着", failed = true), "自宅の近くに着かなかったため、%sの続きは実行していません"),
    CANCELLED(LightsMessage("取り消し", failed = false), "%sの待機を取り消しました"),
    NO_PERMISSION(LightsMessage("位置未許可", failed = true), "位置情報が許可されていません。アプリの「この電話の位置」で登録してください"),
    LOCATION_OFF(LightsMessage("位置オフ", failed = true), "電話の位置情報がオフです。オンにしてからもう一度押してください"),
}

// Short labels fit five full-width characters, the narrowest button's width.
private val DONE = LightsMessage("完了", failed = false)
private val PARTLY = LightsMessage("一部失敗", failed = true)

/** The short label a routine's button shows for a run's answer. */
fun scenePress(result: ApiResult<SceneRun>): LightsMessage = when (result) {
    is ApiResult.Ok -> when (val run = result.value) {
        is SceneRun.Done -> if (run.failed == 0) DONE else PARTLY
        is SceneRun.Waiting -> when (run.reason) {
            "phone_not_registered", "no_phone" -> LightsMessage("電話未登録", failed = true)
            "home_not_set" -> LightsMessage("自宅未登録", failed = true)
            else -> WatchEnd.EXPIRED.label
        }
    }
    is ApiResult.Failed -> if (result.error == ApiError.SCENE_NOT_FOUND) LightsMessage("操作なし", failed = true) else failureLabel(result.error)
}

/** What is read aloud for a run's answer: the routine and, when it did not all go, what to do. */
fun sceneMessage(name: String, result: ApiResult<SceneRun>, host: String): String = when (result) {
    is ApiResult.Ok -> when (val run = result.value) {
        is SceneRun.Done ->
            if (run.failed == 0) "${name}を実行しました"
            else "${name}を実行しましたが、${run.failed}つのステップが失敗しました。最初は「${run.failure}」です"
        is SceneRun.Waiting -> when (run.reason) {
            "phone_not_registered", "no_phone" -> "この電話がhome-linkに登録されていません。アプリの「この電話の位置」で登録してください"
            "home_not_set" -> "自宅の位置が登録されていません。自宅でアプリの「この電話の位置」から登録してください"
            else -> WatchEnd.EXPIRED.message.format(name)
        }
    }
    is ApiResult.Failed ->
        if (result.error == ApiError.SCENE_NOT_FOUND) "home-linkに「$name」という操作がありません。ウィジェットを設定し直してください"
        else failure(result.error, host).text
}

/**
 * How a routine's button reads: its name; a press's short `press` label; or, while it waits for the phone to come
 * near home, that on the accent.
 */
fun sceneFace(name: String, press: LightsMessage?, sending: Boolean = false, waiting: Boolean = false): ButtonFace =
    if (waiting) ButtonFace("待機中", R.drawable.ic_near, Tone.ACTIVE, "$name：自宅の近くに着くのを待っています。押すと取り消します")
    else buttonFace(name, R.drawable.ic_power, null, press, sending)

/** How long ago a fix was measured, in ms, by the clock that stamped it (ns since boot). */
fun fixAgeMs(nowNanos: Long, measuredNanos: Long): Long = ((nowNanos - measuredNanos) / 1_000_000).coerceAtLeast(0)

/** The widest radius, in metres, a fix may vouch for to pin the home by. */
const val PIN_ACCURACY = 50.0

/** Why `fix` is too unsure to pin the home by; none when it is sure enough. */
fun pinRefusal(fix: Fix): String? =
    if (Math.round(fix.accuracy) <= PIN_ACCURACY) null
    else "位置の精度が${Math.round(fix.accuracy)}mです。屋外か窓の近くでやり直してください"

/**
 * How long to hold the next fix back from home-link, in ms, `distance` metres from home: none within a kilometre,
 * and farther away the time a car at 30 m/s takes to come that near, as nothing changes sooner.
 */
fun resendAfterMs(distance: Double?): Long =
    if (distance == null || distance <= 1_000) 0 else ((distance - 1_000) / 30 * 1_000).toLong()
