import {getCurrentInstance, onUnmounted, ref} from 'vue'

/**
 * How long the controls stay up after the last deliberate interaction.
 *
 * Long enough to read a readout, reach for a button and press it without the
 * chrome going out from under the hand — a second was only ever enough to prove
 * the fade worked. Exported so tests need not restate it.
 */
export const IDLE_HIDE_MS = 10_000

/**
 * How far a pointer may travel and still count as a tap. A pan that ends where it
 * started is a tap; one that travelled is not. Without this every drag-to-pan
 * would finish by toggling the controls the drag had nothing to do with.
 */
const TAP_SLOP_PX = 6

/** Anything a press lands on that is a control, not the image behind it. */
const CONTROL_SELECTOR = 'button, a, input, select, [data-overlay-control]'

/**
 * How long after a touch to disown the mouse events that follow it.
 *
 * A browser replays every touch as a compatibility `mousedown`/`mouseup` pair
 * unless the touch handler calls `preventDefault()` — which the live view cannot,
 * since its touch listeners are passive. Both views bind touch *and* mouse, so one
 * tap arrived as two presses and the toggle undid itself: the controls never moved
 * on a phone, the one device the whole fade exists for. 700ms covers the ~300ms
 * click delay a non-`touch-action`-managed page can still impose, with margin.
 */
const GHOST_MOUSE_MS = 700

/**
 * Visibility of the controls layered over a streamed image.
 *
 * The image is the point of these views, so the buttons and readouts fade out
 * `idleMs` after the last interaction. A tap on the image brings them back, and a
 * second tap puts them away again — the toggle decides from the visibility at
 * *press* time, not release, because the press itself may have shown them.
 *
 * Presses that land on a control, and gestures still in flight, only refresh the
 * countdown. Hiding the controls out from under the finger using them is the one
 * thing this must not do.
 */
export function useOverlayVisibility({idleMs = IDLE_HIDE_MS} = {}) {
    const visible = ref(true)
    let hideTimer = null
    let press = null
    let lastTouchAt = 0

    function clearHideTimer() {
        if (hideTimer === null) return
        clearTimeout(hideTimer)
        hideTimer = null
    }

    function show() {
        visible.value = true
        clearHideTimer()
        hideTimer = setTimeout(() => {
            hideTimer = null
            visible.value = false
        }, idleMs)
    }

    function hide() {
        clearHideTimer()
        visible.value = false
    }

    /**
     * Pin the controls up while something attached to them is open.
     *
     * A dropdown menu is teleported out to `body`, so it is not faded by the class
     * on the controls and receives none of their pointer events — left to the
     * countdown, its button would fade away underneath an open menu.
     */
    function setHold(held) {
        if (held) {
            clearHideTimer()
            visible.value = true
            return
        }
        show()
    }

    /**
     * Where a mouse or touch event happened. `changedTouches` first: it is the
     * finger this event is about, and it is the only one populated on `touchend`.
     */
    function pointOf(event) {
        const touch = event?.changedTouches?.[0] ?? event?.touches?.[0]
        if (touch) return {x: touch.clientX, y: touch.clientY}
        return {x: event?.clientX ?? 0, y: event?.clientY ?? 0}
    }

    /** Whether this event came from a finger rather than a mouse. */
    function isTouch(event) {
        return !!(event?.changedTouches || event?.touches)
    }

    /**
     * A mouse event the browser synthesised from the touch we just handled. Acting
     * on it would run the toggle a second time and land back where it started.
     */
    function isGhost(event) {
        return !isTouch(event) && Date.now() - lastTouchAt < GHOST_MOUSE_MS
    }

    function handlePressStart(event) {
        if (isGhost(event)) return
        if (isTouch(event)) lastTouchAt = Date.now()

        const {x, y} = pointOf(event)
        press = {
            x,
            y,
            onControl: !!event?.target?.closest?.(CONTROL_SELECTOR),
            multiTouch: (event?.touches?.length ?? 0) > 1,
            wasVisible: visible.value,
        }
        if (press.onControl) show()
    }

    function handlePressEnd(event) {
        if (isGhost(event)) return
        if (isTouch(event)) lastTouchAt = Date.now()

        // No press to end: a release whose press was disowned as a ghost, or one
        // that arrived after `cancelPress`. Either way it is not a tap.
        if (!press) return
        const {x, y} = pointOf(event)
        const {onControl, multiTouch, wasVisible} = press
        const travelled = Math.hypot(x - press.x, y - press.y)
        press = null

        if (onControl || multiTouch || travelled > TAP_SLOP_PX) {
            show()
            return
        }
        if (wasVisible) hide()
        else show()
    }

    /**
     * Abandon the press in flight without treating it as a tap. `touchcancel` is
     * the system taking the gesture away — a notification swipe, palm rejection —
     * not the observer asking for anything.
     */
    function cancelPress(event) {
        if (isTouch(event)) lastTouchAt = Date.now()
        press = null
    }

    // Armed straight away: opening the view and not touching it *is* inactivity, so
    // the controls introduce themselves and then get out of the way. Without this
    // the first load kept them on screen indefinitely.
    show()

    // Guarded so the composable stays callable from its own unit tests, which have
    // no component instance for a lifecycle hook to attach to.
    if (getCurrentInstance()) onUnmounted(clearHideTimer)

    return {visible, show, hide, setHold, cancelPress, handlePressStart, handlePressEnd}
}
