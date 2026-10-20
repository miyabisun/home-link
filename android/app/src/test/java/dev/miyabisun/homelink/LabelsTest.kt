package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Test

class LabelsTest {
    private val all = SwitchResult(3, 0, 0, emptyList())
    private fun result(action: String, on: Boolean?, counts: SwitchResult = all) = ApiResult.Ok(LabelResult(action, on, counts))

    @Test fun percentsAndLevelsRoundTripAtTheValuesInUse() {
        assertEquals(254, levelOf(100))
        assertEquals(203, levelOf(80))
        assertEquals(3, levelOf(1))
        assertEquals(100, percentOf(254))
        assertEquals(80, percentOf(203))
        // The lowest level still reads as lit.
        assertEquals(1, percentOf(1))
    }

    @Test fun aFixedValueIsTakenOnlyWithinRange() {
        assertEquals(Override(203, 5000), overrideOf("80", "5000"))
        assertEquals(Override(254, 1000), overrideOf(" 100 ", "1000"))
        assertEquals(emptySet<OverrideError>(), overrideErrors("1", "10000"))
        assertEquals(setOf(OverrideError.LEVEL), overrideErrors("0", "5000"))
        assertEquals(setOf(OverrideError.LEVEL), overrideErrors("101", "5000"))
        assertEquals(setOf(OverrideError.KELVIN), overrideErrors("80", "999"))
        assertEquals(setOf(OverrideError.KELVIN), overrideErrors("80", "10001"))
        assertEquals(setOf(OverrideError.LEVEL, OverrideError.KELVIN), overrideErrors("", "5千"))
        assertEquals(null, overrideOf("80", ""))
    }

    @Test fun clocksReadAndWriteAsTheServerDoes() {
        assertEquals("00:00", clock(0))
        assertEquals("05:07", clock(307))
        assertEquals("23:59", clock(1439))
        assertEquals(0, minuteOf("00:00"))
        assertEquals(1439, minuteOf("23:59"))
        assertEquals(null, minuteOf("24:00"))
        assertEquals(null, minuteOf("5:00"))
    }

    @Test fun aScheduleStaysInTimeOrderAndRefusesASecondSwitchAtOneTime() {
        val razor = listOf(ScheduleEntry(0, true), ScheduleEntry(300, false))
        assertEquals(listOf(ScheduleEntry(0, true), ScheduleEntry(300, false), ScheduleEntry(1320, true)),
            scheduleWith(razor, ScheduleEntry(1320, true)))
        assertEquals(listOf(ScheduleEntry(60, false), ScheduleEntry(300, false), ScheduleEntry(1320, true)),
            scheduleWith(listOf(ScheduleEntry(1320, true), ScheduleEntry(300, false)), ScheduleEntry(60, false)))
        // Changing one entry, its own time included, is no clash.
        assertEquals(listOf(ScheduleEntry(0, false), ScheduleEntry(300, false)), scheduleWith(razor, ScheduleEntry(0, false), index = 0))
        assertEquals(listOf(ScheduleEntry(300, false), ScheduleEntry(360, true)), scheduleWith(razor, ScheduleEntry(360, true), index = 0))
        assertEquals(null, scheduleWith(razor, ScheduleEntry(300, true)))
        assertEquals(null, scheduleWith(razor, ScheduleEntry(300, true), index = 0))
    }

    @Test fun aWidgetIsNamedAfterItsLabelUntilAnotherNameIsTyped() {
        val room = Label(1, "room")
        val game = Label(2, "game")
        assertEquals("game", nameAfterPick("", null, game))
        assertEquals("game", nameAfterPick("room", room, game))
        assertEquals("ゲーム", nameAfterPick("ゲーム", room, game))
        assertEquals("ゲーム", widgetName(" ゲーム ", game))
        assertEquals("game", widgetName("  ", game))
    }

    @Test fun eachWidgetButtonReadsByWhatItSends() {
        assertEquals(ButtonFace("寝室オン", R.drawable.ic_light_on, Tone.PLAIN), labelFace(LabelOp.ON, "寝室", on = true, press = null))
        assertEquals(ButtonFace("寝室オフ", R.drawable.ic_light_off, Tone.PLAIN), labelFace(LabelOp.OFF, "寝室", on = true, press = null))
        assertEquals(ButtonFace("仕事", R.drawable.ic_power, Tone.PLAIN), labelFace(LabelOp.TOGGLE, "仕事", on = false, press = null))
        assertEquals(ButtonFace("仕事中", R.drawable.ic_boosted, Tone.ACTIVE), labelFace(LabelOp.TOGGLE, "仕事", on = true, press = null))
        // Unknown never reads as on.
        assertEquals(ButtonFace("仕事", R.drawable.ic_power, Tone.PLAIN), labelFace(LabelOp.TOGGLE, "仕事", on = null, press = null))
        assertEquals(ButtonFace("送信中…", R.drawable.ic_power, Tone.PLAIN),
            labelFace(LabelOp.TOGGLE, "仕事", on = true, press = LightsMessage("送信中…", failed = false), sending = true))
        assertEquals(ButtonFace("一部失敗", R.drawable.ic_error, Tone.FAILED),
            labelFace(LabelOp.ON, "寝室", on = null, press = LightsMessage("一部失敗", failed = true)))
    }

    @Test fun aWidgetPressIsShortAndAToggleShowsItsNewStateInstead() {
        assertEquals(LightsMessage("オン完了", failed = false), pressLabel(LabelOp.ON, result("on", true)))
        assertEquals(LightsMessage("オフ完了", failed = false), pressLabel(LabelOp.OFF, result("off", false)))
        assertEquals(null, pressLabel(LabelOp.TOGGLE, result("release", false)))
        assertEquals(LightsMessage("一部失敗", failed = true), pressLabel(LabelOp.TOGGLE, result("on", true, SwitchResult(2, 1, 0, emptyList()))))
        assertEquals(LightsMessage("失敗", failed = true), pressLabel(LabelOp.OFF, result("off", false, SwitchResult(0, 0, 0, listOf("台所")))))
        assertEquals(LightsMessage("ラベルなし", failed = true), pressLabel(LabelOp.ON, ApiResult.Failed(ApiError.LABEL_NOT_FOUND)))
        assertEquals(LightsMessage("機器なし", failed = true), pressLabel(LabelOp.TOGGLE, ApiResult.Failed(ApiError.LABEL_HAS_NO_DEVICES)))
        assertEquals(LightsMessage("接続不可", failed = true), pressLabel(LabelOp.ON, ApiResult.Failed(ApiError.UNREACHABLE)))
        assertEquals(LightsMessage("接続先未設定", failed = true), pressLabel(LabelOp.ON, ApiResult.Failed(ApiError.NOT_CONFIGURED)))
    }

    @Test fun theAppSaysNothingOnSuccessAndCountsWhatWasLeftBehind() {
        val host = "home-link.example:5011"
        assertEquals(null, labelMessage("room", result("on", true), host))
        assertEquals(LightsMessage("roomの一部の機器をオフにできませんでした。応答なし2台・失敗1台・見つからない2台（台所、通路）", failed = true),
            labelMessage("room", result("off", false, SwitchResult(3, 2, 1, listOf("台所", "通路"))), host))
        assertEquals(LightsMessage("gameの機器を元に戻せませんでした。応答なし3台", failed = true),
            labelMessage("game", result("release", true, SwitchResult(0, 3, 0, emptyList())), host))
        assertEquals(LightsMessage("ラベル「room」に機器がありません", failed = true),
            labelMessage("room", ApiResult.Failed(ApiError.LABEL_HAS_NO_DEVICES), host))
        assertEquals(LightsMessage("home-link（home-link.example:5011）に接続できません。接続先の設定と通信を確かめてください", failed = true),
            labelMessage("room", ApiResult.Failed(ApiError.UNREACHABLE), host))
    }
}
