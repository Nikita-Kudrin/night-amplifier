import {assert, describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {nextTick, ref} from 'vue'
import type {Ref} from 'vue'
import {setupMocks, mountLiveView, createMockFrameData} from './setup.js'
import type {Mocks} from './setup.js'
import type {MockProvides} from './setup.js'
import {reframeForCrop} from '../../../composables/useCropFraming.js'
import type {StackCrop} from '../../../composables/events/captureEvents.js'
import type {StreamCrop, StreamFrameSize} from '../../../composables/api.events.js'
import type {CameraRole} from '../../../composables/api.types.js'

/**
 * The server's auto crop resizes the stream's frames every few subs. A view the observer
 * zoomed keeps its framing across that; a fitted one refits, as on any other resize.
 */
describe('LiveView - auto crop', () => {
    const SOURCE = {source_width: 400, source_height: 300}
    const CROP: StreamCrop = {x: 40, y: 32, width: 320, height: 240, ...SOURCE}
    let mocks: Mocks
    let stackCrop: Ref<StackCrop>

    beforeEach(() => {
        vi.useFakeTimers()
        mocks = setupMocks()
        stackCrop = ref<StackCrop>({crop: null, frame: null})
        mocks.mockImageStream.frameData.value = createMockFrameData(4, 3)
        mocks.mockImageStream.dimensions.value = {width: 400, height: 300}
    })

    afterEach(() => {
        vi.useRealTimers()
        vi.restoreAllMocks()
    })

    async function mount(provides: MockProvides = {}, eventStream: MockProvides['eventStream'] = {}) {
        const wrapper = mountLiveView({eventStream: {stackCrop, ...eventStream}, ...provides})
        const element: Element = wrapper.element
        element.getBoundingClientRect = () => DOMRect.fromRect({x: 0, y: 0, width: 800, height: 600})
        await nextTick()
        vi.advanceTimersByTime(50)
        mocks.mockPanZoom.fitToView.mockClear()
        return wrapper
    }

    /** The server cuts its frames to `crop`, then the first such frame arrives. */
    async function cropTo(crop: StreamCrop, dims: StreamFrameSize) {
        stackCrop.value = {crop, frame: dims}
        await nextTick()
        mocks.mockImageStream.frameData.value = createMockFrameData(4, 3, 101)
        mocks.mockImageStream.dimensions.value = dims
        await nextTick()
        vi.advanceTimersByTime(50)
    }

    it('keeps a zoomed view on the same sky', async () => {
        await mount()
        const view = {scale: 2, position: {x: 30, y: -10}}
        mocks.mockPanZoom.scale.value = view.scale
        mocks.mockPanZoom.position.value = view.position

        await cropTo(CROP, {width: 320, height: 240})

        const expected = reframeForCrop({
            from: null, to: CROP, oldDims: {width: 400, height: 300}, newDims: {width: 320, height: 240}, view,
        })
        assert(expected)
        expect(mocks.mockPanZoom.fitToView).not.toHaveBeenCalled()
        expect(mocks.mockPanZoom.scale.value).toBeCloseTo(expected.scale, 9)
        expect(mocks.mockPanZoom.position.value.x).toBeCloseTo(expected.position.x, 9)
        expect(mocks.mockPanZoom.position.value.y).toBeCloseTo(expected.position.y, 9)
    })

    // The event and the frame travel on different sockets: on a slow link the frame before
    // the cut (still at the old size) can land after the event. Pairing the change with it
    // shifted the old picture, and the real cut frame then refit, losing the zoom.
    it('keeps a zoomed view when an older frame lands between the event and the cut frame', async () => {
        await mount()
        const view = {scale: 2, position: {x: 30, y: -10}}
        mocks.mockPanZoom.scale.value = view.scale
        mocks.mockPanZoom.position.value = view.position

        stackCrop.value = {crop: CROP, frame: {width: 320, height: 240}}
        await nextTick()
        mocks.mockImageStream.frameData.value = createMockFrameData(4, 3, 100)
        await nextTick()
        expect(mocks.mockPanZoom.scale.value).toBe(2)
        expect(mocks.mockPanZoom.position.value).toEqual(view.position)

        mocks.mockImageStream.frameData.value = createMockFrameData(4, 3, 101)
        mocks.mockImageStream.dimensions.value = {width: 320, height: 240}
        await nextTick()
        vi.advanceTimersByTime(50)

        const expected = reframeForCrop({
            from: null, to: CROP, oldDims: {width: 400, height: 300}, newDims: {width: 320, height: 240}, view,
        })
        assert(expected)
        expect(mocks.mockPanZoom.fitToView).not.toHaveBeenCalled()
        expect(mocks.mockPanZoom.scale.value).toBeCloseTo(expected.scale, 9)
        expect(mocks.mockPanZoom.position.value.x).toBeCloseTo(expected.position.x, 9)
    })

    // A change noted for the main stream says nothing about the guide's frames, nor about
    // main frames after switching back: it is dropped with the switch.
    it('drops a pending change when the view switches camera', async () => {
        const viewedCamera = ref<CameraRole>('main')
        await mount({hasGuideCamera: true}, {viewedCamera})
        mocks.mockPanZoom.scale.value = 2

        stackCrop.value = {crop: CROP, frame: {width: 320, height: 240}}
        await nextTick()
        viewedCamera.value = 'guide'
        await nextTick()
        mocks.mockImageStream.frameData.value = createMockFrameData(4, 3, 101)
        mocks.mockImageStream.dimensions.value = {width: 320, height: 240}
        await nextTick()
        vi.advanceTimersByTime(50)

        expect(mocks.mockPanZoom.fitToView).toHaveBeenCalled()
        expect(mocks.mockPanZoom.scale.value).toBe(2)
    })

    it('refits a fitted view', async () => {
        await mount()
        mocks.mockPanZoom.scale.value = 0.95 * Math.min(800 / 400, 600 / 300, 1)

        await cropTo(CROP, {width: 320, height: 240})

        expect(mocks.mockPanZoom.fitToView).toHaveBeenCalledTimes(1)
    })

    // Without the setting the server never announces a crop: a resize refits, as it always did.
    it('refits on a resize no crop explains', async () => {
        await mount()
        mocks.mockPanZoom.scale.value = 2

        mocks.mockImageStream.frameData.value = createMockFrameData(4, 3, 101)
        mocks.mockImageStream.dimensions.value = {width: 320, height: 240}
        await nextTick()
        vi.advanceTimersByTime(50)

        expect(mocks.mockPanZoom.fitToView).toHaveBeenCalledTimes(1)
        expect(mocks.mockPanZoom.scale.value).toBe(2)
    })

    // The guide stream is never cropped, so the main stream's cut says nothing about it.
    it('ignores the crop while the guide camera is shown', async () => {
        await mount({hasGuideCamera: true, viewedCamera: 'guide'})
        mocks.mockPanZoom.scale.value = 2

        await cropTo(CROP, {width: 320, height: 240})

        expect(mocks.mockPanZoom.fitToView).toHaveBeenCalledTimes(1)
        expect(mocks.mockPanZoom.scale.value).toBe(2)
    })
})
