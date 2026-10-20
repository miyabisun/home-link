package dev.miyabisun.homelink

/** The 11-digit Matter manual pairing code printed on devices without a QR code. */
object ManualCode {
    const val LENGTH = 11

    private val D = arrayOf(
        intArrayOf(0, 1, 2, 3, 4, 5, 6, 7, 8, 9), intArrayOf(1, 2, 3, 4, 0, 6, 7, 8, 9, 5),
        intArrayOf(2, 3, 4, 0, 1, 7, 8, 9, 5, 6), intArrayOf(3, 4, 0, 1, 2, 8, 9, 5, 6, 7),
        intArrayOf(4, 0, 1, 2, 3, 9, 5, 6, 7, 8), intArrayOf(5, 9, 8, 7, 6, 0, 4, 3, 2, 1),
        intArrayOf(6, 5, 9, 8, 7, 1, 0, 4, 3, 2), intArrayOf(7, 6, 5, 9, 8, 2, 1, 0, 4, 3),
        intArrayOf(8, 7, 6, 5, 9, 3, 2, 1, 0, 4), intArrayOf(9, 8, 7, 6, 5, 4, 3, 2, 1, 0),
    )
    private val P = arrayOf(
        intArrayOf(0, 1, 2, 3, 4, 5, 6, 7, 8, 9), intArrayOf(1, 5, 7, 6, 2, 8, 3, 0, 9, 4),
        intArrayOf(5, 8, 0, 3, 7, 9, 6, 1, 4, 2), intArrayOf(8, 9, 1, 6, 0, 4, 3, 5, 2, 7),
        intArrayOf(9, 4, 5, 3, 1, 2, 6, 8, 7, 0), intArrayOf(4, 2, 8, 6, 5, 7, 3, 9, 0, 1),
        intArrayOf(2, 7, 9, 3, 8, 0, 6, 4, 1, 5), intArrayOf(7, 0, 4, 6, 9, 1, 3, 2, 5, 8),
    )

    /** The ASCII digits of `text`, up to the code's length. */
    fun digits(text: String): String = text.filter { it in '0'..'9' }.take(LENGTH)

    /** Groups digits 4-3-4 as printed on devices. */
    fun format(digits: String): String =
        listOf(0 until 4, 4 until 7, 7 until LENGTH)
            .mapNotNull { range -> digits.slice(range.first until minOf(range.last + 1, digits.length)).ifEmpty { null } }
            .joinToString(" ")

    /** The position in `formatted` just after its first `digits` digits. */
    fun cursor(formatted: String, digits: Int): Int {
        var seen = 0
        for ((i, c) in formatted.withIndex()) {
            if (seen == digits) return i
            if (c in '0'..'9') seen++
        }
        return formatted.length
    }

    /** Whether `digits` has the full length and a matching Verhoeff check digit. */
    fun isValid(digits: String): Boolean =
        digits.length == LENGTH && digits.reversed().foldIndexed(0) { i, c, digit -> D[c][P[i % 8][digit - '0']] } == 0

    /** The code as printed on the device's label: 11 digits grouped 4-3-4 with hyphens, or null for an undecodable QR payload. */
    fun label(payload: String): String? =
        (if (payload.startsWith("MT:")) fromQr(payload) else payload)?.let { format(it).replace(' ', '-') }

    /**
     * The 11-digit code for a scanned `MT:` payload, as the Matter specification derives it from the
     * discriminator's upper 4 bits and the passcode; null unless it is a version 0 payload for the standard flow.
     */
    fun fromQr(payload: String): String? {
        val bytes = base38(payload.removePrefix("MT:").takeIf { payload.startsWith("MT:") } ?: return null)
        if (bytes == null || bytes.size < 11) return null
        var at = 0
        fun bits(n: Int): Int {
            var value = 0
            for (i in 0 until n) value = value or ((bytes[(at + i) / 8].toInt() shr ((at + i) % 8) and 1) shl i)
            at += n
            return value
        }
        val version = bits(3)
        bits(16 + 16) // vendor and product ids
        val flow = bits(2)
        bits(8) // rendezvous
        val short = bits(12) shr 8
        val passcode = bits(27)
        if (version != 0 || flow != 0) return null
        val digits = "%d%05d%04d".format(short shr 2, (short and 3) shl 14 or (passcode and 0x3FFF), passcode shr 14)
        return digits + ('0'..'9').first { isValid(digits + it) }
    }

    private const val BASE38 = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-."

    /** Base38 chunks of 5, 4 or 2 characters carry 3, 2 or 1 little-endian bytes. */
    private fun base38(text: String): ByteArray? {
        val out = mutableListOf<Byte>()
        for (chunk in text.chunked(5)) {
            val size = when (chunk.length) { 5 -> 3; 4 -> 2; 2 -> 1; else -> return null }
            var value = 0L
            for (c in chunk.reversed()) {
                val digit = BASE38.indexOf(c)
                if (digit < 0) return null
                value = value * 38 + digit
            }
            if (value >= 1L shl (8 * size)) return null
            repeat(size) { out += (value shr (8 * it)).toByte() }
        }
        return out.toByteArray()
    }
}
