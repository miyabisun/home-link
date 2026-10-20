package dev.miyabisun.homelink

import android.app.Activity
import android.content.res.ColorStateList
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.LayerDrawable
import android.graphics.drawable.RippleDrawable
import android.graphics.drawable.StateListDrawable
import android.text.Editable
import android.text.InputFilter
import android.text.InputType
import android.text.TextWatcher
import android.view.Gravity
import android.view.View
import android.view.WindowInsets
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView

// The screens' shared parts, built from Android's standard views with the product's tokens.

/** Sets a scrolling page that keeps clear of the system bars and keyboard, and returns its column. */
internal fun Activity.page(): LinearLayout {
    val scroll = ScrollView(this).apply {
        setBackgroundColor(getColor(R.color.background))
        isFillViewport = true
        setOnApplyWindowInsetsListener { view, insets ->
            val bars = insets.getInsets(WindowInsets.Type.systemBars() or
                WindowInsets.Type.displayCutout() or WindowInsets.Type.ime())
            view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
            insets
        }
    }
    val content = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(dp(16), dp(16), dp(16), dp(24))
    }
    scroll.addView(content)
    setContentView(scroll)
    return content
}

/** A result line: an icon and text on a surface that turns to the failure colors. */
internal class ResultRow(private val activity: Activity, parent: LinearLayout, top: Int) {
    private val panel = LinearLayout(activity).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        setPadding(activity.dp(12), activity.dp(8), activity.dp(12), activity.dp(8))
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        parent.addView(this, LinearLayout.LayoutParams(-1, -2).apply { topMargin = top })
    }
    private val icon = ImageView(activity).apply {
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        panel.addView(this, LinearLayout.LayoutParams(activity.dp(24), activity.dp(24)).apply { marginEnd = activity.dp(8) })
    }
    private val text = TextView(activity).apply {
        textSize = 16f
        setTextColor(activity.getColor(R.color.text))
        panel.addView(this, LinearLayout.LayoutParams(0, -2, 1f))
    }

    fun show(message: String?, failed: Boolean, progress: Int = 0) {
        panel.visibility = if (message == null) View.GONE else View.VISIBLE
        if (message == null) return
        text.text = message
        icon.setImageResource(when {
            progress != 0 -> progress
            failed -> R.drawable.ic_error
            else -> R.drawable.ic_success
        })
        panel.background = GradientDrawable().apply {
            setColor(activity.getColor(if (failed) R.color.danger_subtle else R.color.surface))
            cornerRadius = activity.dp(8).toFloat()
            setStroke(activity.dp(1), activity.getColor(if (failed) R.color.danger else R.color.border))
        }
    }
}

internal fun Activity.show(view: TextView, text: String?, failure: Boolean = true) {
    view.text = text.orEmpty()
    view.visibility = if (text == null) View.GONE else View.VISIBLE
    view.setTextColor(getColor(if (failure) R.color.danger else R.color.text))
}

internal fun changed(update: (String) -> Unit) = object : TextWatcher {
    override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
    override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}
    override fun afterTextChanged(s: Editable?) = update(s.toString())
}

internal fun Activity.panel(content: LinearLayout): LinearLayout = LinearLayout(this).apply {
    orientation = LinearLayout.VERTICAL
    background = surface(R.color.surface)
    setPadding(dp(12), dp(8), dp(12), dp(12))
    content.addView(this, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(12) })
}

internal fun Activity.surface(color: Int) = GradientDrawable().apply {
    setColor(getColor(color))
    cornerRadius = dp(8).toFloat()
    setStroke(dp(1), getColor(R.color.border))
}

internal fun Activity.label(parent: LinearLayout, value: String, size: Int, bold: Boolean = false,
                  muted: Boolean = false): TextView = TextView(this).apply {
    text = value
    textSize = size.toFloat()
    setTextColor(getColor(if (muted) R.color.muted else R.color.text))
    setPadding(0, dp(4), 0, dp(4))
    if (bold) typeface = Typeface.DEFAULT_BOLD
    parent.addView(this)
}

internal fun Activity.message(parent: LinearLayout): TextView = label(parent, "", 14).apply {
    accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
    visibility = View.GONE
}

internal fun Activity.field(parent: LinearLayout, description: String, action: Int, maxLength: Int = 100,
                            submit: () -> Unit): EditText =
    EditText(this).apply {
        textSize = 16f
        contentDescription = description
        inputType = InputType.TYPE_CLASS_TEXT
        imeOptions = action
        filters = arrayOf(InputFilter.LengthFilter(maxLength))
        minHeight = dp(48)
        setTextColor(getColor(R.color.text))
        setHintTextColor(getColor(R.color.muted))
        background = surface(R.color.background)
        setPadding(dp(12), dp(8), dp(12), dp(8))
        setOnEditorActionListener { _, id, _ -> (id == action).also { if (it) submit() } }
        parent.addView(this, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(4) })
    }

internal fun Activity.button(parent: LinearLayout, value: String, primary: Boolean = false, icon: Int = 0,
                   weight: Boolean = false, action: () -> Unit): Button = Button(this).apply {
    text = value
    textSize = 16f
    isAllCaps = false
    setSingleLine(false)
    minHeight = dp(48)
    gravity = Gravity.CENTER
    stateListAnimator = null
    val fill = if (!primary) surface(R.color.surface) else StateListDrawable().apply {
        addState(intArrayOf(-android.R.attr.state_enabled), surface(R.color.surface))
        addState(intArrayOf(), surface(R.color.accent).apply { setStroke(0, 0) })
    }
    background = RippleDrawable(ColorStateList.valueOf(getColor(R.color.border)), fill, null)
    setTextColor(ColorStateList(arrayOf(intArrayOf(-android.R.attr.state_enabled), intArrayOf()),
        intArrayOf(getColor(R.color.muted), getColor(if (primary) R.color.on_accent else R.color.text))))
    if (icon != 0) {
        setCompoundDrawablesRelativeWithIntrinsicBounds(icon, 0, 0, 0)
        compoundDrawablePadding = dp(8)
        // Icons draw in the body text colour, which the accent fill does not carry.
        if (primary) compoundDrawableTintList = textColors
    }
    setPadding(dp(12), dp(12), dp(12), dp(12))
    setOnClickListener { action() }
    val params = if (weight) LinearLayout.LayoutParams(0, -2, 1f).apply { marginEnd = dp(8) }
        else LinearLayout.LayoutParams(-1, -2)
    parent.addView(this, params.apply { topMargin = dp(8) })
}

internal fun Activity.dp(value: Int) = (value * resources.displayMetrics.density).toInt()

/** A select box: the input surface with an expand arrow, so it reads apart from buttons. */
internal fun Activity.select(parent: LinearLayout, items: ArrayAdapter<String>, description: String,
                             selected: (Int) -> Unit): Spinner = Spinner(this).apply {
    adapter = items
    contentDescription = description
    minimumHeight = dp(48)
    background = LayerDrawable(arrayOf(surface(R.color.background), getDrawable(R.drawable.ic_expand))).apply {
        setLayerGravity(1, Gravity.END or Gravity.CENTER_VERTICAL)
        setLayerInsetEnd(1, dp(12))
    }
    setPaddingRelative(0, 0, dp(40), 0)
    onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
        override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) = selected(position)
        override fun onNothingSelected(parent: AdapterView<*>?) {}
    }
    parent.addView(this, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(8) })
}

/** Puts [names] in the adapter only when they changed, so an open drop-down is not reset. */
internal fun fill(adapter: ArrayAdapter<String>, names: List<String>) {
    if (adapter.count != names.size || (0 until adapter.count).any { adapter.getItem(it) != names[it] }) {
        adapter.clear()
        adapter.addAll(names)
    }
}

/**
 * Draws a switch: on in the accent surface, off in the usual one, unanswered (`failed`) with the failure icon;
 * [icon] leads the words unless it is 0.
 */
internal fun Activity.paintSwitch(button: Button, text: String, on: Boolean, failed: Boolean = false, icon: Int = R.drawable.ic_power) {
    button.text = text
    button.background = RippleDrawable(ColorStateList.valueOf(getColor(R.color.border)),
        if (on) surface(R.color.accent).apply { setStroke(0, 0) } else surface(R.color.surface), null)
    val color = getColor(when {
        !button.isEnabled -> R.color.muted
        on -> R.color.on_accent
        failed -> R.color.danger
        else -> R.color.text
    })
    button.setTextColor(color)
    button.setCompoundDrawablesRelativeWithIntrinsicBounds(if (failed) R.drawable.ic_error else icon, 0, 0, 0)
    button.compoundDrawablePadding = dp(8)
    button.compoundDrawableTintList = if (failed) null else ColorStateList.valueOf(color)
}
