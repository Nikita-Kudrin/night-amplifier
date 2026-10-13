/**
 * Where the Push-To chevron sits and how big it draws, from the push direction and the
 * image it is drawn over. Pure, so the geometry is tested without rendering.
 */

const MAX_DISTANCE = 30
const MIN_SCALE = 0.3
const MAX_SCALE = 1.6

const MIN_BASE_SIZE = 60
const MAX_BASE_SIZE = 240
const MAX_BASE_SIZE_OFF_SCREEN = 290
const REFERENCE_CONTAINER_SIZE = 600

const OFF_SCREEN_THRESHOLD = 0.4
const EDGE_MARGIN_PERCENT = 0.10

// Quadratic ease-in: near-centre targets get squashed more aggressively than
// a linear mapping would, while near-edge targets reach a larger ceiling.
const EDGE_PROXIMITY_CURVE = 2

/** The chevron SVG's height over its width (viewBox 100 x 160). */
export const ARROW_ASPECT = 1.6

// The drawing's tail sits ~0.3125 of the SVG height below its centre (viewBox y=130 of
// 160); the info box goes past that plus a small gap.
const TAIL_FRACTION = 0.3125
const INFO_TAIL_GAP_PX = 20

/** How many screens away the target is, or `null` while the FOV is unknown. */
export function screensAway(distanceDeg: number, fovDeg: number | null | undefined): number | null {
    return fovDeg != null && fovDeg > 0 ? distanceDeg / fovDeg : null
}

/** Whether the target lies far enough out that the arrow moves to the frame edge. */
export function isOffScreen(screens: number | null): boolean {
    return screens !== null && screens > OFF_SCREEN_THRESHOLD
}

/**
 * Growth factor: drives on-screen growth by how close the target is to the frame edge,
 * so chevrons visibly lengthen as it drifts outward; falls back to the distance alone
 * before a plate solve has given a FOV.
 */
export function arrowScale(distanceDeg: number, screens: number | null): number {
    const proximity = screens === null
        ? Math.min(distanceDeg / MAX_DISTANCE, 1.0)
        : Math.pow(Math.min(screens / OFF_SCREEN_THRESHOLD, 1.0), EDGE_PROXIMITY_CURVE)
    return MIN_SCALE + (MAX_SCALE - MIN_SCALE) * proximity
}

/** The arrow's width before scaling, following the image's smaller side. */
export function baseSize(imageWidth: number, imageHeight: number, offScreen: boolean): number {
    const sizeFactor = Math.min(imageWidth, imageHeight) / REFERENCE_CONTAINER_SIZE
    const maxSize = offScreen ? MAX_BASE_SIZE_OFF_SCREEN : MAX_BASE_SIZE
    return Math.max(MIN_BASE_SIZE, Math.min(maxSize, 100 * sizeFactor))
}

/**
 * How far from the image centre the arrow is pinned, toward the target and as far out as
 * fits inside a margin: against the rectangle, or the circle a circular eyepiece shows.
 */
export function edgeOffset({angleDeg, arrowSize, imageWidth, imageHeight, isCircular = false}: {
    /** Direction of the target, clockwise from up. */
    angleDeg: number
    /** Scaled arrow width in px. */
    arrowSize: number
    imageWidth: number
    imageHeight: number
    isCircular?: boolean
}): {x: number; y: number} {
    const angleRad = (angleDeg * Math.PI) / 180
    // After rotation by angleRad, the SVG's axis-aligned bounding box has
    // half-extents W|cos θ| + H|sin θ| (x) and W|sin θ| + H|cos θ| (y).
    const cosA = Math.abs(Math.cos(angleRad))
    const sinA = Math.abs(Math.sin(angleRad))
    const svgHalfW = arrowSize * 0.5
    const svgHalfH = arrowSize * ARROW_ASPECT * 0.5
    const halfWRot = svgHalfW * cosA + svgHalfH * sinA
    const halfHRot = svgHalfW * sinA + svgHalfH * cosA

    const dirX = Math.sin(angleRad)
    const dirY = -Math.cos(angleRad)
    if (Math.abs(dirX) < 0.001 && Math.abs(dirY) < 0.001) return {x: 0, y: 0}

    if (isCircular) {
        const radius = Math.min(imageWidth, imageHeight) / 2
        const maxOffset = Math.max(0, radius - svgHalfH - radius * EDGE_MARGIN_PERCENT)
        return {x: dirX * maxOffset, y: dirY * maxOffset}
    }

    // Clamp to 0 so the arrow pins to image centre on viewports where the
    // rotated arrow plus margin already spans the image half-size.
    const maxOffsetX = Math.max(0, imageWidth / 2 - halfWRot - imageWidth * EDGE_MARGIN_PERCENT)
    const maxOffsetY = Math.max(0, imageHeight / 2 - halfHRot - imageHeight * EDGE_MARGIN_PERCENT)
    const scaleX = Math.abs(dirX) > 0.001 ? maxOffsetX / Math.abs(dirX) : Infinity
    const scaleY = Math.abs(dirY) > 0.001 ? maxOffsetY / Math.abs(dirY) : Infinity
    const boundaryScale = Math.min(scaleX, scaleY)
    return {x: dirX * boundaryScale, y: dirY * boundaryScale}
}

/**
 * Where the distance readout goes relative to the arrow's centre: on the chevrons' tail
 * side, opposite the target, so it does not cover what the arrow points at.
 */
export function infoOffset(angleDeg: number, arrowSize: number): {x: number; y: number} {
    const angleRad = (angleDeg * Math.PI) / 180
    const tail = arrowSize * ARROW_ASPECT * TAIL_FRACTION + INFO_TAIL_GAP_PX
    return {x: -Math.sin(angleRad) * tail, y: Math.cos(angleRad) * tail}
}

/** "40% screen", "2.5× screen", "12× screen"; nothing when it is under a tenth of one. */
export function formatScreens(screens: number | null): string | null {
    if (screens === null || screens < 0.1) return null
    if (screens < 1) return `${(screens * 100).toFixed(0)}% screen`
    if (screens < 10) return `${screens.toFixed(1)}× screen`
    return `${Math.round(screens)}× screen`
}
