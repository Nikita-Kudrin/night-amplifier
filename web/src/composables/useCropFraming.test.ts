import {describe, it, expect} from 'vitest'
import {isFitted, reframeForCrop, useCropFraming} from './useCropFraming.js'
import {FIT_MARGIN} from '../constants'
import type {View} from './usePanZoom.js'
import type {StreamCrop} from './api.events.js'

const SOURCE = {source_width: 400, source_height: 300}
const CROP: StreamCrop = {x: 40, y: 32, width: 320, height: 240, ...SOURCE}

/** The stack pixel under the viewport's centre, and stack pixels per screen pixel. */
function skyAtCentre(crop: StreamCrop | null, dims: {width: number; height: number}, view: View | null) {
    if (!view) throw new Error('not reframed')
    const rect = crop ?? {x: 0, y: 0, width: SOURCE.source_width, height: SOURCE.source_height}
    const k = dims.width / rect.width
    return {
        x: rect.x + (dims.width / 2 - view.position.x / view.scale) / k,
        y: rect.y + (dims.height / 2 - view.position.y / view.scale) / k,
        skyScale: view.scale * k,
    }
}

type Sky = ReturnType<typeof skyAtCentre>

function expectSameSky(a: Sky, b: Sky) {
    expect(b.x).toBeCloseTo(a.x, 9)
    expect(b.y).toBeCloseTo(a.y, 9)
    expect(b.skyScale).toBeCloseTo(a.skyScale, 9)
}

describe('reframeForCrop', () => {
    const view = {scale: 2, position: {x: 30, y: -10}}

    it('keeps the sky under the centre at the same scale when the stream is the crop', () => {
        const oldDims = {width: 400, height: 300}
        const newDims = {width: 320, height: 240}
        const reframed = reframeForCrop({from: null, to: CROP, oldDims, newDims, view})
        expectSameSky(skyAtCentre(null, oldDims, view), skyAtCentre(CROP, newDims, reframed))
    })

    it('keeps it when both crops are scaled into the stream box', () => {
        const oldDims = {width: 1440, height: 1080}
        const newDims = {width: 1440, height: 1080}
        const wider = {...CROP, x: 8, y: 8, width: 384, height: 288}
        const reframed = reframeForCrop({from: wider, to: CROP, oldDims, newDims, view})
        expectSameSky(skyAtCentre(wider, oldDims, view), skyAtCentre(CROP, newDims, reframed))
        expect(reframed?.scale).toBeCloseTo(view.scale * (1440 / 384) / (1440 / 320), 12)
    })

    it('follows a crop being dropped as well', () => {
        const oldDims = {width: 320, height: 240}
        const newDims = {width: 400, height: 300}
        const reframed = reframeForCrop({from: CROP, to: null, oldDims, newDims, view})
        expectSameSky(skyAtCentre(CROP, oldDims, view), skyAtCentre(null, newDims, reframed))
    })

    it('gives up on a frame with no size', () => {
        expect(reframeForCrop({from: null, to: CROP, oldDims: {width: 0, height: 0}, newDims: {width: 320, height: 240}, view})).toBeNull()
    })
})

describe('isFitted', () => {
    const container = {width: 800, height: 600}
    const dims = {width: 400, height: 300}
    const fit = Math.min(800 / 400, 600 / 300, 1) * FIT_MARGIN

    it('recognises the fit the view gives itself', () => {
        expect(isFitted({scale: fit, position: {x: 0, y: 0}}, container, dims)).toBe(true)
    })

    it('tells a zoom or a pan apart from it', () => {
        expect(isFitted({scale: fit * 1.2, position: {x: 0, y: 0}}, container, dims)).toBe(false)
        expect(isFitted({scale: fit, position: {x: 5, y: 0}}, container, dims)).toBe(false)
    })
})

describe('useCropFraming', () => {
    it('hands a change over once, from where the shown frames were', () => {
        const framing = useCropFraming()
        framing.note(CROP, null)
        framing.note({...CROP, x: 48}, CROP)
        expect(framing.take()).toEqual({from: null, to: {...CROP, x: 48}})
        expect(framing.take()).toBeNull()
    })

    it('waits for the frame size the change names', () => {
        const framing = useCropFraming()
        framing.note(CROP, null, {width: 320, height: 240})
        expect(framing.take({width: 400, height: 300})).toBeNull()
        expect(framing.take({width: 320, height: 240})).toEqual({from: null, to: CROP})
        expect(framing.take({width: 320, height: 240})).toBeNull()
    })

    it('waits for the latest change, from the first frames still shown', () => {
        const framing = useCropFraming()
        const next = {...CROP, x: 48, width: 312}
        framing.note(CROP, null, {width: 320, height: 240})
        framing.note(next, CROP, {width: 312, height: 240})
        expect(framing.take({width: 320, height: 240})).toBeNull()
        expect(framing.take({width: 312, height: 240})).toEqual({from: null, to: next})
    })

    it('forgets a change once cleared', () => {
        const framing = useCropFraming()
        framing.note(CROP, null, {width: 320, height: 240})
        framing.clear()
        expect(framing.take({width: 320, height: 240})).toBeNull()
    })
})
