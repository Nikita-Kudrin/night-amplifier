import { describe, it, expect, expectTypeOf, vi, beforeEach } from 'vitest'
import { useCanvas2DRenderer } from '../useCanvas2DRenderer.js'
import type { useWebGLRenderer } from '../useWebGLRenderer.js'

function createContext() {
    return {
        createImageData: vi.fn(() => ({ data: new Uint8ClampedArray(4 * 10 * 10), width: 10, height: 10 })),
        putImageData: vi.fn(),
        drawImage: vi.fn(),
    }
}

describe('useCanvas2DRenderer', () => {
    let canvas: HTMLCanvasElement
    let ctx2d: ReturnType<typeof createContext>

    beforeEach(() => {
        ctx2d = createContext()
        const fake = {
            getContext: vi.fn((type: string) => {
                if (type === '2d') return ctx2d
                return null
            }),
            width: 0,
            height: 0,
        }
        canvas = fake as unknown as HTMLCanvasElement
    })

    it('initializes correctly', () => {
        const { init, backend, isInitialized } = useCanvas2DRenderer()
        const success = init(canvas)
        expect(success).toBe(true)
        expect(backend.value).toBe('canvas2d')
        expect(isInitialized()).toBe(true)
    })

    it('fails to initialize if canvas is missing', () => {
        const { init, backend, isInitialized } = useCanvas2DRenderer()
        const consoleSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
        const success = init(null)
        expect(success).toBe(false)
        expect(backend.value).toBe('unknown')
        expect(isInitialized()).toBe(false)
        consoleSpy.mockRestore()
    })

    it('renders Uint8Array correctly', () => {
        const { init, render } = useCanvas2DRenderer()
        init(canvas)

        const frameData = new Uint8Array(10 * 10 * 3)
        expect(render(canvas, frameData, 10, 10)).toBe(true)

        expect(ctx2d.createImageData).toHaveBeenCalledWith(10, 10)
        expect(ctx2d.putImageData).toHaveBeenCalled()
        expect(ctx2d.drawImage).not.toHaveBeenCalled()
    })

    it('renders ImageBitmap correctly', () => {
        const { init, render } = useCanvas2DRenderer()
        init(canvas)

        // Mock ImageBitmap
        class MockImageBitmap {}
        // Because ImageBitmap might not exist in JSDOM, we mock it globally for this test
        vi.stubGlobal('ImageBitmap', MockImageBitmap)

        const frameData = new MockImageBitmap() as unknown as ImageBitmap
        expect(render(canvas, frameData, 10, 10)).toBe(true)

        expect(ctx2d.drawImage).toHaveBeenCalledWith(frameData, 0, 0)
        expect(ctx2d.putImageData).not.toHaveBeenCalled()
    })

    it('takes what the WebGL renderer takes', () => {
        const { render } = useCanvas2DRenderer()
        expectTypeOf(render).toEqualTypeOf<ReturnType<typeof useWebGLRenderer>['render']>()
    })

    it('reports that it drew nothing before init or without a frame', () => {
        const { init, render } = useCanvas2DRenderer()
        expect(render(canvas, new Uint8Array(10 * 10 * 3), 10, 10)).toBe(false)
        init(canvas)
        expect(render(canvas, null, 10, 10)).toBe(false)
    })
})
