package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Test

class LabelsTest {
    private val all = SwitchResult(3, 0, 0, emptyList())
    private fun result(action: String, on: Boolean?, counts: SwitchResult = all) = ApiResult.Ok(LabelResult(action, on, counts))

    @Test fun clocksReadAsTheServerWritesThem() {
        assertEquals(0, minuteOf("00:00"))
        assertEquals(1439, minuteOf("23:59"))
        assertEquals(null, minuteOf("24:00"))
        assertEquals(null, minuteOf("5:00"))
    }

    @Test fun aWidgetIsNamedAfterItsLabelUntilAnotherNameIsTyped() {
        val ambient = Label(1, "ambient")
        val game = Label(2, "game")
        assertEquals("game", nameAfterPick("", null, game))
        assertEquals("game", nameAfterPick("ambient", ambient, game))
        assertEquals("ゲーム", nameAfterPick("ゲーム", ambient, game))
        assertEquals("ゲーム", widgetName(" ゲーム ", game))
        assertEquals("game", widgetName("  ", game))
    }

    @Test fun eachWidgetButtonReadsByWhatItSends() {
        assertEquals(ButtonFace("寝室オン", R.drawable.ic_light_on, Tone.PLAIN), labelFace(LabelOp.ON, "寝室", on = true, press = null))
        assertEquals(ButtonFace("寝室オフ", R.drawable.ic_light_off, Tone.PLAIN), labelFace(LabelOp.OFF, "寝室", on = true, press = null))
        assertEquals(ButtonFace("仕事", R.drawable.ic_power, Tone.PLAIN), labelFace(LabelOp.TOGGLE, "仕事", on = false, press = null))
        assertEquals(ButtonFace("仕事中", R.drawable.ic_boosted, Tone.ACTIVE), labelFace(LabelOp.TOGGLE, "仕事", on = true, press = null))
        // Unknown reads as neither on nor off.
        assertEquals(ButtonFace("仕事", R.drawable.ic_unknown, Tone.PLAIN, "仕事（状態不明）"), labelFace(LabelOp.TOGGLE, "仕事", on = null, press = null))
        assertEquals(ButtonFace("送信中…", R.drawable.ic_power, Tone.PLAIN),
            labelFace(LabelOp.TOGGLE, "仕事", on = true, press = LightsMessage("送信中…", failed = false), sending = true))
        assertEquals(ButtonFace("一部失敗", R.drawable.ic_error, Tone.FAILED),
            labelFace(LabelOp.ON, "寝室", on = null, press = LightsMessage("一部失敗", failed = true)))
    }

    @Test fun aWidgetPressIsShortAndAToggleShowsItsNewStateInstead() {
        assertEquals(LightsMessage("オン完了", failed = false), pressLabel(LabelOp.ON, result("on", true)))
        assertEquals(LightsMessage("オフ完了", failed = false), pressLabel(LabelOp.OFF, result("off", false)))
        assertEquals(null, pressLabel(LabelOp.TOGGLE, result("release", false)))
        assertEquals(LightsMessage("一部失敗", failed = true), pressLabel(LabelOp.TOGGLE, result("on", true, SwitchResult(2, 1, 1, emptyList()))))
        // Lights that did not answer, apart from failures: all of them, or some.
        assertEquals(LightsMessage("照明無応答", failed = true), pressLabel(LabelOp.ON, result("on", true, SwitchResult(0, 3, 0, emptyList()))))
        assertEquals(LightsMessage("一部無応答", failed = true), pressLabel(LabelOp.TOGGLE, result("on", true, SwitchResult(2, 1, 0, emptyList()))))
        assertEquals(LightsMessage("回線なし", failed = true), pressLabel(LabelOp.ON, ApiResult.Failed(ApiError.NO_NETWORK)))
        assertEquals(LightsMessage("結果不明", failed = true), pressLabel(LabelOp.ON, ApiResult.Failed(ApiError.TIMEOUT)))
        assertEquals(LightsMessage("失敗", failed = true), pressLabel(LabelOp.OFF, result("off", false, SwitchResult(0, 0, 0, listOf("台所")))))
        assertEquals(LightsMessage("ラベルなし", failed = true), pressLabel(LabelOp.ON, ApiResult.Failed(ApiError.LABEL_NOT_FOUND)))
        assertEquals(LightsMessage("機器なし", failed = true), pressLabel(LabelOp.TOGGLE, ApiResult.Failed(ApiError.LABEL_HAS_NO_DEVICES)))
        assertEquals(LightsMessage("接続不可", failed = true), pressLabel(LabelOp.ON, ApiResult.Failed(ApiError.UNREACHABLE)))
        assertEquals(LightsMessage("接続先なし", failed = true), pressLabel(LabelOp.ON, ApiResult.Failed(ApiError.NOT_CONFIGURED)))
    }

    @Test fun theAppSaysNothingOnSuccessAndCountsWhatWasLeftBehind() {
        val host = "home-link.example:5011"
        assertEquals(null, labelMessage("ambient", result("on", true), host))
        assertEquals(LightsMessage("ambientの一部の機器をオフにできませんでした。応答なし2台・失敗1台・見つからない2台（台所、通路）", failed = true),
            labelMessage("ambient", result("off", false, SwitchResult(3, 2, 1, listOf("台所", "通路"))), host))
        assertEquals(LightsMessage("gameの照明が応答しませんでした。応答なし3台", failed = true),
            labelMessage("game", result("release", true, SwitchResult(0, 3, 0, emptyList())), host))
        assertEquals(LightsMessage("roomの一部の照明が応答しませんでした。応答なし1台", failed = true),
            labelMessage("room", result("on", true, SwitchResult(13, 1, 0, emptyList())), host))
        assertEquals(LightsMessage("gameの機器を元に戻せませんでした。応答なし2台・失敗1台", failed = true),
            labelMessage("game", result("release", true, SwitchResult(0, 2, 1, emptyList())), host))
        assertEquals(LightsMessage("電話に使える回線が無く、送れませんでした", failed = true),
            labelMessage("ambient", ApiResult.Failed(ApiError.NO_NETWORK), host))
        assertEquals(LightsMessage("ラベル「ambient」に機器がありません", failed = true),
            labelMessage("ambient", ApiResult.Failed(ApiError.LABEL_HAS_NO_DEVICES), host))
        assertEquals(LightsMessage("home-link（home-link.example:5011）に接続できません。接続先の設定と通信を確かめてください", failed = true),
            labelMessage("ambient", ApiResult.Failed(ApiError.UNREACHABLE), host))
    }

    @Test fun aFailedReadKeepsTheStatesKnownAndOnlyAnswersChangeThem() {
        val known = mapOf(1L to true, 2L to false)
        assertEquals(mapOf(1L to false, 3L to true),
            readStates(known, ApiResult.Ok(listOf(Label(1, "a", on = false), Label(2, "b", on = null), Label(3, "c", on = true)))))
        for (error in listOf(ApiError.UNREACHABLE, ApiError.TIMEOUT, ApiError.SERVER)) {
            assertEquals("$error", known, readStates(known, ApiResult.Failed(error)))
        }
    }

    @Test fun aPressThatMayHaveActedUnseenMakesTheStateUnknown() {
        assertEquals(false, pressedState(true, ApiResult.Ok(false)))
        assertEquals(null, pressedState(true, ApiResult.Ok(null)))
        // Sent with no answer: home-link may have switched.
        assertEquals(null, pressedState(true, ApiResult.Failed(ApiError.TIMEOUT)))
        // Never sent, or refused: unchanged.
        for (error in listOf(ApiError.UNREACHABLE, ApiError.CONNECT_TIMEOUT, ApiError.NO_NETWORK, ApiError.LABEL_NOT_FOUND)) {
            assertEquals("$error", true, pressedState(true, ApiResult.Failed(error)))
        }
        assertEquals(true, readState(true, ApiResult.Failed(ApiError.UNREACHABLE)))
        assertEquals(false, readState(true, ApiResult.Ok(false)))
    }
}
