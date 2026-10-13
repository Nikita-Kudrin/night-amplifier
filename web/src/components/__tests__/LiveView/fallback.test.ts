import {nextTick, ref, shallowRef} from 'vue'
import {mountWithDefaults} from '../../../test/mount.js'
import {installCanvasContexts, quietly} from '../../../test/renderStubs.js'
import {capabilities} from '../../../test/fixtures.js'

// The real renderers, unlike the other LiveView files: only the socket is faked.
vi.mock('../../../composables/useImageStream.js', () => ({useImageStream: vi.fn()}))

import {useImageStream} from '../../../composables/useImageStream.js'
import type {FrameData, FrameSize, ImageStream} from '../../../composables/useImageStream.js'
import LiveView from '../../LiveView.vue'

function mountWithStream() {
    const stream = {
        connected: ref(true),
        error: ref<string | null>(null),
        decodeError: ref<string | null>(null),
        frameData: shallowRef<FrameData | null>(null),
        dimensions: ref<FrameSize>({width: 0, height: 0}),
        isJpeg: ref(false),
        isDynamicJpeg: false,
        frameNumber: ref(0),
        fps: ref(0),
        connect: vi.fn(),
        disconnect: vi.fn(),
        clearFrameData: vi.fn(),
    } satisfies ImageStream
    vi.mocked(useImageStream).mockReturnValue(stream)
    const wrapper = quietly(() =>
        mountWithDefaults(LiveView, {provide: {capabilities: capabilities({debug_logging: true})}}))
    return {wrapper, stream}
}

describe('LiveView - renderer fallback', () => {
    afterEach(() => vi.restoreAllMocks())

    it('shows a Canvas 2D frame when the WebGL shaders fail', async () => {
        const {context2dOf} = installCanvasContexts({compiling: []})
        const {wrapper, stream} = mountWithStream()

        stream.dimensions.value = {width: 3, height: 2}
        stream.frameData.value = new Uint8Array(3 * 2 * 3)
        await nextTick()

        const ctx = context2dOf(wrapper.find('canvas').element)
        expect(ctx.type).toBe('2d')
        expect(ctx.putImageData).toHaveBeenCalledTimes(1)
        expect(wrapper.find('.render-backend').text()).toBe('Canvas 2D (3x2)')
    })

    it('shows a WebGL1 frame when only WebGL2 fails', async () => {
        const {glOf} = installCanvasContexts({compiling: ['webgl']})
        const {wrapper, stream} = mountWithStream()

        stream.dimensions.value = {width: 3, height: 2}
        stream.frameData.value = new Uint8Array(3 * 2 * 3)
        await nextTick()

        expect(glOf(wrapper.find('canvas').element).uploads).toEqual([{width: 3, height: 2}])
        expect(wrapper.find('.render-backend').text()).toBe('WebGL1 (3x2)')
    })
})
