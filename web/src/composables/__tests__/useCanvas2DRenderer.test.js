import { describe, it, expect, vi, beforeEach } from 'vitest'
import { useCanvas2DRenderer } from '../useCanvas2DRenderer.js'

describe('useCanvas2DRenderer', () => {
    let canvas
    let ctx2d

    beforeEach(() => {
        ctx2d = {
            createImageData: vi.fn(() => ({ data: new Uint8ClampedArray(4 * 10 * 10), width: 10, height: 10 })),
            putImageData: vi.fn(),
            drawImage: vi.fn(),
        }
        canvas = {
            getContext: vi.fn((type) => {
                if (type === '2d') return ctx2d
                return null
            }),
            width: 0,
            height: 0,
        }
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
        render(canvas, frameData, 10, 10)

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
        global.ImageBitmap = MockImageBitmap

        const frameData = new MockImageBitmap()
        render(canvas, frameData, 10, 10)

        expect(ctx2d.drawImage).toHaveBeenCalledWith(frameData, 0, 0)
        expect(ctx2d.putImageData).not.toHaveBeenCalled()
        
        delete global.ImageBitmap
    })
})
