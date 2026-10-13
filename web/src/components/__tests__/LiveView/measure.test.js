import {nextTick} from 'vue'
import {setupMocks, mountLiveView} from './setup.js'

const DIRECTION = {angleDeg: 45, distanceDeg: 10, directionHint: 'NorthEast', isClose: false}
const TARGET = {designation: 'M31', raDegrees: 10.68, decDegrees: 41.27}

/** Lays the container out at 800x600 and the canvas where the pan puts it, counting reads. */
function layOut(wrapper, mocks) {
    const reads = {canvas: 0}
    wrapper.element.getBoundingClientRect = () => ({left: 0, top: 0, width: 800, height: 600})
    wrapper.find('canvas').element.getBoundingClientRect = () => {
        reads.canvas++
        const {x, y} = mocks.mockPanZoom.position.value
        const scale = mocks.mockPanZoom.scale.value
        return {left: 100 + x, top: 50 + y, width: 400 * scale, height: 300 * scale}
    }
    return reads
}

describe('LiveView - measuring the canvas', () => {
    let mocks

    beforeEach(() => {
        vi.useFakeTimers()
        mocks = setupMocks()
    })

    afterEach(() => {
        vi.useRealTimers()
        vi.restoreAllMocks()
    })

    async function showFrame(width, height) {
        mocks.mockImageStream.frameData.value = new Uint8Array(width * height * 3)
        mocks.mockImageStream.dimensions.value = {width, height}
        await nextTick()
        await vi.advanceTimersByTimeAsync(20)
    }

    it('re-measures on a new frame size, not on every frame', async () => {
        const wrapper = mountLiveView()
        const reads = layOut(wrapper, mocks)
        await showFrame(400, 300)
        const afterFirst = reads.canvas

        for (let i = 0; i < 5; i++) await showFrame(400, 300)
        expect(reads.canvas).toBe(afterFirst)

        await showFrame(640, 480)
        expect(reads.canvas).toBeGreaterThan(afterFirst)
    })

    it('moves the arrow with the image when the view pans or zooms between frames', async () => {
        const wrapper = mountLiveView({pushDirection: DIRECTION, currentTarget: TARGET})
        layOut(wrapper, mocks)
        await showFrame(400, 300)
        const arrow = () => wrapper.findComponent({name: 'GuideArrow'})
        expect(arrow().props('imageLeft')).toBe(100)

        mocks.mockPanZoom.position.value = {x: -30, y: 20}
        await nextTick()
        expect(arrow().props('imageLeft')).toBe(70)
        expect(arrow().props('imageTop')).toBe(70)

        mocks.mockPanZoom.scale.value = 2
        await nextTick()
        expect(arrow().props('imageWidth')).toBe(800)
    })

    it('leaves no timer behind when it goes away', async () => {
        const wrapper = mountLiveView()
        layOut(wrapper, mocks)
        mocks.mockImageStream.frameData.value = new Uint8Array(400 * 300 * 3)
        mocks.mockImageStream.dimensions.value = {width: 400, height: 300}
        await nextTick()

        wrapper.unmount()

        expect(vi.getTimerCount()).toBe(0)
    })
})
