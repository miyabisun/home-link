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
}
