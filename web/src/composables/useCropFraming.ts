import {FIT_MARGIN} from '../constants'
import {fitScale} from '../utils/viewGeometry.js'
import type {StreamCrop, StreamFrameSize} from './api.events.js'
import type {View} from './usePanZoom.js'

/**
 * Keeps the observer's framing when the server's auto crop moves (`stack_crop_changed`).
 *
 * A crop change resizes the stream's frames, and the live view refits on every resize, which
 * would undo a zoom every few subs. A view the observer zoomed or panned instead keeps the
 * same sky point under the viewport's centre at the same sky scale; a fitted view refits.
 * The view is `transform: translate(position) scale(scale)` about the canvas centre.
 */

/** A width and height: a frame's, or a container's. */
interface Size {
    width: number
    height: number
}

/** A crop change: the crop the shown frames were cut with, and the one now in force. */
export interface CropChange {
    from: StreamCrop | null
    to: StreamCrop | null
}

/** A crop as a rectangle of the stack; `null` (shown whole) as the whole of `like`'s stack. */
function rectOf(crop: StreamCrop | null, like: StreamCrop) {
    if (crop) return crop
    return {x: 0, y: 0, width: like.source_width, height: like.source_height}
}

/**
 * The view that shows, in the new frame, what `view` showed in the old one; `null` when the
 * change cannot be followed (a frame size of zero, both crops unknown).
 */
export function reframeForCrop({from, to, oldDims, newDims, view}: CropChange & {
    oldDims: Size | null | undefined
    newDims: Size | null | undefined
    view: View
}): View | null {
    const like = to ?? from
    if (!like || !oldDims?.width || !oldDims?.height || !newDims?.width || !newDims?.height) return null
    const before = rectOf(from, like)
    const after = rectOf(to, like)
    // Stream pixels per stack pixel, before and after: a frame is its crop scaled into the box.
    const k0 = oldDims.width / before.width
    const k1 = newDims.width / after.width
    const scale = (view.scale * k0) / k1
    const axis = (pos: number, oldLen: number, newLen: number, origin0: number, origin1: number) => {
        const centre = oldLen / 2 - pos / view.scale
        const sky = origin0 + centre / k0
        return (newLen / 2 - (sky - origin1) * k1) * scale
    }
    return {
        scale,
        position: {
            x: axis(view.position.x, oldDims.width, newDims.width, before.x, after.x),
            y: axis(view.position.y, oldDims.height, newDims.height, before.y, after.y),
        },
    }
}

/** Whether `view` is the fit `usePanZoom.fitToView` gives `dims` in `container`. */
export function isFitted(view: View, container: Size | null | undefined, dims: Size | null | undefined) {
    if (!container?.width || !dims?.width || !dims?.height) return true
    const fit = fitScale(container, dims.width, dims.height, FIT_MARGIN)
    const still = view.position.x === 0 && view.position.y === 0
    return still && Math.abs(view.scale - fit) <= fit * 1e-6
}

/**
 * Holds a crop change until the frame that carries it is shown: `note(to, from, frame)` from
 * the event, `take(dims)` once per arriving frame.
 *
 * The event and the frames travel on different sockets, so a frame cut the old way can land
 * after the event; reframing that one moved the old picture, and the real cut then refit,
 * losing the zoom. The event names the size of the frame cut the new way (`frame`), and only
 * a frame of that size takes the change. A change that keeps the size (a pure move, or two
 * crops scaled into one stream box) cannot be told apart and is taken one frame early at
 * worst. An event without a size (none published yet) goes to the next frame.
 */
export function useCropFraming() {
    let pending: (CropChange & {frame: StreamFrameSize | null}) | null = null

    function note(to: StreamCrop | null, from: StreamCrop | null, frame: StreamFrameSize | null = null) {
        pending = {from: pending ? pending.from : from, to, frame}
    }

    function take(dims?: Size | null): CropChange | null {
        if (!pending) return null
        const {from, to, frame} = pending
        if (frame && (dims?.width !== frame.width || dims?.height !== frame.height)) return null
        pending = null
        return {from, to}
    }

    function clear() {
        pending = null
    }

    return {note, take, clear}
}
