import {describe, it, expect, beforeEach, vi} from 'vitest'
import {ref} from 'vue'
import {flushPromises} from '@vue/test-utils'
import {setupMocks, mountLiveView, createMockFrameData} from './setup.js'

describe('LiveView - Guide Arrow Integration', () => {
    let mocks

    beforeEach(() => {
        vi.clearAllMocks()
        mocks = setupMocks()
    })

    describe('visibility', () => {
        it('does not show guide arrow when no target is set', () => {
            const wrapper = mountLiveView({
                pushDirection: null,
                currentTarget: null,
            })

            expect(wrapper.findComponent({name: 'GuideArrow'}).exists()).toBe(false)
        })

        it('does not show guide arrow when target is set but no direction', () => {
            const wrapper = mountLiveView({
                pushDirection: null,
                currentTarget: {
                    designation: 'M31',
                    raDegrees: 10.68,
                    decDegrees: 41.27,
                },
            })

            expect(wrapper.findComponent({name: 'GuideArrow'}).exists()).toBe(false)
        })

        it('does not show guide arrow when direction exists but no target', () => {
            const wrapper = mountLiveView({
                pushDirection: {
                    angleDeg: 45,
                    distanceDeg: 10,
                    directionHint: 'NorthEast',
                    isClose: false,
                },
                currentTarget: null,
            })

            expect(wrapper.findComponent({name: 'GuideArrow'}).exists()).toBe(false)
        })

        it('shows guide arrow when both target and direction are set', () => {
            const wrapper = mountLiveView({
                pushDirection: {
                    angleDeg: 45,
                    distanceDeg: 10,
                    directionHint: 'NorthEast',
                    isClose: false,
                },
                currentTarget: {
                    designation: 'M31',
                    raDegrees: 10.68,
                    decDegrees: 41.27,
                },
            })

            expect(wrapper.findComponent({name: 'GuideArrow'}).exists()).toBe(true)
        })
    })

    describe('props passing', () => {
        it('passes correct angle to GuideArrow', () => {
            const wrapper = mountLiveView({
                pushDirection: {
                    angleDeg: 135,
                    distanceDeg: 15,
                    directionHint: 'SouthEast',
                    isClose: false,
                },
                currentTarget: {
                    designation: 'M42',
                    raDegrees: 83.82,
                    decDegrees: -5.39,
                },
            })

            const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
            expect(guideArrow.props('angleDeg')).toBe(135)
        })

        it('passes correct distance to GuideArrow', () => {
            const wrapper = mountLiveView({
                pushDirection: {
                    angleDeg: 90,
                    distanceDeg: 25.5,
                    directionHint: 'East',
                    isClose: false,
                },
                currentTarget: {
                    designation: 'M45',
                    raDegrees: 56.87,
                    decDegrees: 24.12,
                },
            })

            const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
            expect(guideArrow.props('distanceDeg')).toBe(25.5)
        })

        it('passes correct isClose flag to GuideArrow', () => {
            const wrapper = mountLiveView({
                pushDirection: {
                    angleDeg: 0,
                    distanceDeg: 0.5,
                    directionHint: 'North',
                    isClose: true,
                },
                currentTarget: {
                    designation: 'M13',
                    raDegrees: 250.42,
                    decDegrees: 36.46,
                },
            })

            const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
            expect(guideArrow.props('isClose')).toBe(true)
        })

        it('passes correct directionHint to GuideArrow', () => {
            const wrapper = mountLiveView({
                pushDirection: {
                    angleDeg: 225,
                    distanceDeg: 8,
                    directionHint: 'SouthWest',
                    isClose: false,
                },
                currentTarget: {
                    designation: 'NGC 7000',
                    raDegrees: 314.75,
                    decDegrees: 44.33,
                },
            })

            const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
            expect(guideArrow.props('directionHint')).toBe('SouthWest')
        })
    })

    describe('on target state', () => {
        it('shows guide arrow with OnTarget hint when very close', () => {
            const wrapper = mountLiveView({
                pushDirection: {
                    angleDeg: 0,
                    distanceDeg: 0.05,
                    directionHint: 'OnTarget',
                    isClose: true,
                },
                currentTarget: {
                    designation: 'M31',
                    raDegrees: 10.68,
                    decDegrees: 41.27,
                },
            })

            const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
            expect(guideArrow.exists()).toBe(true)
            expect(guideArrow.props('directionHint')).toBe('OnTarget')
            expect(guideArrow.props('distanceDeg')).toBe(0.05)
        })
    })

    describe('custom coordinates target', () => {
        it('shows guide arrow for custom coordinate target (null designation)', () => {
            const wrapper = mountLiveView({
                pushDirection: {
                    angleDeg: 180,
                    distanceDeg: 12,
                    directionHint: 'South',
                    isClose: false,
                },
                currentTarget: {
                    designation: null,
                    raDegrees: 180.0,
                    decDegrees: 45.0,
                },
            })

            const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
            expect(guideArrow.exists()).toBe(true)
            expect(guideArrow.props('distanceDeg')).toBe(12)
        })
    })

    describe('all direction hints', () => {
        const directions = [
            {hint: 'North', angle: 0},
            {hint: 'NorthEast', angle: 45},
            {hint: 'East', angle: 90},
            {hint: 'SouthEast', angle: 135},
            {hint: 'South', angle: 180},
            {hint: 'SouthWest', angle: 225},
            {hint: 'West', angle: 270},
            {hint: 'NorthWest', angle: 315},
        ]

        directions.forEach(({hint, angle}) => {
            it(`renders guide arrow for ${hint} direction`, () => {
                const wrapper = mountLiveView({
                    pushDirection: {
                        angleDeg: angle,
                        distanceDeg: 10,
                        directionHint: hint,
                        isClose: false,
                    },
                    currentTarget: {
                        designation: 'M31',
                        raDegrees: 10.68,
                        decDegrees: 41.27,
                    },
                })

                const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
                expect(guideArrow.exists()).toBe(true)
                expect(guideArrow.props('angleDeg')).toBe(angle)
                expect(guideArrow.props('directionHint')).toBe(hint)
            })
        })
    })
})
