package dev.miyabisun.homelink

import java.util.Locale
import java.util.UUID

/** Bluetooth UUIDs in the forms the BLE proxy protocol allows: 16-bit short, canonical or compact. */
object Uuids {
    private const val BASE = "-0000-1000-8000-00805f9b34fb"
    private val hex = Regex("[0-9a-fA-F]+")

    fun parse(text: String): UUID? {
        val compact = text.replace("-", "")
        if (!hex.matches(compact)) return null
        val full = when (compact.length) {
            4 -> "0000$compact$BASE"
            8 -> "$compact$BASE"
            32 -> compact.substring(0, 8) + "-" + compact.substring(8, 12) + "-" + compact.substring(12, 16) +
                "-" + compact.substring(16, 20) + "-" + compact.substring(20)
            else -> return null
        }
        return UUID.fromString(full)
    }

    /** The 16-bit form for UUIDs on the Bluetooth base, else the canonical upper-case form. */
    fun show(uuid: UUID): String {
        val text = uuid.toString()
        return if (text.startsWith("0000") && text.endsWith(BASE)) text.substring(4, 8).uppercase(Locale.ROOT)
            else text.uppercase(Locale.ROOT)
    }
}
