import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {nextTick} from 'vue'
import {setupMocks, mountLiveView, createMockFrameData, withRealPanZoom} from './setup'
import type {Mocks} from './setup'

/** Two fingers `distance` apart, level about (195, 150). */
function fingers(distance: number) {
    return {
        touches: [
            {clientX: 195 - distance / 2, clientY: 150},
            {clientX: 195 + distance / 2, clientY: 150},
        ],
    }
}

describe('LiveView - Touch Interactions', () => {
    let mocks: Mocks

    beforeEach(() => {
        mocks = setupMocks()
    })

    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('handles single touch for panning', async () => {
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
        const wrapper = mountLiveView()

        await nextTick()

        const liveView = wrapper.find('.live-view')

        // Start touch
        await liveView.trigger('touchstart', {
            touches: [{clientX: 100, clientY: 100}],
        })

        // Move touch
        await liveView.trigger('touchmove', {
            touches: [{clientX: 150, clientY: 150}],
        })

        // The canvas should have a transform applied
        const canvas = wrapper.find<HTMLCanvasElement>('.live-canvas')
        expect(canvas.element.style.transform).toContain('translate')
    })

    it('handles two-finger touch for pinch zoom', async () => {
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
        const wrapper = mountLiveView()

        await nextTick()

        const liveView = wrapper.find('.live-view')

        // Start pinch
        await liveView.trigger('touchstart', {
            touches: [
                {clientX: 100, clientY: 100},
                {clientX: 200, clientY: 200},
            ],
        })

        // Spread fingers apart
        await liveView.trigger('touchmove', {
            touches: [
                {clientX: 50, clientY: 50},
                {clientX: 250, clientY: 250},
            ],
        })

        // Scale should have changed
        const canvas = wrapper.find<HTMLCanvasElement>('.live-canvas')
        expect(canvas.element.style.transform).toContain('scale')
    })

    describe('with the real pan and zoom', () => {
        let view: Awaited<ReturnType<typeof withRealPanZoom>>

        beforeEach(async () => {
            view = await withRealPanZoom()
            // A phone's portrait live view.
            vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect')
                .mockReturnValue(DOMRect.fromRect({x: 0, y: 0, width: 390, height: 300}))
        })

        async function mountFitted() {
            // A native stream: its canvas is 3008 px wide, fitted at ~0.095.
            mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
            mocks.mockImageStream.dimensions.value = {width: 3008, height: 3008}
            const wrapper = mountLiveView()
            await nextTick()
            await wrapper.find('[title="Fit to view"]').trigger('click')
            return wrapper
        }

        async function pinch(wrapper: Awaited<ReturnType<typeof mountFitted>>, from: number, to: number) {
            const liveView = wrapper.find('.live-view')
            await liveView.trigger('touchstart', fingers(from))
            await liveView.trigger('touchmove', fingers(to))
            await liveView.trigger('touchend', {touches: []})
        }

        it('a native stream on a phone zooms back out to its fit after a pinch in', async () => {
            const wrapper = await mountFitted()
            const fit = (300 / 3008) * 0.95
            expect(view().scale.value).toBeCloseTo(fit)

            await pinch(wrapper, 100, 400)
            expect(view().scale.value).toBeCloseTo(fit * 4)

            await pinch(wrapper, 400, 100)
            expect(view().scale.value).toBeCloseTo(fit)
        })

        it('zooms about the point between the fingers, not the image centre', async () => {
            const wrapper = await mountFitted()
            const liveView = wrapper.find('.live-view')
            // Off the container's centre (195, 150), so a zoom about the centre would move it.
            const under = {x: 95, y: 50}

            await liveView.trigger('touchstart', {
                touches: [{clientX: under.x - 50, clientY: under.y}, {clientX: under.x + 50, clientY: under.y}],
            })
            const before = {scale: view().scale.value, position: view().position.value}
            await liveView.trigger('touchmove', {
                touches: [{clientX: under.x - 100, clientY: under.y}, {clientX: under.x + 100, clientY: under.y}],
            })

            expect(view().scale.value).toBeCloseTo(before.scale * 2)
            const imageX = (scale: number, x: number) => (under.x - 195 - x) / scale
            expect(imageX(view().scale.value, view().position.value.x)).toBeCloseTo(imageX(before.scale, before.position.x))
        })
    })
})
