import {mount} from '@vue/test-utils'
import GuideArrow from './GuideArrow.vue'

const DEFAULT_PROPS = {
    angleDeg: 0,
    distanceDeg: 10,
    isClose: false,
    directionHint: 'North',
}

const IMAGE_PROPS = {
    imageLeft: 0,
    imageTop: 0,
    imageWidth: 800,
    imageHeight: 600,
}

function mountArrow(propsOverrides = {}) {
    return mount(GuideArrow, {
        props: {...DEFAULT_PROPS, ...propsOverrides},
    })
}

function getPosition(wrapper) {
    const style = wrapper.find('.guide-arrow-container').attributes('style')
    const leftMatch = style.match(/left:\s*([\d.]+)px/)
    const topMatch = style.match(/top:\s*([\d.]+)px/)
    return {
        left: leftMatch ? parseFloat(leftMatch[1]) : null,
        top: topMatch ? parseFloat(topMatch[1]) : null,
    }
}

function getSvgWidth(wrapper) {
    const svg = wrapper.find('.arrow-wrapper svg')
    return parseFloat(svg.attributes('width'))
}

describe('GuideArrow', () => {
    describe('rendering', () => {
        it('renders arrow wrapper when not on target', () => {
            const wrapper = mountArrow({angleDeg: 45, directionHint: 'NorthEast'})

            expect(wrapper.find('.arrow-wrapper').exists()).toBe(true)
            expect(wrapper.find('.on-target').exists()).toBe(false)
        })

        it('renders on-target indicator when distance is very small', () => {
            const wrapper = mountArrow({
                distanceDeg: 0.05,
                isClose: true,
                directionHint: 'OnTarget',
            })

            expect(wrapper.find('.on-target').exists()).toBe(true)
            expect(wrapper.find('.arrow-wrapper').exists()).toBe(false)
        })

        it('renders on-target indicator when directionHint is OnTarget', () => {
            const wrapper = mountArrow({
                distanceDeg: 0.08,
                isClose: true,
                directionHint: 'OnTarget',
            })

            expect(wrapper.find('.on-target').exists()).toBe(true)
            expect(wrapper.find('.on-target-label').text()).toBe('On Target')
        })

        it('renders distance info when not on target', () => {
            const wrapper = mountArrow({
                angleDeg: 90,
                distanceDeg: 5.5,
                directionHint: 'East',
            })

            expect(wrapper.find('.distance-info').exists()).toBe(true)
            expect(wrapper.find('.distance').text()).toBe('5.5°')
            expect(wrapper.find('.hint').text()).toBe('East')
        })

        it('does not render distance info when on target', () => {
            const wrapper = mountArrow({
                distanceDeg: 0.05,
                isClose: true,
                directionHint: 'OnTarget',
            })

            expect(wrapper.find('.distance-info').exists()).toBe(false)
        })
    })

    describe('rotation', () => {
        const rotationCases = [
            {angle: 0, direction: 'North'},
            {angle: 90, direction: 'East'},
            {angle: 180, direction: 'South'},
            {angle: 270, direction: 'West'},
            {angle: 135, direction: 'SouthEast'},
        ]

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
            // MAX_BASE_SIZE is 160, MAX_SCALE is 1.0 => 160
            expect(getSvgWidth(wrapper)).toBeLessThanOrEqual(160)
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
                expect(path.attributes('stroke')).toBe('url(#chevronGradient)')
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
                const d = path.attributes('d')
                const match = d.match(/^M-?\d+(?:\.\d+)? (-?\d+(?:\.\d+)?)/)
                return match ? parseFloat(match[1]) : null
            })

            for (let i = 1; i < yOffsets.length; i++) {
                expect(yOffsets[i]).toBeLessThan(yOffsets[i - 1])
            }
        })
    })

    describe('direction hints', () => {
        const hints = ['North', 'NorthEast', 'East', 'SouthEast', 'South', 'SouthWest', 'West', 'NorthWest']

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
                directionHint: 'NorthEast',
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
                    directionHint: 'North',
                    leftRelation: 'equal',
                    topRelation: 'lessThan',
                    description: 'top edge for north'
                },
                {
                    angleDeg: 180,
                    directionHint: 'South',
                    leftRelation: 'equal',
                    topRelation: 'greaterThan',
                    description: 'bottom edge for south'
                },
                {
                    angleDeg: 90,
                    directionHint: 'East',
                    leftRelation: 'greaterThan',
                    topRelation: 'equal',
                    description: 'right edge for east'
                },
                {
                    angleDeg: 270,
                    directionHint: 'West',
                    leftRelation: 'lessThan',
                    topRelation: 'equal',
                    description: 'left edge for west'
                },
            ]

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
                const wrapper = mountArrow({...offScreenProps, angleDeg: 45, directionHint: 'NorthEast'})
                const pos = getPosition(wrapper)

                expect(pos.left).toBeGreaterThan(400)
                expect(pos.top).toBeLessThan(300)
            })

            it('positions near bottom-left corner for southwest (225°) when off-screen', () => {
                const wrapper = mountArrow({...offScreenProps, angleDeg: 225, directionHint: 'SouthWest'})
                const pos = getPosition(wrapper)

                expect(pos.left).toBeLessThan(400)
                expect(pos.top).toBeGreaterThan(300)
            })
        })

        describe('edge positioning with image offset', () => {
            const offsetProps = {...IMAGE_PROPS, imageLeft: 100, imageTop: 50, fovDeg: 2, distanceDeg: 5}

            it('respects image offset when positioning near top edge', () => {
                const wrapper = mountArrow({...offsetProps, angleDeg: 0, directionHint: 'North'})
                const pos = getPosition(wrapper)

                expect(pos.left).toBe(500) // 100 + 800/2
                expect(pos.top).toBeGreaterThan(50)
                expect(pos.top).toBeLessThan(350) // Less than center
            })

            it('respects image offset when positioning near right edge', () => {
                const wrapper = mountArrow({...offsetProps, angleDeg: 90, directionHint: 'East'})
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
                    directionHint: 'East',
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
                    directionHint: 'South',
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
        })

        describe('on-target positioning', () => {
            it('centers on-target indicator on image', () => {
                const wrapper = mountArrow({
                    distanceDeg: 0.05,
                    directionHint: 'OnTarget',
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

            it('stays centered even with high distance if directionHint is OnTarget', () => {
                const wrapper = mountArrow({
                    angleDeg: 45,
                    distanceDeg: 0.08,
                    directionHint: 'OnTarget',
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
