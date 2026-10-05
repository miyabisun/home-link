package dev.miyabisun.homelink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class RegisterFormTest {
    private val qr = "MT:Y.K9042C00KA0648G00"

    @Test fun registrationNeedsAScannedCodeAndARoom() {
        val form = RegisterForm()
        assertFalse(form.canRegister())
        form.rooms(ApiResult.Ok(listOf(Room(1, "寝室"))))
        assertEquals(1L, form.roomId)
        assertFalse(form.canRegister())
        form.scanned(qr)
        assertTrue(form.canRegister())
        form.busy = true
        assertFalse(form.canRegister())
    }

    @Test fun scanningANonMatterCodeKeepsThePreviousCode() {
        val form = RegisterForm()
        form.scanned(" $qr\n")
        assertEquals(qr, form.payload)
        form.scanned("https://example.com/")
        assertEquals(qr, form.payload)
        assertEquals(Status.NotMatter, form.status)
        form.scanned(qr)
        assertNull(form.status)
    }

    @Test fun reloadingRoomsKeepsTheSelectionWhileItExists() {
        val form = RegisterForm()
        form.rooms(ApiResult.Ok(listOf(Room(1, "寝室"), Room(2, "居間"))))
        form.roomId = 2
        form.rooms(ApiResult.Ok(listOf(Room(1, "寝室"), Room(2, "居間"), Room(3, "書斎"))))
        assertEquals(2L, form.roomId)
        form.rooms(ApiResult.Ok(listOf(Room(1, "寝室"))))
        assertEquals(1L, form.roomId)
        form.rooms(ApiResult.Ok(emptyList()))
        assertNull(form.roomId)
    }

    @Test fun failedRoomLoadsKeepTheKnownRooms() {
        val form = RegisterForm()
        form.rooms(ApiResult.Ok(listOf(Room(1, "寝室"))))
        form.rooms(ApiResult.Failed(ApiError.UNREACHABLE))
        assertEquals(listOf(Room(1, "寝室")), form.rooms)
        assertEquals(1L, form.roomId)
        assertEquals(Status.Failed(ApiError.UNREACHABLE), form.status)
    }

    @Test fun createdRoomsAreAddedAndSelected() {
        val form = RegisterForm()
        form.rooms(ApiResult.Ok(listOf(Room(1, "寝室"))))
        form.roomCreated(ApiResult.Ok(Room(5, "書斎")))
        assertEquals(listOf(Room(1, "寝室"), Room(5, "書斎")), form.rooms)
        assertEquals(5L, form.roomId)
        assertEquals(Status.RoomAdded("書斎"), form.status)
        form.roomCreated(ApiResult.Failed(ApiError.DUPLICATE_ROOM))
        assertEquals(5L, form.roomId)
        assertEquals(Status.Failed(ApiError.DUPLICATE_ROOM), form.status)
    }

    @Test fun successClearsTheCodeAndNameButKeepsTheRoom() {
        val form = RegisterForm()
        form.rooms(ApiResult.Ok(listOf(Room(1, "寝室"))))
        form.scanned(qr)
        form.name = " 天井灯 "
        form.busy = true
        form.registered(ApiResult.Ok(Unit))
        assertFalse(form.busy)
        assertEquals(Status.Registered("寝室", "天井灯"), form.status)
        assertNull(form.payload)
        assertEquals("", form.name)
        assertEquals(1L, form.roomId)
    }

    @Test fun failureKeepsEveryInput() {
        val form = RegisterForm()
        form.rooms(ApiResult.Ok(listOf(Room(1, "寝室"))))
        form.scanned(qr)
        form.name = "天井灯"
        form.busy = true
        form.registered(ApiResult.Failed(ApiError.DUPLICATE_QR))
        assertFalse(form.busy)
        assertEquals(Status.Failed(ApiError.DUPLICATE_QR), form.status)
        assertEquals(qr, form.payload)
        assertEquals("天井灯", form.name)
        assertEquals(1L, form.roomId)
    }
}
