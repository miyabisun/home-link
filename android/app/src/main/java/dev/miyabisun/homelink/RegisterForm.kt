package dev.miyabisun.homelink

sealed interface Status {
    data object NotMatter : Status
    data object ScanFailed : Status
    data class RoomAdded(val room: String) : Status
    data class Registered(val room: String, val name: String) : Status
    data class Failed(val error: ApiError) : Status
}

enum class ManualError { LENGTH, CHECK_DIGIT }

class RegisterForm {
    /** A scanned `MT:` payload or the 11 digits of a manual pairing code. */
    var payload: String? = null
    var manualError: ManualError? = null
    var roomId: Long? = null
    var rooms: List<Room> = emptyList()
    var name = ""
    var busy = false
    var roomsFailed = false
    var status: Status? = null

    fun canRegister() = payload != null && roomId != null && !busy

    /** Accepts a scanned code; anything but a Matter payload leaves the previous code. */
    fun scanned(value: String) {
        val code = value.trim()
        if (code.startsWith("MT:")) {
            payload = code
            status = null
        } else {
            status = Status.NotMatter
        }
    }

    /** Takes the code once all digits are typed; a wrong check digit shows at once. */
    fun typed(text: String): Boolean {
        val digits = ManualCode.digits(text)
        if (digits.length < ManualCode.LENGTH) {
            manualError = null
            return false
        }
        return accept(digits)
    }

    /** Like [typed], but also reports a code that is still too short. */
    fun submitted(text: String): Boolean {
        val digits = ManualCode.digits(text)
        if (digits.length < ManualCode.LENGTH) {
            manualError = ManualError.LENGTH
            return false
        }
        return accept(digits)
    }

    private fun accept(digits: String): Boolean {
        val valid = ManualCode.isValid(digits)
        manualError = if (valid) null else ManualError.CHECK_DIGIT
        if (valid) {
            payload = digits
            status = null
        }
        return valid
    }

    fun rooms(result: ApiResult<List<Room>>) {
        when (result) {
            is ApiResult.Ok -> {
                rooms = result.value
                if (rooms.none { it.id == roomId }) roomId = rooms.firstOrNull()?.id
                // A successful load proves the server is reachable again.
                if (status == Status.Failed(ApiError.UNREACHABLE)) status = null
                roomsFailed = false
            }
            is ApiResult.Failed -> {
                roomsFailed = true
                status = Status.Failed(result.error)
            }
        }
    }

    fun roomCreated(result: ApiResult<Room>) {
        when (result) {
            is ApiResult.Ok -> {
                rooms = rooms + result.value
                roomId = result.value.id
                status = Status.RoomAdded(result.value.name)
            }
            is ApiResult.Failed -> status = Status.Failed(result.error)
        }
    }

    fun registered(result: ApiResult<Unit>) {
        busy = false
        status = when (result) {
            is ApiResult.Ok -> Status.Registered(rooms.firstOrNull { it.id == roomId }?.name.orEmpty(), name.trim()).also {
                payload = null
                name = ""
            }
            is ApiResult.Failed -> Status.Failed(result.error)
        }
    }
}
