import {describe, it, expect, vi, beforeEach} from 'vitest'
import {usePanZoom} from './usePanZoom'
import type {PanZoomOptions, View} from './usePanZoom'

// Mock ZOOM_LIMITS
vi.mock('../constants', () => ({
    FIT_MARGIN: 0.95,
    ZOOM_LIMITS: {
        minOfFit: 0.5,
        max: 10,
        wheelZoomIn: 1.1,
        wheelZoomOut: 0.9,
        zoomInFactor: 1.25,
        zoomOutFactor: 0.8,
    },
}))

/** A wheel notch at the pointer: negative `deltaY` zooms in. */
function wheel(deltaY: number, clientX = 0, clientY = 0) {
    return {deltaY, clientX, clientY, preventDefault: vi.fn()}
}

/** Two fingers `distance` apart, level about `centre`. */
function fingers(distance: number, centre = {x: 0, y: 0}) {
    return {
        touches: [
            {clientX: centre.x - distance / 2, clientY: centre.y},
            {clientX: centre.x + distance / 2, clientY: centre.y},
        ],
    }
}

/** A pinch from `from` px apart to each of `to`, about one centre. */
function pinch(panZoom: ReturnType<typeof usePanZoom>, from: number, ...to: number[]) {
    panZoom.handleTouchStart(fingers(from))
    for (const distance of to) panZoom.handleTouchMove(fingers(distance))
    panZoom.handleTouchEnd({touches: []})
}

/** The image point (canvas px from its centre) a screen point shows, given the canvas centre at rest. */
function imagePointAt(view: View, centre: {x: number; y: number}, screen: {x: number; y: number}) {
    return {
        x: (screen.x - centre.x - view.position.x) / view.scale,
        y: (screen.y - centre.y - view.position.y) / view.scale,
    }
}

describe('usePanZoom', () => {
    let panZoom: ReturnType<typeof usePanZoom>

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
            const mockEvent = wheel(-100)

            panZoom.handleWheel(mockEvent)

            expect(panZoom.scale.value).toBeCloseTo(1.1)
            expect(mockEvent.preventDefault).toHaveBeenCalled()
        })

        it('zooms out on positive deltaY', () => {
            panZoom.handleWheel(wheel(100))

            expect(panZoom.scale.value).toBeCloseTo(0.9)
        })

        it('clamps scale to half the fit, a fit of 1 while none is known', () => {
            panZoom.scale.value = 0.52

            panZoom.handleWheel(wheel(100))

            // 0.52 * 0.9 = 0.468 -> clamped to 0.5
            expect(panZoom.scale.value).toBe(0.5)
        })

        it('clamps scale to max', () => {
            panZoom.scale.value = 9.5

            panZoom.handleWheel(wheel(-100))

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

            panZoom.handleTouchEnd({touches: []})

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

    describe('zoom bounds follow the fitted scale', () => {
        // A phone fits a native 3008 px stream at about 0.12: far below any absolute floor.
        const PHONE_FIT = 0.12

        function atFit(fit: number | null, options: PanZoomOptions = {}) {
            const view = usePanZoom({fitScale: () => fit, ...options})
            view.scale.value = fit ?? 1
            return view
        }

        it('a slight pinch out from the fitted view stays near it, not at a fixed floor', () => {
            const view = atFit(PHONE_FIT)

            pinch(view, 100, 105)

            expect(view.scale.value).toBeCloseTo(PHONE_FIT * 1.05)
        })

        it('pinching out after pinching in returns to the fit, and no further than half of it', () => {
            const view = atFit(PHONE_FIT)

            pinch(view, 100, 300)
            expect(view.scale.value).toBeCloseTo(PHONE_FIT * 3)

            pinch(view, 300, 100)
            expect(view.scale.value).toBeCloseTo(PHONE_FIT)

            pinch(view, 100, 10)
            expect(view.scale.value).toBeCloseTo(PHONE_FIT / 2)
        })

        it('zoom out and the wheel shrink a fitted image rather than enlarging it', () => {
            const view = atFit(PHONE_FIT)

            view.zoomOut()
            expect(view.scale.value).toBeCloseTo(PHONE_FIT * 0.8)

            view.handleWheel(wheel(100))
            expect(view.scale.value).toBeCloseTo(PHONE_FIT * 0.8 * 0.9)
        })

        it('a fit above the maximum is still reachable without a jump', () => {
            const view = atFit(12)

            pinch(view, 100, 95)
            expect(view.scale.value).toBeCloseTo(12 * 0.95)

            pinch(view, 100, 200)
            expect(view.scale.value).toBeCloseTo(12)
        })

        it('a view left outside the bounds moves on from where it is, without jumping', () => {
            const view = atFit(1)
            view.scale.value = 0.3

            pinch(view, 100, 105)
            expect(view.scale.value).toBeCloseTo(0.315)

            pinch(view, 100, 50)
            expect(view.scale.value).toBeCloseTo(0.315)
        })

        it('falls back to the fixed range while no fit is known', () => {
            const view = atFit(null)

            pinch(view, 100, 10)
            expect(view.scale.value).toBeCloseTo(0.5)

            pinch(view, 100, 10000)
            expect(view.scale.value).toBeCloseTo(10)
        })
    })

    describe('zoom anchor', () => {
        const centre = {x: 200, y: 150}

        function anchored() {
            return usePanZoom({fitScale: () => 1, viewCentre: () => centre})
        }

        it('a pinch keeps the image point between the fingers under them', () => {
            const view = anchored()
            view.position.value = {x: 30, y: -20}
            const midpoint = {x: 120, y: 90}
            const before = imagePointAt({scale: 1, position: view.position.value}, centre, midpoint)

            view.handleTouchStart(fingers(100, midpoint))
            view.handleTouchMove(fingers(250, midpoint))

            expect(view.scale.value).toBeCloseTo(2.5)
            const after = imagePointAt({scale: view.scale.value, position: view.position.value}, centre, midpoint)
            expect(after.x).toBeCloseTo(before.x)
            expect(after.y).toBeCloseTo(before.y)
        })

        it('the image point follows the fingers when they move together', () => {
            const view = anchored()
            const start = {x: 120, y: 90}
            const end = {x: 160, y: 110}
            const before = imagePointAt({scale: 1, position: {x: 0, y: 0}}, centre, start)

            view.handleTouchStart(fingers(100, start))
            view.handleTouchMove(fingers(200, end))

            const after = imagePointAt({scale: view.scale.value, position: view.position.value}, centre, end)
            expect(after.x).toBeCloseTo(before.x)
            expect(after.y).toBeCloseTo(before.y)
        })

        it('the wheel keeps the image point under the pointer', () => {
            const view = anchored()
            const pointer = {x: 300, y: 60}
            const before = imagePointAt({scale: 1, position: {x: 0, y: 0}}, centre, pointer)

            view.handleWheel(wheel(-100, pointer.x, pointer.y))

            expect(view.scale.value).toBeCloseTo(1.1)
            const after = imagePointAt({scale: view.scale.value, position: view.position.value}, centre, pointer)
            expect(after.x).toBeCloseTo(before.x)
            expect(after.y).toBeCloseTo(before.y)
        })

        it('without a view centre the position is left alone (zoom about the canvas centre)', () => {
            panZoom.position.value = {x: 30, y: -20}

            panZoom.handleTouchStart(fingers(100, {x: 120, y: 90}))
            panZoom.handleTouchMove(fingers(200, {x: 120, y: 90}))

            expect(panZoom.scale.value).toBeCloseTo(2)
            expect(panZoom.position.value).toEqual({x: 30, y: -20})
        })
    })

    describe('fingers lifting and landing', () => {
        it('a pinch that drops to one finger pans on from where it is', () => {
            panZoom.handleTouchStart(fingers(100))
            panZoom.handleTouchMove(fingers(200))
            const position = panZoom.position.value

            panZoom.handleTouchEnd({touches: [{clientX: 100, clientY: 0}]})
            panZoom.handleTouchMove({touches: [{clientX: 130, clientY: 10}]})

            expect(panZoom.isDragging.value).toBe(true)
            expect(panZoom.position.value).toEqual({x: position.x + 30, y: position.y + 10})
        })

        it('a second finger landing mid-pan starts a pinch from the panned view', () => {
            panZoom.handleTouchStart({touches: [{clientX: 0, clientY: 0}]})
            panZoom.handleTouchMove({touches: [{clientX: 40, clientY: 0}]})

            panZoom.handleTouchStart(fingers(100))
            panZoom.handleTouchMove(fingers(150))

            expect(panZoom.isDragging.value).toBe(false)
            expect(panZoom.scale.value).toBeCloseTo(1.5)
            expect(panZoom.position.value).toEqual({x: 40, y: 0})
        })

        it('a third finger lifting re-anchors the pinch on the two left', () => {
            panZoom.handleTouchStart(fingers(100))
            panZoom.handleTouchMove(fingers(200))

            // The two left are 50 px apart: carrying the old anchor would read that as a zoom out.
            panZoom.handleTouchEnd(fingers(50))
            panZoom.handleTouchMove(fingers(50))

            expect(panZoom.scale.value).toBeCloseTo(2)
        })

        it('a cancelled gesture ends both the pinch and the pan', () => {
            panZoom.handleTouchStart(fingers(100))
            panZoom.handleTouchCancel()
            panZoom.handleTouchMove(fingers(300))
            panZoom.handleTouchMove({touches: [{clientX: 50, clientY: 50}]})

            expect(panZoom.scale.value).toBe(1)
            expect(panZoom.position.value).toEqual({x: 0, y: 0})
            expect(panZoom.isDragging.value).toBe(false)
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
