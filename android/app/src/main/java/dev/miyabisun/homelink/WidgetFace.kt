package dev.miyabisun.homelink

import android.widget.RemoteViews

/**
 * Draws `face` on the widget button `cell` and its words `text`, the one button both widgets share.
 * Colours are resources, not values, so a widget drawn before the OS switches dark or light follows it.
 */
fun RemoteViews.showFace(cell: Int, text: Int, face: ButtonFace) {
    setInt(cell, "setBackgroundResource", if (face.tone == Tone.ACTIVE) R.drawable.widget_button_active else R.drawable.widget_button)
    setTextViewText(text, face.text)
    setColorStateList(text, "setTextColor", when (face.tone) {
        Tone.ACTIVE -> R.color.on_accent
        Tone.FAILED -> R.color.danger
        Tone.PLAIN -> R.color.text
    })
    setTextViewCompoundDrawablesRelative(text, 0, face.icon, 0, 0)
}
