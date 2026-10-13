/** Small geometry shared by the live and eyepiece views' zoom and pan. */

export function clamp(value, min, max) {
    return Math.min(max, Math.max(min, value))
}

/** Scale that fits an image inside a container, never enlarging it, at `margin` of the fit. */
export function fitScale(container, width, height, margin = 1) {
    return Math.min(container.width / width, container.height / height, 1) * margin
}

/** Distance between the first two touches of a pinch. */
export function touchDistance(touches) {
    return Math.hypot(touches[0].clientX - touches[1].clientX, touches[0].clientY - touches[1].clientY)
}

/** The point a gesture is anchored at: one finger, or midway between two. */
export function touchCenter(touches) {
    if (touches.length === 1) return {x: touches[0].clientX, y: touches[0].clientY}
    return {
        x: (touches[0].clientX + touches[1].clientX) / 2,
        y: (touches[0].clientY + touches[1].clientY) / 2,
    }
}
