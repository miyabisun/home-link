package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PcsTest {
    private val host = "home-link.example:5011"

    @Test fun aSleepingPcSaysSoOnTheAccentAndOneAwakeShowsItsName() {
        assertEquals(ButtonFace("スリープ中", R.drawable.ic_sleep, Tone.ACTIVE, "書斎（スリープ中）"), pcFace("書斎", true, null))
        assertEquals(ButtonFace("書斎", R.drawable.ic_power, Tone.PLAIN), pcFace("書斎", false, null))
        // Not listed or never read: unknown, never shown as awake.
        assertEquals(ButtonFace("書斎", R.drawable.ic_unknown, Tone.PLAIN, "書斎（状態不明）"), pcFace("書斎", null, null))
        assertEquals(ButtonFace("送信中…", R.drawable.ic_power, Tone.PLAIN), pcFace("書斎", true, SENDING, sending = true))
        assertEquals(ButtonFace("PC未接続", R.drawable.ic_error, Tone.FAILED),
            pcFace("書斎", null, failureLabel(ApiError.PC_NOT_CONNECTED)))
    }

    @Test fun aPressWakesASleepingPcAndPutsAnyOtherToSleep() {
        assertEquals(PcAction.WAKE, pcAction(true))
        assertEquals(PcAction.SLEEP, pcAction(false))
        assertEquals(PcAction.SLEEP, pcAction(null))
    }

    @Test fun aPressShowsNothingWhenThePcSaysItsNewStateElseWhatHappened() {
        assertNull(pcPress(ApiResult.Ok(true)))
        assertNull(pcPress(ApiResult.Ok(false)))
        // Taken by a PC that does not say its state.
        assertEquals(LightsMessage("受付済み", failed = false), pcPress(ApiResult.Ok(null)))
        assertEquals(
            listOf("PC未接続", "PC拒否", "PC無応答", "PC切断", "接続不可", "結果不明"),
            listOf(ApiError.PC_NOT_CONNECTED, ApiError.PC_REJECTED, ApiError.PC_NO_ANSWER, ApiError.PC_DISCONNECTED,
                ApiError.UNREACHABLE, ApiError.TIMEOUT).map { pcPress(ApiResult.Failed(it))?.text },
        )
    }

    @Test fun whatIsReadAloudNamesThePcAndWhatToDo() {
        assertNull(pcMessage("書斎", PcAction.SLEEP, ApiResult.Ok(true), host))
        assertEquals("書斎がスリープを受け付けました", pcMessage("書斎", PcAction.SLEEP, ApiResult.Ok(null), host))
        assertEquals("書斎が復帰を受け付けました", pcMessage("書斎", PcAction.WAKE, ApiResult.Ok(null), host))
        assertEquals(
            listOf(
                "書斎はhome-linkにつながっていません。PCでwindows-linkが動いているか確かめてください",
                "書斎がスリープを受け付けませんでした",
                "書斎がスリープに答えませんでした",
                "書斎への接続がスリープの途中で切れました",
                "home-link（home-link.example:5011）に接続できません。接続先の設定と通信を確かめてください",
            ),
            listOf(ApiError.PC_NOT_CONNECTED, ApiError.PC_REJECTED, ApiError.PC_NO_ANSWER, ApiError.PC_DISCONNECTED,
                ApiError.UNREACHABLE).map { pcMessage("書斎", PcAction.SLEEP, ApiResult.Failed(it), host) },
        )
    }

    @Test fun aReadTakesThePcsStateAndOneNotConnectedIsUnknown() {
        val read = ApiResult.Ok(listOf(Pc("書斎", true), Pc("ゲーム", null)))
        assertEquals(true, pcState("書斎", false, read))
        assertNull(pcState("ゲーム", false, read))
        assertNull(pcState("ノート", false, read))
        // A failed read keeps what was known.
        assertEquals(false, pcState("書斎", false, ApiResult.Failed(ApiError.UNREACHABLE)))
    }

    @Test fun theNameIsThePickedPcsUnlessAnotherIsTyped() {
        assertEquals("書斎", pcNameAfterPick("", null, "書斎"))
        assertEquals("ゲーム", pcNameAfterPick("書斎", "書斎", "ゲーム"))
        assertEquals("自分の名前", pcNameAfterPick("自分の名前", "書斎", "ゲーム"))
    }
}
