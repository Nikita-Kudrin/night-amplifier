/** Small geometry shared by the live and eyepiece views' zoom and pan. */

interface Size {
    width: number
    height: number
}

interface Point {
    x: number
    y: number
}

/** What a gesture reads of each touch: `Touch`, or a test's stand-in. */
type TouchPoint = Pick<Touch, 'clientX' | 'clientY'>

/** The first touches of a gesture: `TouchList`, or an array of them. */
interface Touches {
    readonly length: number
    readonly [index: number]: TouchPoint
}

export function clamp(value: number, min: number, max: number): number {
    return Math.min(max, Math.max(min, value))
}

/** Scale that fits an image inside a container, never enlarging it, at `margin` of the fit. */
export function fitScale(container: Size, width: number, height: number, margin = 1): number {
    return Math.min(container.width / width, container.height / height, 1) * margin
}

/** Distance between the first two touches of a pinch. */
export function touchDistance(touches: Touches): number {
    return Math.hypot(touches[0].clientX - touches[1].clientX, touches[0].clientY - touches[1].clientY)
}

/** The point a gesture is anchored at: one finger, or midway between two. */
export function touchCenter(touches: Touches): Point {
    if (touches.length === 1) return {x: touches[0].clientX, y: touches[0].clientY}
    return {
        x: (touches[0].clientX + touches[1].clientX) / 2,
        y: (touches[0].clientY + touches[1].clientY) / 2,
    }
}

/** An element's layout box within its offset parent — where an overlay beside it goes. */
export function offsetBounds(
    element: Pick<HTMLElement, 'offsetLeft' | 'offsetTop' | 'offsetWidth' | 'offsetHeight'>
): {left: number; top: number; width: number; height: number} {
    return {
        left: element.offsetLeft,
        top: element.offsetTop,
        width: element.offsetWidth,
        height: element.offsetHeight,
    }
}
