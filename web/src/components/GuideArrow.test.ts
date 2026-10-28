import {mount} from '@vue/test-utils'
import type {VueWrapper} from '@vue/test-utils'
import GuideArrow from './GuideArrow.vue'
import type {pushDirectionFrom, ClientPushDirection} from '../composables/events/pushToEvents'
import type {PushDirection} from '../composables/api.events'
import {DIRECTION_HINT_VALUES} from '../composables/api.types'
import type {FrameSize} from '../composables/useImageStream'

type ArrowProps = InstanceType<typeof GuideArrow>['$props']

/**
 * The arrow's props as the tables below spell them: the direction's fields and the image box,
 * flat. `frame` defaults to the box's size, as a canvas off the circular view takes the frame's.
 */
type FlatProps = Partial<Pick<ClientPushDirection, 'angleDeg' | 'distanceDeg' | 'isClose' | 'directionHint'>> &
    Omit<ArrowProps, 'direction' | 'bounds' | 'frame'> & {
        frame?: FrameSize
        imageLeft?: number
        imageTop?: number
        imageWidth?: number
        imageHeight?: number
    }

const DEFAULT_PROPS = {
    angleDeg: 0,
    distanceDeg: 10,
    isClose: false,
    directionHint: 'N',
} satisfies FlatProps

const IMAGE_PROPS = {
    imageLeft: 0,
    imageTop: 0,
    imageWidth: 800,
    imageHeight: 600,
}

/** Mounts from flat props, as the tables below spell them: direction and image box split out. */
function mountArrow(propsOverrides: FlatProps = {}) {
    const {angleDeg, distanceDeg, isClose, directionHint, imageLeft, imageTop, imageWidth, imageHeight, frame, ...rest} =
        {...DEFAULT_PROPS, ...propsOverrides}
    const direction: ClientPushDirection = {angleDeg, distanceDeg, isClose, directionHint, fovDeg: 0, solvedCamera: null}
    const bounds = imageWidth !== undefined && imageHeight !== undefined
        ? {left: imageLeft ?? 0, top: imageTop ?? 0, width: imageWidth, height: imageHeight}
        : undefined
    const props: ArrowProps = {
        direction,
        frame: frame ?? {width: bounds?.width ?? 400, height: bounds?.height ?? 300},
        ...rest,
        ...(bounds && {bounds}),
    }
    return mount(GuideArrow, {props})
}

/** The arrow container's position; throws when its style names none. */
function getPosition(wrapper: VueWrapper) {
    const style = wrapper.find('.guide-arrow-container').attributes('style') ?? ''
    const px = (name: string, pattern: RegExp) => {
        const match = style.match(pattern)
        if (!match) throw new Error(`no ${name} in "${style}"`)
        return parseFloat(match[1])
    }
    return {
        left: px('left', /left:\s*([\d.]+)px/),
        top: px('top', /top:\s*([\d.]+)px/),
    }
}

/** A numeric attribute, NaN when it is missing. */
function numberAttr(wrapper: VueWrapper, selector: string, name: string) {
    return parseFloat(wrapper.find(selector).attributes(name) ?? '')
}

function getSvgWidth(wrapper: VueWrapper) {
    return numberAttr(wrapper, '.arrow-wrapper svg', 'width')
}

function getSvgRotatedBbox(wrapper: VueWrapper, angleDeg: number) {
    const svgW = numberAttr(wrapper, '.arrow-wrapper svg', 'width')
    const svgH = numberAttr(wrapper, '.arrow-wrapper svg', 'height')
    const rad = (angleDeg * Math.PI) / 180
    const cosA = Math.abs(Math.cos(rad))
    const sinA = Math.abs(Math.sin(rad))
    return {
        halfW: (svgW / 2) * cosA + (svgH / 2) * sinA,
        halfH: (svgW / 2) * sinA + (svgH / 2) * cosA,
    }
}

function expectSvgWithinImage(wrapper: VueWrapper, {imageLeft = 0, imageTop = 0, imageWidth, imageHeight, angleDeg}: {
    imageLeft?: number
    imageTop?: number
    imageWidth: number
    imageHeight: number
    angleDeg: number
}) {
    const pos = getPosition(wrapper)
    const {halfW, halfH} = getSvgRotatedBbox(wrapper, angleDeg)
    // After the restructure, the guide-arrow-container is sized to the SVG (distance-info is
    // absolute-positioned and out of flow), so `pos` is the SVG's rotation centre.
    expect(pos.left - halfW).toBeGreaterThanOrEqual(imageLeft)
    expect(pos.left + halfW).toBeLessThanOrEqual(imageLeft + imageWidth)
    expect(pos.top - halfH).toBeGreaterThanOrEqual(imageTop)
    expect(pos.top + halfH).toBeLessThanOrEqual(imageTop + imageHeight)
}

describe('GuideArrow', () => {
    // The camelCase object `pushDirectionFrom` builds, not the wire's snake_case one.
    it('takes the direction as the event stream holds it', () => {
        expectTypeOf<ArrowProps['direction']>().toEqualTypeOf<ReturnType<typeof pushDirectionFrom> | null>()
        expectTypeOf<PushDirection>().not.toExtend<ArrowProps['direction']>()
    })

    describe('over another camera than the one it was solved on', () => {
        const GUIDE = {name: 'Neptune-C II', connected: true}
        const OTHER = {...IMAGE_PROPS, angleDeg: 37, distanceDeg: 26.1, directionHint: 'NE', fovDeg: 1.45, otherCamera: GUIDE} satisfies FlatProps

        it('drops the chevron, whose rotation belongs to the other camera', () => {
            const wrapper = mountArrow(OTHER)

            expect(wrapper.find('.arrow-wrapper').exists()).toBe(false)
            expect(wrapper.find('.other-camera').text()).toBe('Arrow on Neptune-C II')
        })

        it('still gives the distance and compass point, at the centre of the view', () => {
            const wrapper = mountArrow(OTHER)

            expect(wrapper.find('.distance').text()).toBe('26.1°')
            expect(wrapper.find('.hint').text()).toBe('NE')
            expect(getPosition(wrapper)).toEqual({left: 400, top: 300})
            expect(wrapper.find('.distance-info').classes()).toContain('centered')
        })

        it('still says on target, which needs no orientation', () => {
            const wrapper = mountArrow({...OTHER, distanceDeg: 0.05, isClose: true, directionHint: 'OK'})

            expect(wrapper.find('.on-target').exists()).toBe(true)
        })

        it('names no camera that has disconnected, and still drops its chevron', () => {
            const wrapper = mountArrow({...OTHER, otherCamera: {...GUIDE, connected: false}})

            expect(wrapper.find('.arrow-wrapper').exists()).toBe(false)
            expect(wrapper.find('.other-camera').exists()).toBe(false)
            expect(wrapper.find('.distance').text()).toBe('26.1°')
            expect(wrapper.find('.hint').text()).toBe('NE')
            expect(wrapper.find('.distance-info').classes()).toContain('centered')
        })

        it('draws the chevron as before when it belongs to this view', () => {
            const wrapper = mountArrow({...OTHER, otherCamera: null})

            expect(wrapper.find('.arrow-wrapper').exists()).toBe(true)
            expect(wrapper.find('.other-camera').exists()).toBe(false)
        })
    })

    describe('while Push-To is solving', () => {
        const STAGE = {label: 'blind', attempt: 2, total: 3}

        it('turns a compass in place of the readout, whose numbers are the last solve\'s', () => {
            const wrapper = mountArrow({...IMAGE_PROPS, solving: true})

            expect(wrapper.find('.push-to-solving').attributes('role')).toBe('status')
            expect(wrapper.find('.needle').classes()).toContain('spinning')
            expect(wrapper.find('.distance').exists()).toBe(false)
            expect(wrapper.find('.hint').exists()).toBe(false)
        })

        it('keeps the chevrons pointing the last known way', () => {
            const wrapper = mountArrow({...IMAGE_PROPS, angleDeg: 135, solving: true})

            expect(wrapper.find('.arrow-wrapper').attributes('style')).toContain('rotate(135deg)')
            expect(wrapper.findAll('.arrow-wrapper path')).toHaveLength(5)
        })

        it('puts the compass where the readout sits', () => {
            const props = {...IMAGE_PROPS, angleDeg: 60, distanceDeg: 3, fovDeg: 1}
            const readout = mountArrow(props).find('.distance-info')
            const compass = mountArrow({...props, solving: true}).find('.distance-info')

            expect(compass.classes()).toContain('push-to-solving')
            expect(compass.attributes('style')).toBe(readout.attributes('style'))
        })

        it('counts the steps of a multi-strategy solve, and only of one', () => {
            expect(mountArrow({...IMAGE_PROPS, solving: true, solveStage: STAGE}).find('.step').text()).toBe('step 2/3')
            expect(mountArrow({...IMAGE_PROPS, solving: true, solveStage: {...STAGE, attempt: 1, total: 1}})
                .find('.step').exists()).toBe(false)
            expect(mountArrow({...IMAGE_PROPS, solving: true}).find('.step').exists()).toBe(false)
        })

        it('centres the compass over another camera, as it centres the readout', () => {
            const wrapper = mountArrow({
                ...IMAGE_PROPS, fovDeg: 1.45, solving: true, otherCamera: {name: 'Neptune-C II', connected: true},
            })

            expect(wrapper.find('.arrow-wrapper').exists()).toBe(false)
            expect(wrapper.find('.push-to-solving').classes()).toContain('centered')
            expect(getPosition(wrapper)).toEqual({left: 400, top: 300})
        })

        it('leaves On Target as it is', () => {
            const wrapper = mountArrow({...IMAGE_PROPS, distanceDeg: 0.05, isClose: true, directionHint: 'OK', solving: true})

            expect(wrapper.find('.on-target').exists()).toBe(true)
            expect(wrapper.find('.push-to-solving').exists()).toBe(false)
        })

        // `aimAt` clears the direction, so a new target's first solve, up to minutes on the
        // blind rung, had nothing on screen at all.
        it('before a target\'s first solve, shows only the compass, at the centre of the view', () => {
            const wrapper = mount(GuideArrow, {
                props: {
                    direction: null,
                    frame: {width: 800, height: 600},
                    bounds: {left: 0, top: 0, width: 800, height: 600},
                    solving: true,
                    solveStage: STAGE,
                },
            })

            expect(wrapper.find('.push-to-solving').exists()).toBe(true)
            expect(wrapper.find('.step').text()).toBe('step 2/3')
            expect(wrapper.find('.arrow-wrapper').exists()).toBe(false)
            expect(wrapper.find('.on-target').exists()).toBe(false)
            expect(wrapper.find('.distance-info').exists()).toBe(false)
            expect(getPosition(wrapper)).toEqual({left: 400, top: 300})
        })
    })

    describe('rendering', () => {
        it('renders arrow wrapper when not on target', () => {
            const wrapper = mountArrow({angleDeg: 45, directionHint: 'NE'})

            expect(wrapper.find('.arrow-wrapper').exists()).toBe(true)
            expect(wrapper.find('.on-target').exists()).toBe(false)
        })

        it('renders on-target indicator when distance is very small', () => {
            const wrapper = mountArrow({
                distanceDeg: 0.05,
                isClose: true,
                directionHint: 'OK',
            })

            expect(wrapper.find('.on-target').exists()).toBe(true)
            expect(wrapper.find('.arrow-wrapper').exists()).toBe(false)
        })

        // 'OK' is the wire's on-target hint (Pro's `DirectionHint::OnTarget`); the branch once
        // compared 'OnTarget', never sent, so only the distance below 0.1° ever carried it.
        it('renders on-target indicator when directionHint is OK', () => {
            const wrapper = mountArrow({
                distanceDeg: 0.1,
                isClose: true,
                directionHint: 'OK',
            })

            expect(wrapper.find('.on-target').exists()).toBe(true)
            expect(wrapper.find('.on-target-label').text()).toBe('On Target')
        })

        it('renders distance info when not on target', () => {
            const wrapper = mountArrow({
                angleDeg: 90,
                distanceDeg: 5.5,
                directionHint: 'E',
            })

            expect(wrapper.find('.distance-info').exists()).toBe(true)
            expect(wrapper.find('.distance').text()).toBe('5.5°')
            expect(wrapper.find('.hint').text()).toBe('E')
        })

        it('does not render distance info when on target', () => {
            const wrapper = mountArrow({
                distanceDeg: 0.05,
                isClose: true,
                directionHint: 'OK',
            })

            expect(wrapper.find('.distance-info').exists()).toBe(false)
        })
    })

    describe('rotation', () => {
        const rotationCases = [
            {angle: 0, direction: 'N'},
            {angle: 90, direction: 'E'},
            {angle: 180, direction: 'S'},
            {angle: 270, direction: 'W'},
            {angle: 135, direction: 'SE'},
        ] as const

        rotationCases.forEach(({angle, direction}) => {
            it(`applies correct rotation for ${direction} (${angle} degrees)`, () => {
                const wrapper = mountArrow({angleDeg: angle, directionHint: direction})

                const arrowWrapper = wrapper.find('.arrow-wrapper')
                expect(arrowWrapper.attributes('style')).toContain(`rotate(${angle}deg)`)
            })
        })
    })

    describe('distance formatting', () => {
        const distanceCases = [
            {distance: 5.5, expected: '5.5°', description: 'degrees when >= 1 degree'},
            {distance: 0.5, expected: "30.0'", description: 'arcminutes when < 1 degree', isClose: true},
            {distance: 0.25, expected: "15.0'", description: 'very small distance in arcminutes', isClose: true},
        ]

        distanceCases.forEach(({distance, expected, description, isClose = false}) => {
            it(`formats distance in ${description}`, () => {
                const wrapper = mountArrow({distanceDeg: distance, isClose})
                expect(wrapper.find('.distance').text()).toBe(expected)
            })
        })
    })

    describe('scaling based on distance', () => {
        const scalingImageProps = {imageWidth: 600, imageHeight: 400}

        it('renders smaller arrow for close distances', () => {
            const wrapper = mountArrow({distanceDeg: 1, isClose: true, ...scalingImageProps})
            expect(getSvgWidth(wrapper)).toBeLessThan(50)
        })

        it('renders larger arrow for far distances', () => {
            const wrapper = mountArrow({distanceDeg: 30, ...scalingImageProps})
            expect(getSvgWidth(wrapper)).toBeGreaterThan(60)
        })

        it('caps arrow size at max distance', () => {
            const wrapper30 = mountArrow({distanceDeg: 30, ...scalingImageProps})
            const wrapper60 = mountArrow({distanceDeg: 60, ...scalingImageProps})

            expect(wrapper30.find('.arrow-wrapper svg').attributes('width')).toBe(
                wrapper60.find('.arrow-wrapper svg').attributes('width')
            )
        })

        it('scales with image size - larger image means larger arrow', () => {
            const wrapperSmall = mountArrow({distanceDeg: 15, imageWidth: 300, imageHeight: 200})
            const wrapperLarge = mountArrow({distanceDeg: 15, imageWidth: 900, imageHeight: 600})

            expect(getSvgWidth(wrapperLarge)).toBeGreaterThan(getSvgWidth(wrapperSmall))
        })

        it('respects minimum arrow size', () => {
            const wrapper = mountArrow({
                distanceDeg: 1,
                isClose: true,
                imageWidth: 100,
                imageHeight: 100,
            })
            // MIN_BASE_SIZE is 60, MIN_SCALE is 0.3 => 60 * 0.3 = 18
            expect(getSvgWidth(wrapper)).toBeGreaterThanOrEqual(18)
        })

        it('respects maximum arrow size', () => {
            const wrapper = mountArrow({
                distanceDeg: 30,
                imageWidth: 2000,
                imageHeight: 2000,
            })
            // MAX_BASE_SIZE is 240, MAX_SCALE is 1.6 => 384
            expect(getSvgWidth(wrapper)).toBeLessThanOrEqual(384)
        })
    })

    describe('scaling by on-screen edge proximity', () => {
        const proximityImageProps = {imageWidth: 600, imageHeight: 400}

        it('grows as the target drifts toward the screen edge at fixed FOV', () => {
            const near = mountArrow({
                distanceDeg: 0.3,
                fovDeg: 4,
                isClose: true,
                ...proximityImageProps,
            })
            const far = mountArrow({
                distanceDeg: 1.5,
                fovDeg: 4,
                ...proximityImageProps,
            })
            expect(getSvgWidth(far)).toBeGreaterThan(getSvgWidth(near))
        })

        it('on-screen arrow at the threshold is no larger than off-screen arrow', () => {
            const atEdge = mountArrow({
                distanceDeg: 1.6, // exactly OFF_SCREEN_THRESHOLD (0.4) * fovDeg
                fovDeg: 4,
                ...proximityImageProps,
            })
            const offScreen = mountArrow({
                distanceDeg: 5,
                fovDeg: 4,
                ...proximityImageProps,
            })
            expect(getSvgWidth(offScreen)).toBeGreaterThanOrEqual(getSvgWidth(atEdge))
        })

        it('falls back to distance-based scaling when fovDeg is unknown', () => {
            const small = mountArrow({distanceDeg: 1, isClose: true, ...proximityImageProps})
            const large = mountArrow({distanceDeg: 30, ...proximityImageProps})
            expect(getSvgWidth(small)).toBeLessThan(getSvgWidth(large))
        })

        it('does not grow with distance when the target stays near centre on a wide FOV', () => {
            const a = mountArrow({distanceDeg: 0.3, fovDeg: 20, isClose: true, ...proximityImageProps})
            const b = mountArrow({distanceDeg: 0.6, fovDeg: 40, isClose: true, ...proximityImageProps})
            // Both targets sit at ~1.5% of the screen — sizes should match even though
            // absolute distance doubles, because scaling is screen-relative.
            expect(getSvgWidth(a)).toBeCloseTo(getSvgWidth(b), 5)
        })

        it('squashes near-centre more aggressively than a linear mapping would', () => {
            // edgeProximity raw ≈ 0.25 (quarter of the on-screen band)
            const nearCentre = mountArrow({
                distanceDeg: 0.4,
                fovDeg: 4,
                isClose: true,
                ...proximityImageProps,
            })
            // edgeProximity raw = 1.0 (at the on/off-screen threshold)
            const nearEdge = mountArrow({distanceDeg: 1.6, fovDeg: 4, ...proximityImageProps})

            // Old linear mapping produced scales 0.475 → 1.000 (ratio ≈ 2.1×). With the
            // quadratic ease-in the ratio is strictly greater, codifying the "more contrast"
            // outcome so a regression to linear is caught.
            const ratio = getSvgWidth(nearEdge) / getSvgWidth(nearCentre)
            expect(ratio).toBeGreaterThan(3)
        })
    })

    describe('chevron rendering', () => {
        function mountChevronArrow() {
            return mountArrow({distanceDeg: 15})
        }

        it('renders five chevron paths', () => {
            const paths = mountChevronArrow().findAll('.arrow-wrapper svg g path')
            expect(paths.length).toBe(5)
        })

        it('renders chevrons with gradient stroke', () => {
            const paths = mountChevronArrow().findAll('.arrow-wrapper svg g path')
            paths.forEach((path) => {
                expect(path.attributes('stroke')).toMatch(/^url\(#chevronGradient-[\w\d]+\)$/)
            })
        })

        it('first chevron starts from center (y=0 offset)', () => {
            const paths = mountChevronArrow().findAll('.arrow-wrapper svg g path')
            const d = paths[0].attributes('d')
            expect(d).toMatch(/^M-?\d+(\.\d+)? -?0/)
        })

        it('chevrons progress upward from center', () => {
            const paths = mountChevronArrow().findAll('.arrow-wrapper svg g path')
            const yOffsets = paths.map((path) => {
                const d = path.attributes('d') ?? ''
                const match = d.match(/^M-?\d+(?:\.\d+)? (-?\d+(?:\.\d+)?)/)
                return match ? parseFloat(match[1]) : NaN
            })

            for (let i = 1; i < yOffsets.length; i++) {
                expect(yOffsets[i]).toBeLessThan(yOffsets[i - 1])
            }
        })
    })

    describe('direction hints', () => {
        const hints = DIRECTION_HINT_VALUES.filter((hint) => hint !== 'OK')

        hints.forEach((hint) => {
            it(`displays ${hint} direction hint`, () => {
                const wrapper = mountArrow({directionHint: hint})
                expect(wrapper.find('.hint').text()).toBe(hint)
            })
        })
    })

    describe('screen size display', () => {
        const screenSizeCases = [
            {fovDeg: 0, distanceDeg: 5, shouldExist: false, description: 'does not show when fovDeg is 0'},
            {fovDeg: 2, distanceDeg: 0.1, shouldExist: false, description: 'does not show when distance is very small'},
            {
                fovDeg: 2,
                distanceDeg: 0.5,
                shouldExist: true,
                expected: '25% screen',
                description: 'shows percentage when less than 1 FOV'
            },
            {
                fovDeg: 2,
                distanceDeg: 6,
                shouldExist: true,
                expected: '3.0× screen',
                description: 'shows multiplier when more than 1 FOV'
            },
            {
                fovDeg: 2,
                distanceDeg: 30,
                shouldExist: true,
                expected: '15× screen',
                description: 'shows rounded screen size for large values'
            },
        ]

        // 2026-10-12, the guide's 2539x1440 stream, NGC 5866 22.22° off at 268.2°: in heights
        // this read 15× screen, across the frame it is 8.7 of them
        it('counts screens across the frame for a sideways push', () => {
            const wrapper = mountArrow({
                angleDeg: 268.2, distanceDeg: 22.22, directionHint: 'W', fovDeg: 1.45,
                imageWidth: 2539, imageHeight: 1440,
            })
            expect(wrapper.find('.screen-size').text()).toBe('8.7× screen')
        })

        it('counts screens in heights for a vertical push on the same frame', () => {
            const wrapper = mountArrow({distanceDeg: 22.22, fovDeg: 1.45, imageWidth: 2539, imageHeight: 1440})
            expect(wrapper.find('.screen-size').text()).toBe('15× screen')
        })

        describe('the same on every page (2026-10-24)', () => {
            // The guide's 2712x1538 frame, first push of the night at 60.1°, 10° off: `/` read
            // 3.4 screens, the round eyepiece 6.9 — it counted its circle, one height every way.
            const GUIDE_FRAME = {width: 2712, height: 1538}
            const PUSH = {angleDeg: 60.14, distanceDeg: 10, directionHint: 'NW', fovDeg: 1.45, frame: GUIDE_FRAME} satisfies FlatProps
            const label = (props: FlatProps) => mountArrow({...PUSH, ...props}).find('.screen-size').text()

            it('counts frames on the circular eyepiece as `/` does', () => {
                const liveView = label({imageWidth: 844, imageHeight: 479})
                const eyepiece = label({imageWidth: 800, imageHeight: 800, isCircular: true})

                expect(liveView).toBe('3.4× screen')
                expect(eyepiece).toBe(liveView)
            })

            it('measures the frame, not the box it is drawn in', () => {
                // `/` before its first frame: the phone's portrait container
                expect(label({imageWidth: 390, imageHeight: 844})).toBe('3.4× screen')
            })

            it('still pins the arrow at the circle when the target is in the frame but off it', () => {
                // Sideways 0.6° on a 1.45° frame: 23 % of a frame across, past 0.4 of the circle
                const sideways = {...PUSH, angleDeg: 90, distanceDeg: 0.6, directionHint: 'W', imageWidth: 800, imageHeight: 800} satisfies FlatProps
                const circular = mountArrow({...sideways, isCircular: true})
                const rectangular = mountArrow(sideways)

                expect(circular.find('.screen-size').text()).toBe('23% screen')
                expect(rectangular.find('.screen-size').text()).toBe('23% screen')
                expect(getPosition(circular).left).toBeGreaterThan(400)
                expect(getPosition(rectangular)).toEqual({left: 400, top: 400})
            })

            it('places against a portrait frame\'s narrower circle', () => {
                // The circle is the frame's width across, half its height: 0.3 heights is 0.6 of it
                const portrait = {
                    angleDeg: 0, distanceDeg: 0.3, fovDeg: 1, frame: {width: 1000, height: 2000},
                    imageWidth: 800, imageHeight: 800,
                } satisfies FlatProps

                expect(getPosition(mountArrow({...portrait, isCircular: true})).top).toBeLessThan(400)
                expect(getPosition(mountArrow(portrait))).toEqual({left: 400, top: 400})
            })
        })

        it('counts in heights when the angle belongs to another camera', () => {
            const wrapper = mountArrow({
                angleDeg: 268.2, distanceDeg: 22.22, fovDeg: 1.45, otherCamera: {name: 'Neptune-C II', connected: true},
                imageWidth: 2539, imageHeight: 1440,
            })
            expect(wrapper.find('.screen-size').text()).toBe('15× screen')
        })

        screenSizeCases.forEach(({fovDeg, distanceDeg, shouldExist, expected, description}) => {
            it(description, () => {
                const wrapper = mountArrow({fovDeg, distanceDeg, isClose: distanceDeg < 1})
                const screenSize = wrapper.find('.screen-size')

                expect(screenSize.exists()).toBe(shouldExist)
                if (shouldExist) {
                    expect(screenSize.text()).toBe(expected)
                }
            })
        })
    })

    describe('off-screen positioning', () => {
        const offScreenImageProps = {
            ...IMAGE_PROPS,
            imageLeft: 100,
            imageTop: 50,
        }

        it('positions at image center when target is on screen', () => {
            const wrapper = mountArrow({
                angleDeg: 45,
                distanceDeg: 0.5,
                directionHint: 'NE',
                ...offScreenImageProps,
                fovDeg: 2,
            })

            const pos = getPosition(wrapper)
            expect(pos.left).toBe(500) // 100 + 800/2
            expect(pos.top).toBe(350) // 50 + 600/2
        })

        it('positions near edge when target is off screen', () => {
            const wrapper = mountArrow({
                distanceDeg: 5,
                ...offScreenImageProps,
                fovDeg: 2,
            })

            const pos = getPosition(wrapper)
            expect(pos.top).not.toBe(350) // Not at center
        })

        it('uses larger arrow size when off screen', () => {
            const baseProps = {imageWidth: 600, imageHeight: 400, fovDeg: 2}
            const wrapperOnScreen = mountArrow({distanceDeg: 0.5, ...baseProps})
            const wrapperOffScreen = mountArrow({distanceDeg: 5, ...baseProps})

            expect(getSvgWidth(wrapperOffScreen)).toBeGreaterThan(getSvgWidth(wrapperOnScreen))
        })
    })

    describe('chevron positioning calculations', () => {
        describe('on-screen positioning (centered)', () => {
            const centeredCases = [
                {
                    imageLeft: 0,
                    imageTop: 0,
                    imageWidth: 400,
                    imageHeight: 300,
                    expectedLeft: 200,
                    expectedTop: 150,
                    description: 'zero offset'
                },
                {
                    imageLeft: 50,
                    imageTop: 0,
                    imageWidth: 400,
                    imageHeight: 300,
                    expectedLeft: 250,
                    expectedTop: 150,
                    description: 'left offset'
                },
                {
                    imageLeft: 0,
                    imageTop: 100,
                    imageWidth: 400,
                    imageHeight: 300,
                    expectedLeft: 200,
                    expectedTop: 250,
                    description: 'top offset'
                },
                {
                    imageLeft: 75,
                    imageTop: 50,
                    imageWidth: 600,
                    imageHeight: 400,
                    expectedLeft: 375,
                    expectedTop: 250,
                    description: 'both offsets'
                },
            ]

            centeredCases.forEach(({
                                       imageLeft,
                                       imageTop,
                                       imageWidth,
                                       imageHeight,
                                       expectedLeft,
                                       expectedTop,
                                       description
                                   }) => {
                it(`centers on image with ${description}`, () => {
                    const wrapper = mountArrow({
                        distanceDeg: 0.3,
                        fovDeg: 2,
                        imageLeft,
                        imageTop,
                        imageWidth,
                        imageHeight,
                    })

                    const pos = getPosition(wrapper)
                    expect(pos.left).toBe(expectedLeft)
                    expect(pos.top).toBe(expectedTop)
                })
            })
        })

        describe('off-screen edge positioning', () => {
            const offScreenProps = {...IMAGE_PROPS, fovDeg: 2, distanceDeg: 5}

            const edgeCases = [
                {
                    angleDeg: 0,
                    directionHint: 'N',
                    leftRelation: 'equal',
                    topRelation: 'lessThan',
                    description: 'top edge for north'
                },
                {
                    angleDeg: 180,
                    directionHint: 'S',
                    leftRelation: 'equal',
                    topRelation: 'greaterThan',
                    description: 'bottom edge for south'
                },
                {
                    angleDeg: 90,
                    directionHint: 'E',
                    leftRelation: 'greaterThan',
                    topRelation: 'equal',
                    description: 'right edge for east'
                },
                {
                    angleDeg: 270,
                    directionHint: 'W',
                    leftRelation: 'lessThan',
                    topRelation: 'equal',
                    description: 'left edge for west'
                },
            ] as const

            edgeCases.forEach(({angleDeg, directionHint, leftRelation, topRelation, description}) => {
                it(`positions near ${description} (${angleDeg}°) when off-screen`, () => {
                    const wrapper = mountArrow({...offScreenProps, angleDeg, directionHint})
                    const pos = getPosition(wrapper)
                    const centerX = 400, centerY = 300

                    if (leftRelation === 'equal') expect(pos.left).toBe(centerX)
                    else if (leftRelation === 'greaterThan') expect(pos.left).toBeGreaterThan(centerX)
                    else expect(pos.left).toBeLessThan(centerX)

                    if (topRelation === 'equal') expect(pos.top).toBe(centerY)
                    else if (topRelation === 'greaterThan') expect(pos.top).toBeGreaterThan(centerY)
                    else expect(pos.top).toBeLessThan(centerY)
                })
            })

            it('positions near top-right corner for northeast (45°) when off-screen', () => {
                const wrapper = mountArrow({...offScreenProps, angleDeg: 45, directionHint: 'NE'})
                const pos = getPosition(wrapper)

                expect(pos.left).toBeGreaterThan(400)
                expect(pos.top).toBeLessThan(300)
            })

            it('positions near bottom-left corner for southwest (225°) when off-screen', () => {
                const wrapper = mountArrow({...offScreenProps, angleDeg: 225, directionHint: 'SW'})
                const pos = getPosition(wrapper)

                expect(pos.left).toBeLessThan(400)
                expect(pos.top).toBeGreaterThan(300)
            })
        })

        describe('edge positioning with image offset', () => {
            const offsetProps = {...IMAGE_PROPS, imageLeft: 100, imageTop: 50, fovDeg: 2, distanceDeg: 5}

            it('respects image offset when positioning near top edge', () => {
                const wrapper = mountArrow({...offsetProps, angleDeg: 0, directionHint: 'N'})
                const pos = getPosition(wrapper)

                expect(pos.left).toBe(500) // 100 + 800/2
                expect(pos.top).toBeGreaterThan(50)
                expect(pos.top).toBeLessThan(350) // Less than center
            })

            it('respects image offset when positioning near right edge', () => {
                const wrapper = mountArrow({...offsetProps, angleDeg: 90, directionHint: 'E'})
                const pos = getPosition(wrapper)

                expect(pos.left).toBeLessThan(900) // Less than image right
                expect(pos.left).toBeGreaterThan(500) // Greater than center
                expect(pos.top).toBe(350) // 50 + 600/2
            })
        })

        describe('positioning stays within image bounds', () => {
            it('keeps chevron within image horizontal bounds', () => {
                const wrapper = mountArrow({
                    angleDeg: 90,
                    distanceDeg: 10,
                    directionHint: 'E',
                    imageLeft: 50,
                    imageTop: 0,
                    imageWidth: 400,
                    imageHeight: 300,
                    fovDeg: 2,
                })

                const pos = getPosition(wrapper)
                expect(pos.left).toBeLessThanOrEqual(450) // 50 + 400
                expect(pos.left).toBeGreaterThanOrEqual(50)
            })

            it('keeps chevron within image vertical bounds', () => {
                const wrapper = mountArrow({
                    angleDeg: 180,
                    distanceDeg: 10,
                    directionHint: 'S',
                    imageLeft: 0,
                    imageTop: 100,
                    imageWidth: 400,
                    imageHeight: 300,
                    fovDeg: 2,
                })

                const pos = getPosition(wrapper)
                expect(pos.top).toBeLessThanOrEqual(400) // 100 + 300
                expect(pos.top).toBeGreaterThanOrEqual(100)
            })

            // Rotated SVG bounds: axis-aligned bounds on `pos` alone are insufficient because
            // for diagonal angles (e.g. 135°) the rotated SVG extends further along both axes
            // than its unrotated half-dimensions. These checks caught the mobile overflow bug.
            const rotatedBoundsCases = [
                {angleDeg: 0, description: 'cardinal north'},
                {angleDeg: 45, description: 'diagonal NE'},
                {angleDeg: 90, description: 'cardinal east'},
                {angleDeg: 135, description: 'diagonal SE'},
                {angleDeg: 180, description: 'cardinal south'},
                {angleDeg: 225, description: 'diagonal SW'},
                {angleDeg: 270, description: 'cardinal west'},
                {angleDeg: 315, description: 'diagonal NW'},
                {angleDeg: 60, description: 'non-orthogonal 60°'},
                {angleDeg: 200, description: 'non-orthogonal 200°'},
            ]

            describe('rotated SVG stays within image on desktop-sized viewport', () => {
                rotatedBoundsCases.forEach(({angleDeg, description}) => {
                    it(`keeps rotated SVG within image for ${description}`, () => {
                        const props = {
                            angleDeg,
                            distanceDeg: 20,
                            directionHint: 'N' as const,
                            imageLeft: 0,
                            imageTop: 0,
                            imageWidth: 800,
                            imageHeight: 600,
                            fovDeg: 2,
                        }
                        const wrapper = mountArrow(props)
                        expectSvgWithinImage(wrapper, props)
                    })
                })
            })

            describe('rotated SVG stays within image on mobile-sized viewport', () => {
                rotatedBoundsCases.forEach(({angleDeg, description}) => {
                    it(`keeps rotated SVG within image for ${description} at 370×210`, () => {
                        const props = {
                            angleDeg,
                            distanceDeg: 80,
                            directionHint: 'N' as const,
                            imageLeft: 5,
                            imageTop: 45,
                            imageWidth: 370,
                            imageHeight: 210,
                            fovDeg: 6,
                        }
                        const wrapper = mountArrow(props)
                        expectSvgWithinImage(wrapper, props)
                    })
                })
            })

            it('pins arrow to image centre when rotated SVG cannot fit with margin', () => {
                const props = {
                    angleDeg: 135,
                    distanceDeg: 50,
                    directionHint: 'N' as const,
                    imageLeft: 0,
                    imageTop: 0,
                    imageWidth: 200,
                    imageHeight: 150,
                    fovDeg: 2,
                }
                const wrapper = mountArrow(props)
                const pos = getPosition(wrapper)
                expect(pos.left).toBe(100)
                expect(pos.top).toBe(75)
            })
        })

        describe('circular bounds positioning', () => {
            const circularProps = {
                distanceDeg: 20,
                directionHint: 'N' as const,
                imageLeft: 0,
                imageTop: 0,
                imageWidth: 600,
                imageHeight: 600,
                fovDeg: 2,
                isCircular: true,
            }

            it('keeps arrow radially within the circle for north', () => {
                const wrapper = mountArrow({...circularProps, angleDeg: 0})
                const pos = getPosition(wrapper)
                const {halfH} = getSvgRotatedBbox(wrapper, 0)
                // Center is 300, 300. Radius is 300.
                // Arrow points north, so it should be at y < 300, exactly at x = 300
                expect(pos.left).toBe(300)
                expect(pos.top).toBeLessThan(300)
                // Tip should be inside circle: pos.top - halfH >= margin
                const radius = 300
                const margin = radius * 0.1
                expect(pos.top - halfH).toBeCloseTo(margin, 0)
            })

            it('keeps arrow radially within the circle for diagonal NE', () => {
                const wrapper = mountArrow({...circularProps, angleDeg: 45})
                const pos = getPosition(wrapper)
                const radius = 300
                
                // Calculate distance from center to arrow center
                const dx = pos.left - 300
                const dy = pos.top - 300
                const dist = Math.hypot(dx, dy)
                
                // Tip distance is dist + halfH (since it points radially)
                // wait, the bounding logic uses maxOffset = radius - halfH - margin
                // actually in GuideArrow: maxOffset = radius - svgHalfH - margin
                const svgH = numberAttr(wrapper, '.arrow-wrapper svg', 'height')
                const svgHalfH = svgH / 2
                const margin = radius * 0.1

                expect(dist).toBeCloseTo(radius - svgHalfH - margin, 0)
            })
        })

        describe('on-target positioning', () => {
            it('centers on-target indicator on image', () => {
                const wrapper = mountArrow({
                    distanceDeg: 0.05,
                    directionHint: 'OK',
                    imageLeft: 100,
                    imageTop: 50,
                    imageWidth: 600,
                    imageHeight: 400,
                    fovDeg: 2,
                })

                const pos = getPosition(wrapper)
                expect(pos.left).toBe(400) // 100 + 600/2
                expect(pos.top).toBe(250) // 50 + 400/2
            })

            it('stays centered even off screen if directionHint is OK', () => {
                const wrapper = mountArrow({
                    angleDeg: 45,
                    distanceDeg: 5,
                    directionHint: 'OK',
                    ...IMAGE_PROPS,
                    fovDeg: 2,
                })

                const pos = getPosition(wrapper)
                expect(pos.left).toBe(400)
                expect(pos.top).toBe(300)
            })
        })
    })
})
