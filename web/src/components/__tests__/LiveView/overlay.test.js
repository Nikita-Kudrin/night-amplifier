import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {nextTick} from 'vue'
import {setupMocks, mountLiveView, createMockFrameData} from './setup.js'
import {IDLE_HIDE_MS} from '../../../composables/useOverlayVisibility.js'

/** The overlay lives on `LiveViewControls`' own root element. */
function controls(wrapper) {
    return wrapper.find('.controls-overlay')
}

async function tap(wrapper, x = 5, y = 5) {
    await wrapper.trigger('mousedown', {button: 0, clientX: x, clientY: y})
    await wrapper.trigger('mouseup', {clientX: x, clientY: y})
}

describe('LiveView - Overlay auto-hide', () => {
    let mocks

    beforeEach(() => {
        vi.useFakeTimers()
        mocks = setupMocks()
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
    })

    afterEach(() => {
        vi.useRealTimers()
        vi.restoreAllMocks()
    })

    it('starts with the controls visible', async () => {
        const wrapper = mountLiveView()
        await nextTick()

        expect(controls(wrapper).classes()).not.toContain('overlay-hidden')
    })

    it('hides the controls once the idle countdown runs out', async () => {
        const wrapper = mountLiveView()
        await nextTick()

        // Two taps: away, then back — the second arms the countdown.
        await tap(wrapper)
        await tap(wrapper)
        expect(controls(wrapper).classes()).not.toContain('overlay-hidden')

        vi.advanceTimersByTime(IDLE_HIDE_MS)
        await nextTick()

        expect(controls(wrapper).classes()).toContain('overlay-hidden')
    })

    it('toggles the controls off on a tap and back on the next', async () => {
        const wrapper = mountLiveView()
        await nextTick()

        await tap(wrapper)
        expect(controls(wrapper).classes()).toContain('overlay-hidden')

        await tap(wrapper)
        expect(controls(wrapper).classes()).not.toContain('overlay-hidden')
    })

    // A drag is panning, not a tap on the image, and must leave the controls alone.
    it('does not toggle at the end of a pan', async () => {
        const wrapper = mountLiveView()
        await nextTick()

        await wrapper.trigger('mousedown', {button: 0, clientX: 10, clientY: 10})
        await wrapper.trigger('mousemove', {clientX: 90, clientY: 90})
        await wrapper.trigger('mouseup', {clientX: 90, clientY: 90})

        expect(controls(wrapper).classes()).not.toContain('overlay-hidden')
    })

    // Pressing a button must not take the controls away from under the finger.
    it('keeps the controls up when one of their buttons is pressed', async () => {
        const wrapper = mountLiveView()
        await nextTick()
        const button = wrapper.findAll('.btn-overlay')[0]

        await button.trigger('mousedown', {button: 0, clientX: 5, clientY: 5})
        await button.trigger('mouseup', {clientX: 5, clientY: 5})

        expect(controls(wrapper).classes()).not.toContain('overlay-hidden')
    })

    it('keeps the controls up for as long as a pan lasts', async () => {
        const wrapper = mountLiveView()
        await nextTick()
        await tap(wrapper)
        await tap(wrapper)

        await wrapper.trigger('mousedown', {button: 0, clientX: 10, clientY: 10})
        vi.advanceTimersByTime(IDLE_HIDE_MS - 100)
        await wrapper.trigger('mousemove', {clientX: 40, clientY: 40})
        vi.advanceTimersByTime(IDLE_HIDE_MS - 100)
        await nextTick()

        expect(controls(wrapper).classes()).not.toContain('overlay-hidden')
    })

    /**
     * The whole point of the fade, on the device it exists for. A browser replays
     * a touch as compatibility mouse events, and this view's touch listeners are
     * passive so it cannot suppress them — handled naively, one tap ran the toggle
     * twice and the controls never moved on a phone.
     */
    it('toggles once for one tap, not once per synthesised mouse event', async () => {
        const wrapper = mountLiveView()
        await nextTick()

        await wrapper.trigger('touchstart', {touches: [{clientX: 5, clientY: 5}]})
        await wrapper.trigger('touchend', {touches: [], changedTouches: [{clientX: 5, clientY: 5}]})
        await wrapper.trigger('mousedown', {button: 0, clientX: 5, clientY: 5})
        await wrapper.trigger('mouseup', {clientX: 5, clientY: 5})

        expect(controls(wrapper).classes()).toContain('overlay-hidden')
    })

    it('brings them back on the next tap', async () => {
        const wrapper = mountLiveView()
        await nextTick()
        const tapWithGhost = async () => {
            await wrapper.trigger('touchstart', {touches: [{clientX: 5, clientY: 5}]})
            await wrapper.trigger('touchend', {touches: [], changedTouches: [{clientX: 5, clientY: 5}]})
            await wrapper.trigger('mousedown', {button: 0, clientX: 5, clientY: 5})
            await wrapper.trigger('mouseup', {clientX: 5, clientY: 5})
        }

        await tapWithGhost()
        await tapWithGhost()

        expect(controls(wrapper).classes()).not.toContain('overlay-hidden')
    })

    // A cancelled gesture is the system taking over, not a tap.
    it('does not toggle on touchcancel', async () => {
        const wrapper = mountLiveView()
        await nextTick()

        await wrapper.trigger('touchstart', {touches: [{clientX: 5, clientY: 5}]})
        await wrapper.trigger('touchcancel', {touches: [], changedTouches: [{clientX: 5, clientY: 5}]})

        expect(controls(wrapper).classes()).not.toContain('overlay-hidden')
    })

    // Keyboard users get no pointer events at all; focus has to count as activity
    // or Tab lands on a button faded to nothing.
    it('shows the controls when something inside them takes focus', async () => {
        const wrapper = mountLiveView()
        await nextTick()
        await tap(wrapper)
        expect(controls(wrapper).classes()).toContain('overlay-hidden')

        await wrapper.trigger('focusin')

        expect(controls(wrapper).classes()).not.toContain('overlay-hidden')
    })

    it('fades on its own if the view is never touched', async () => {
        const wrapper = mountLiveView()
        await nextTick()

        vi.advanceTimersByTime(IDLE_HIDE_MS)
        await nextTick()

        expect(controls(wrapper).classes()).toContain('overlay-hidden')
    })

    // The Push-To chevron is navigation, not chrome: it is the one overlay that
    // stays put while the rest fades.
    it('never hides the Push-To chevron', async () => {
        const wrapper = mountLiveView({
            pushDirection: {angleDeg: 45, distanceDeg: 10, isClose: false, directionHint: 'NE'},
            currentTarget: {id: 'M42', name: 'Orion Nebula'},
        })
        await nextTick()

        await tap(wrapper)
        vi.advanceTimersByTime(IDLE_HIDE_MS * 2)
        await nextTick()

        const arrow = wrapper.findComponent({name: 'GuideArrow'})
        expect(arrow.exists()).toBe(true)
        expect(arrow.classes()).not.toContain('overlay-hidden')
    })
})

describe('LiveView - Fit all on viewport changes', () => {
    let mocks

    beforeEach(() => {
        vi.useFakeTimers()
        mocks = setupMocks()
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
    })

    afterEach(() => {
        vi.useRealTimers()
        vi.restoreAllMocks()
    })

    it('fits the image on a resize while fullscreen', async () => {
        const wrapper = mountLiveView()
        await nextTick()
        mocks.mockPanZoom.isFullscreen.value = true
        mocks.mockPanZoom.fitToView.mockClear()

        window.dispatchEvent(new Event('resize'))
        vi.advanceTimersByTime(250)

        expect(mocks.mockPanZoom.fitToView).toHaveBeenCalled()
        wrapper.unmount()
    })

    // A rotation does not always come with a `resize` on mobile Safari.
    it('fits the image on an orientation change while fullscreen', async () => {
        const wrapper = mountLiveView()
        await nextTick()
        mocks.mockPanZoom.isFullscreen.value = true
        mocks.mockPanZoom.fitToView.mockClear()

        window.dispatchEvent(new Event('orientationchange'))
        vi.advanceTimersByTime(250)

        expect(mocks.mockPanZoom.fitToView).toHaveBeenCalled()
        wrapper.unmount()
    })

    // Windowed, the user's own pan and zoom must survive a resize — dragging the
    // sidebar should not reframe the image.
    it('leaves the view alone on a resize outside fullscreen', async () => {
        const wrapper = mountLiveView()
        await nextTick()
        mocks.mockPanZoom.isFullscreen.value = false
        mocks.mockPanZoom.fitToView.mockClear()

        window.dispatchEvent(new Event('resize'))
        vi.advanceTimersByTime(250)

        expect(mocks.mockPanZoom.fitToView).not.toHaveBeenCalled()
        wrapper.unmount()
    })

    it('still reports the new resolution on a resize', async () => {
        const wrapper = mountLiveView()
        await nextTick()
        mocks.mockImageStream.sendResolution.mockClear()

        window.dispatchEvent(new Event('resize'))
        vi.advanceTimersByTime(250)

        expect(mocks.mockImageStream.sendResolution).toHaveBeenCalled()
        wrapper.unmount()
    })

    it('stops listening once unmounted', async () => {
        const wrapper = mountLiveView()
        await nextTick()
        wrapper.unmount()
        mocks.mockImageStream.sendResolution.mockClear()

        window.dispatchEvent(new Event('orientationchange'))
        vi.advanceTimersByTime(250)

        expect(mocks.mockImageStream.sendResolution).not.toHaveBeenCalled()
    })
})

/**
 * Entering fullscreen fits all. The composable owns the transition detection, so
 * what LiveView must get right is handing it a callback that fits.
 */
describe('LiveView - Fit all on entering fullscreen', () => {
    let mocks

    beforeEach(() => {
        vi.useFakeTimers()
        mocks = setupMocks()
        mocks.mockImageStream.frameData.value = createMockFrameData(2, 2)
        mocks.mockImageStream.dimensions.value = {width: 2, height: 2}
    })

    afterEach(() => {
        vi.useRealTimers()
        vi.restoreAllMocks()
    })

    it('passes usePanZoom an onChange that fits the view', async () => {
        const {usePanZoom} = await import('../../../composables/usePanZoom.js')
        const wrapper = mountLiveView()
        await nextTick()
        mocks.mockPanZoom.fitToView.mockClear()

        const {onChange} = usePanZoom.mock.calls[usePanZoom.mock.calls.length - 1][0]
        onChange(false)
        vi.advanceTimersByTime(50)

        expect(mocks.mockPanZoom.fitToView).toHaveBeenCalled()
        wrapper.unmount()
    })
})
