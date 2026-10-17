import { ref } from 'vue'
import { TOUCH_SLOP_PX } from '../constants'

// The touchend grace period, in ms: the tap a scroll ends on must not reach a control.
const RELEASE_MS = 50

const TOUCH_EVENTS = ['touchstart', 'touchmove', 'touchend', 'touchcancel'] as const

/** What the guard listens on: the window, or a test's element. */
type TouchTarget = Pick<Window, 'addEventListener' | 'removeEventListener'>

/** The event a control hands over, whose target is the control. */
export type ControlEvent = Pick<Event, 'target'>

/** A control's value to put back: a slider's number, a toggle's state. */
export type ControlValue = string | number | boolean

/**
 * Tells a page scroll from a control gesture on touch screens, so a finger scrolling the
 * sidebar does not drag a slider or flip a toggle on its way. One instance watches the
 * whole page (four capture-phase window listeners) for every control that asks.
 */
export function createScrollProtect(target: TouchTarget | null = window) {
  const isScrolling = ref(false)
  const isSliding = ref(false)
  let isTouching = false
  let startX = 0
  let startY = 0
  // A gesture starting inside the grace period is a new one, whose intent the old timer
  // must not reset halfway through deciding.
  let releaseTimer: ReturnType<typeof setTimeout> | null = null

  function clearRelease() {
    clearTimeout(releaseTimer ?? undefined)
    releaseTimer = null
  }

  function reset() {
    isScrolling.value = false
    isSliding.value = false
  }

  const listeners: Record<(typeof TOUCH_EVENTS)[number], (e: TouchEvent) => void> = {
    touchstart(e) {
      if (e.touches.length !== 1) return
      clearRelease()
      isTouching = true
      reset()
      startX = e.touches[0].clientX
      startY = e.touches[0].clientY
    },
    touchmove(e) {
      if (!isTouching || isScrolling.value || isSliding.value || e.touches.length !== 1) return
      const deltaX = Math.abs(e.touches[0].clientX - startX)
      const deltaY = Math.abs(e.touches[0].clientY - startY)
      if (deltaY > deltaX && deltaY > TOUCH_SLOP_PX) {
        isScrolling.value = true
      } else if (deltaX >= deltaY && deltaX > TOUCH_SLOP_PX) {
        isSliding.value = true
      }
    },
    touchend() {
      isTouching = false
      clearRelease()
      releaseTimer = setTimeout(() => {
        releaseTimer = null
        reset()
      }, RELEASE_MS)
    },
    touchcancel() {
      clearRelease()
      isTouching = false
      reset()
    },
  }
  const options = { passive: true, capture: true }
  for (const type of TOUCH_EVENTS) target?.addEventListener(type, listeners[type], options)

  function restoreValue(event: ControlEvent, originalValue: ControlValue | undefined) {
    const control = event.target
    if (originalValue === undefined || !(control instanceof HTMLInputElement)) return
    if (control.type === 'checkbox') {
      control.checked = Boolean(originalValue)
    } else {
      control.value = String(originalValue)
    }
  }

  /**
   * Whether `event` belongs to a scroll rather than to its control; if so, the control's
   * `originalValue` is put back. A slider is held still until the gesture shows it is a
   * slide, so a finger landing on one to scroll does not nudge it first.
   */
  function blockIfScrolling(event: ControlEvent, originalValue: ControlValue | undefined = undefined) {
    const control = event.target
    const undecidedSlider = isTouching && !isSliding.value && control instanceof HTMLInputElement && control.type === 'range'
    if (!isScrolling.value && !undecidedSlider) return false
    restoreValue(event, originalValue)
    return true
  }

  function dispose() {
    clearRelease()
    for (const type of TOUCH_EVENTS) target?.removeEventListener(type, listeners[type], options)
  }

  return { isScrolling, blockIfScrolling, dispose }
}

let instance: ReturnType<typeof createScrollProtect> | null = null

/** The page's one scroll guard; see `createScrollProtect`. */
export function useScrollProtect() {
  instance ??= createScrollProtect(typeof window === 'undefined' ? null : window)
  return instance
}

/** For tests: removes the page's listeners; the next `useScrollProtect()` starts afresh. */
export function __resetForTests() {
  instance?.dispose()
  instance = null
}
