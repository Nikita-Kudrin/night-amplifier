import {describe, it, expect, vi, beforeEach} from 'vitest'
import {ref} from 'vue'
import type {Ref} from 'vue'
import {useCometRoi} from './useCometRoi'
import {updateSettings} from './api'
import {settings as settingsFixture} from '../test/fixtures'
import type {Settings} from './api.types'
import type {ScreenRect} from './useCometRoi'

vi.mock('./api', async (importOriginal) =>
    (await import('../test/mockApi')).mockApi(await importOriginal()))

describe('useCometRoi', () => {
    let settings: Ref<Settings>
    let dimensions: Ref<{width: number; height: number}>
    let canvasBounds: Ref<ScreenRect>
    let containerRef: Ref<HTMLElement>

    beforeEach(() => {
        vi.clearAllMocks()
        settings = ref(settingsFixture({stacking_type: 'comet', comet_roi: {x: 100, y: 50, width: 200, height: 100}}))
        dimensions = ref({width: 1000, height: 500})
        // The canvas drawn at half size, 20 px in and 10 px down inside its container.
        canvasBounds = ref({left: 20, top: 10, width: 500, height: 250})
        const container = document.createElement('div')
        vi.spyOn(container, 'getBoundingClientRect').mockReturnValue(DOMRect.fromRect({width: 800, height: 600}))
        containerRef = ref(container)
    })

    function roi() {
        return useCometRoi(settings, dimensions, canvasBounds, containerRef)
    }

    it('places the stored ROI over the canvas, scaled to its display size', () => {
        const {roiDisplayRect, isCometMode} = roi()

        expect(isCometMode.value).toBe(true)
        expect(roiDisplayRect.value).toEqual({left: 70, top: 35, width: 100, height: 50})
    })

    it('follows the canvas when the view zooms or pans', () => {
        const {roiDisplayRect} = roi()
        expect(roiDisplayRect.value?.left).toBe(70)

        canvasBounds.value = {left: -480, top: -240, width: 2000, height: 1000}

        expect(roiDisplayRect.value).toEqual({left: -280, top: -140, width: 400, height: 200})
    })

    it('shows nothing without a stored ROI or a frame', () => {
        const {roiDisplayRect} = roi()

        dimensions.value = {width: 0, height: 0}
        expect(roiDisplayRect.value).toBeNull()

        dimensions.value = {width: 1000, height: 500}
        settings.value = {...settings.value, comet_roi: null}
        expect(roiDisplayRect.value).toBeNull()
    })

    function drag(handlers: ReturnType<typeof roi>, from: [number, number], to: [number, number]) {
        handlers.handleMouseDown({clientX: from[0], clientY: from[1]})
        handlers.handleMouseMove({clientX: to[0], clientY: to[1]})
        return handlers.handleMouseUp()
    }

    it('turns a drag over the canvas into image pixels, clamped to the canvas', async () => {
        const handlers = roi()
        handlers.startCometRoiSelection()

        await drag(handlers, [0, 60], [120, 110])

        expect(updateSettings).toHaveBeenCalledWith({comet_roi: {x: 0, y: 100, width: 200, height: 100}})
        expect(handlers.isSelectingCometRoi.value).toBe(false)
    })

    it('draws the selection rectangle while dragging', () => {
        const handlers = roi()
        handlers.startCometRoiSelection()

        handlers.handleMouseDown({clientX: 120, clientY: 110})
        handlers.handleMouseMove({clientX: 20, clientY: 60})

        expect(handlers.selectionRect.value).toEqual({left: 20, top: 60, width: 100, height: 50})
    })

    it('drops a selection too small to track a nucleus', async () => {
        const handlers = roi()
        handlers.startCometRoiSelection()

        await drag(handlers, [100, 100], [105, 140])

        expect(updateSettings).not.toHaveBeenCalled()
        expect(handlers.isSelectingCometRoi.value).toBe(false)
    })

    it('ignores the mouse unless a selection was started', async () => {
        const handlers = roi()

        await drag(handlers, [0, 0], [300, 300])

        expect(handlers.selectionRect.value).toBeNull()
        expect(updateSettings).not.toHaveBeenCalled()
    })
})
