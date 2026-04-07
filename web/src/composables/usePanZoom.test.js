import {describe, it, expect, vi, beforeEach} from 'vitest'
import {usePanZoom} from './usePanZoom.js'

// Mock ZOOM_LIMITS
vi.mock('../constants', () => ({
    ZOOM_LIMITS: {
        min: 0.1,
        max: 10,
        wheelZoomIn: 1.1,
        wheelZoomOut: 0.9,
        zoomInFactor: 1.25,
        zoomOutFactor: 0.8,
    },
}))

describe('usePanZoom', () => {
    let panZoom

    beforeEach(() => {
        panZoom = usePanZoom()
        // Reset any document state
        Object.defineProperty(document, 'fullscreenElement', {
            value: null,
            writable: true,
            configurable: true,
        })
    })

    describe('initial state', () => {
        it('starts at scale 1', () => {
            expect(panZoom.scale.value).toBe(1)
        })

        it('starts at position (0, 0)', () => {
            expect(panZoom.position.value).toEqual({x: 0, y: 0})
        })

        it('starts not dragging', () => {
            expect(panZoom.isDragging.value).toBe(false)
        })

        it('starts not fullscreen', () => {
            expect(panZoom.isFullscreen.value).toBe(false)
        })
    })

    describe('canvasStyle', () => {
        it('returns correct transform', () => {
            panZoom.scale.value = 2
            panZoom.position.value = {x: 100, y: 50}

            expect(panZoom.canvasStyle.value.transform).toBe('translate(100px, 50px) scale(2)')
        })

        it('returns grab cursor when not dragging', () => {
            panZoom.isDragging.value = false

            expect(panZoom.canvasStyle.value.cursor).toBe('grab')
        })

        it('returns grabbing cursor when dragging', () => {
            panZoom.isDragging.value = true

            expect(panZoom.canvasStyle.value.cursor).toBe('grabbing')
        })
    })

    describe('handleWheel', () => {
        it('zooms in on negative deltaY', () => {
            const mockEvent = {deltaY: -100, preventDefault: vi.fn()}

            panZoom.handleWheel(mockEvent)

            expect(panZoom.scale.value).toBeCloseTo(1.1)
            expect(mockEvent.preventDefault).toHaveBeenCalled()
        })

        it('zooms out on positive deltaY', () => {
            const mockEvent = {deltaY: 100, preventDefault: vi.fn()}

            panZoom.handleWheel(mockEvent)

            expect(panZoom.scale.value).toBeCloseTo(0.9)
        })

        it('clamps scale to min', () => {
            panZoom.scale.value = 0.11
            const mockEvent = {deltaY: 100, preventDefault: vi.fn()}

            panZoom.handleWheel(mockEvent)

            // 0.11 * 0.9 = 0.099 -> clamped to 0.1
            expect(panZoom.scale.value).toBe(0.1) // min
        })

        it('clamps scale to max', () => {
            panZoom.scale.value = 9.5
            const mockEvent = {deltaY: -100, preventDefault: vi.fn()}

            panZoom.handleWheel(mockEvent)

            expect(panZoom.scale.value).toBe(10) // max
        })
    })

    describe('handleMouseDown', () => {
        it('starts dragging on left click', () => {
            const mockEvent = {
                button: 0,
                clientX: 100,
                clientY: 200,
            }

            panZoom.handleMouseDown(mockEvent)

            expect(panZoom.isDragging.value).toBe(true)
        })

        it('ignores right click', () => {
            const mockEvent = {
                button: 2,
                clientX: 100,
                clientY: 200,
            }

            panZoom.handleMouseDown(mockEvent)

            expect(panZoom.isDragging.value).toBe(false)
        })

        it('ignores middle click', () => {
            const mockEvent = {
                button: 1,
                clientX: 100,
                clientY: 200,
            }

            panZoom.handleMouseDown(mockEvent)

            expect(panZoom.isDragging.value).toBe(false)
        })
    })

    describe('handleMouseMove', () => {
        it('updates position when dragging', () => {
            panZoom.handleMouseDown({button: 0, clientX: 100, clientY: 100})
            panZoom.handleMouseMove({clientX: 150, clientY: 120})

            expect(panZoom.position.value).toEqual({x: 50, y: 20})
        })

        it('ignores movement when not dragging', () => {
            panZoom.handleMouseMove({clientX: 150, clientY: 120})

            expect(panZoom.position.value).toEqual({x: 0, y: 0})
        })
    })

    describe('handleMouseUp', () => {
        it('stops dragging', () => {
            panZoom.isDragging.value = true

            panZoom.handleMouseUp()

            expect(panZoom.isDragging.value).toBe(false)
        })
    })

    describe('handleTouchStart', () => {
        it('starts dragging on single touch', () => {
            const mockEvent = {
                touches: [{clientX: 100, clientY: 100}],
            }

            panZoom.handleTouchStart(mockEvent)

            expect(panZoom.isDragging.value).toBe(true)
        })

        it('does not start dragging on two-finger touch', () => {
            const mockEvent = {
                touches: [
                    {clientX: 100, clientY: 100},
                    {clientX: 200, clientY: 200},
                ],
            }

            panZoom.handleTouchStart(mockEvent)

            expect(panZoom.isDragging.value).toBe(false)
        })
    })

    describe('handleTouchMove', () => {
        it('updates position on single touch drag', () => {
            panZoom.handleTouchStart({touches: [{clientX: 100, clientY: 100}]})
            panZoom.handleTouchMove({touches: [{clientX: 150, clientY: 120}]})

            expect(panZoom.position.value).toEqual({x: 50, y: 20})
        })

        it('updates scale on pinch zoom', () => {
            // Start with fingers 100px apart
            panZoom.handleTouchStart({
                touches: [
                    {clientX: 0, clientY: 0},
                    {clientX: 100, clientY: 0},
                ],
            })

            // Move fingers to 200px apart (2x zoom)
            panZoom.handleTouchMove({
                touches: [
                    {clientX: 0, clientY: 0},
                    {clientX: 200, clientY: 0},
                ],
            })

            expect(panZoom.scale.value).toBeCloseTo(2)
        })
    })

    describe('handleTouchEnd', () => {
        it('stops dragging', () => {
            panZoom.isDragging.value = true

            panZoom.handleTouchEnd()

            expect(panZoom.isDragging.value).toBe(false)
        })
    })

    describe('zoomIn', () => {
        it('increases scale by zoom factor', () => {
            panZoom.zoomIn()

            expect(panZoom.scale.value).toBeCloseTo(1.25)
        })
    })

    describe('zoomOut', () => {
        it('decreases scale by zoom factor', () => {
            panZoom.zoomOut()

            expect(panZoom.scale.value).toBeCloseTo(0.8)
        })
    })

    describe('resetView', () => {
        it('resets scale to 1', () => {
            panZoom.scale.value = 2

            panZoom.resetView()

            expect(panZoom.scale.value).toBe(1)
        })

        it('resets position to origin', () => {
            panZoom.position.value = {x: 100, y: 200}

            panZoom.resetView()

            expect(panZoom.position.value).toEqual({x: 0, y: 0})
        })
    })

    describe('fitToView', () => {
        it('calculates scale to fit container', () => {
            const containerRect = {width: 800, height: 600}
            const canvasWidth = 1600
            const canvasHeight = 900

            panZoom.fitToView(containerRect, canvasWidth, canvasHeight)

            // Scale should be min(800/1600, 600/900, 1) * 0.95 = 0.5 * 0.95 = 0.475
            expect(panZoom.scale.value).toBeCloseTo(0.475)
        })

        it('resets position to origin', () => {
            panZoom.position.value = {x: 100, y: 200}

            const containerRect = {width: 800, height: 600}
            panZoom.fitToView(containerRect, 800, 600)

            expect(panZoom.position.value).toEqual({x: 0, y: 0})
        })

        it('does not scale above 1 (95%)', () => {
            const containerRect = {width: 800, height: 600}
            const canvasWidth = 100
            const canvasHeight = 100

            panZoom.fitToView(containerRect, canvasWidth, canvasHeight)

            // Scale should be min(8, 6, 1) * 0.95 = 0.95
            expect(panZoom.scale.value).toBeCloseTo(0.95)
        })

        it('handles null containerRect', () => {
            const initialScale = panZoom.scale.value

            panZoom.fitToView(null, 100, 100)

            expect(panZoom.scale.value).toBe(initialScale)
        })
    })

    describe('handleFullscreenChange', () => {
        it('sets isFullscreen to true when in fullscreen', () => {
            Object.defineProperty(document, 'fullscreenElement', {
                value: document.body,
                configurable: true,
            })

            panZoom.handleFullscreenChange()

            expect(panZoom.isFullscreen.value).toBe(true)
        })

        it('sets isFullscreen to false when not in fullscreen', () => {
            panZoom.isFullscreen.value = true
            Object.defineProperty(document, 'fullscreenElement', {
                value: null,
                configurable: true,
            })

            panZoom.handleFullscreenChange()

            expect(panZoom.isFullscreen.value).toBe(false)
        })
    })
})
