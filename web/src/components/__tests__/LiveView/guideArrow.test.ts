import {nextTick} from 'vue'
import {setupMocks, mountLiveView} from './setup'
import type {Mocks} from './setup'
import {camera, clientDirection} from '../../../test/fixtures'
import type {PushToTarget} from '../../../composables/api.events'
import {NOT_SOLVING} from '../../../composables/events/pushToEvents'

describe('LiveView - Guide Arrow Integration', () => {
    let mocks: Mocks

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

        it('does not show guide arrow when target is set but no direction and nothing is solving', () => {
            const wrapper = mountLiveView({
                pushDirection: null,
                currentTarget: {
                    name: null,
                    designation: 'M31',
                    ra_degrees: 10.68,
                    dec_degrees: 41.27,
                },
            })

            expect(wrapper.findComponent({name: 'GuideArrow'}).exists()).toBe(false)
        })

        it('does not show guide arrow when direction exists but no target', () => {
            const wrapper = mountLiveView({
                pushDirection: clientDirection({
                    angleDeg: 45,
                    distanceDeg: 10,
                    directionHint: 'NE',
                    isClose: false,
                }),
                currentTarget: null,
            })

            expect(wrapper.findComponent({name: 'GuideArrow'}).exists()).toBe(false)
        })

        it('shows guide arrow when both target and direction are set', () => {
            const wrapper = mountLiveView({
                pushDirection: clientDirection({
                    angleDeg: 45,
                    distanceDeg: 10,
                    directionHint: 'NE',
                    isClose: false,
                }),
                currentTarget: {
                    name: null,
                    designation: 'M31',
                    ra_degrees: 10.68,
                    dec_degrees: 41.27,
                },
            })

            expect(wrapper.findComponent({name: 'GuideArrow'}).exists()).toBe(true)
        })
    })

    describe('while Push-To is solving', () => {
        const TARGET: PushToTarget = {name: null, designation: 'M31', ra_degrees: 10.68, dec_degrees: 41.27}
        const STAGE = {label: 'blind', attempt: 2, total: 3}
        const SOLVING = {...NOT_SOLVING, inProgress: true, targetName: 'M31', stage: STAGE}

        it('shows the compass alone while a new target\'s first solve runs', () => {
            const wrapper = mountLiveView({currentTarget: TARGET, eventStream: {plateSolving: SOLVING}})

            expect(wrapper.findComponent({name: 'GuideArrow'}).props('direction')).toBeNull()
            expect(wrapper.find('.push-to-solving').exists()).toBe(true)
        })

        it('hands the arrow the solve and its step', () => {
            const wrapper = mountLiveView({
                currentTarget: TARGET,
                pushDirection: clientDirection({angleDeg: 45, distanceDeg: 10, directionHint: 'NE'}),
                eventStream: {plateSolving: SOLVING},
            })

            const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
            expect(guideArrow.props('solving')).toBe(true)
            expect(guideArrow.props('solveStage')).toEqual(STAGE)
            expect(wrapper.find('.distance').exists()).toBe(false)
        })
    })

    describe('props passing', () => {
        // The screens count is measured on the frame, which the eyepiece pages share; the
        // canvas box is the container's until the first frame lands.
        it('hands the arrow the streamed frame\'s size', async () => {
            const wrapper = mountLiveView({
                pushDirection: clientDirection({angleDeg: 90, distanceDeg: 10, directionHint: 'W', isClose: false}),
                currentTarget: {name: null, designation: 'M31', ra_degrees: 10.68, dec_degrees: 41.27},
            })
            const frame = () => wrapper.findComponent({name: 'GuideArrow'}).props('frame')

            expect(frame()).toEqual({width: 0, height: 0})
            mocks.mockImageStream.dimensions.value = {width: 2712, height: 1538}
            await nextTick()
            expect(frame()).toEqual({width: 2712, height: 1538})
        })

        it('passes correct angle to GuideArrow', () => {
            const wrapper = mountLiveView({
                pushDirection: clientDirection({
                    angleDeg: 135,
                    distanceDeg: 15,
                    directionHint: 'SE',
                    isClose: false,
                }),
                currentTarget: {
                    name: null,
                    designation: 'M42',
                    ra_degrees: 83.82,
                    dec_degrees: -5.39,
                },
            })

            const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
            expect(guideArrow.props('direction').angleDeg).toBe(135)
        })

        it('passes correct distance to GuideArrow', () => {
            const wrapper = mountLiveView({
                pushDirection: clientDirection({
                    angleDeg: 90,
                    distanceDeg: 25.5,
                    directionHint: 'E',
                    isClose: false,
                }),
                currentTarget: {
                    name: null,
                    designation: 'M45',
                    ra_degrees: 56.87,
                    dec_degrees: 24.12,
                },
            })

            const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
            expect(guideArrow.props('direction').distanceDeg).toBe(25.5)
        })

        it('passes correct isClose flag to GuideArrow', () => {
            const wrapper = mountLiveView({
                pushDirection: clientDirection({
                    angleDeg: 0,
                    distanceDeg: 0.5,
                    directionHint: 'N',
                    isClose: true,
                }),
                currentTarget: {
                    name: null,
                    designation: 'M13',
                    ra_degrees: 250.42,
                    dec_degrees: 36.46,
                },
            })

            const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
            expect(guideArrow.props('direction').isClose).toBe(true)
        })

        it('passes correct directionHint to GuideArrow', () => {
            const wrapper = mountLiveView({
                pushDirection: clientDirection({
                    angleDeg: 225,
                    distanceDeg: 8,
                    directionHint: 'SW',
                    isClose: false,
                }),
                currentTarget: {
                    name: null,
                    designation: 'NGC 7000',
                    ra_degrees: 314.75,
                    dec_degrees: 44.33,
                },
            })

            const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
            expect(guideArrow.props('direction').directionHint).toBe('SW')
        })
    })

    describe('on target state', () => {
        it('shows guide arrow with the OK hint when very close', () => {
            const wrapper = mountLiveView({
                pushDirection: clientDirection({
                    angleDeg: 0,
                    distanceDeg: 0.05,
                    directionHint: 'OK',
                    isClose: true,
                }),
                currentTarget: {
                    name: null,
                    designation: 'M31',
                    ra_degrees: 10.68,
                    dec_degrees: 41.27,
                },
            })

            const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
            expect(guideArrow.exists()).toBe(true)
            expect(guideArrow.props('direction').directionHint).toBe('OK')
            expect(guideArrow.props('direction').distanceDeg).toBe(0.05)
        })
    })

    describe('custom coordinates target', () => {
        it('shows guide arrow for custom coordinate target (null designation)', () => {
            const wrapper = mountLiveView({
                pushDirection: clientDirection({
                    angleDeg: 180,
                    distanceDeg: 12,
                    directionHint: 'S',
                    isClose: false,
                }),
                currentTarget: {
                    name: null,
                    designation: null,
                    ra_degrees: 180.0,
                    dec_degrees: 45.0,
                },
            })

            const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
            expect(guideArrow.exists()).toBe(true)
            expect(guideArrow.props('direction').distanceDeg).toBe(12)
        })
    })

    describe('all direction hints', () => {
        const directions = [
            {hint: 'N', angle: 0},
            {hint: 'NE', angle: 45},
            {hint: 'E', angle: 90},
            {hint: 'SE', angle: 135},
            {hint: 'S', angle: 180},
            {hint: 'SW', angle: 225},
            {hint: 'W', angle: 270},
            {hint: 'NW', angle: 315},
        ] as const

        directions.forEach(({hint, angle}) => {
            it(`renders guide arrow for ${hint} direction`, () => {
                const wrapper = mountLiveView({
                    pushDirection: clientDirection({
                        angleDeg: angle,
                        distanceDeg: 10,
                        directionHint: hint,
                        isClose: false,
                    }),
                    currentTarget: {
                        name: null,
                        designation: 'M31',
                        ra_degrees: 10.68,
                        dec_degrees: 41.27,
                    },
                })

                const guideArrow = wrapper.findComponent({name: 'GuideArrow'})
                expect(guideArrow.exists()).toBe(true)
                expect(guideArrow.props('direction').angleDeg).toBe(angle)
                expect(guideArrow.props('direction').directionHint).toBe(hint)
            })
        })
    })

    describe('camera the direction was solved on', () => {
        const TARGET: PushToTarget = {name: null, designation: 'NGC 40', ra_degrees: 3.25, dec_degrees: 72.52}
        const SOLVED_ON_GUIDE = clientDirection({
            angleDeg: 37, distanceDeg: 26.1, directionHint: 'NE', isClose: false, solvedCamera: 'Neptune-C II',
        })
        const CAMERAS = {
            hasGuideCamera: true,
            mainCamera: camera({name: 'Ares-C PRO'}),
            guideCamera: camera({name: 'Neptune-C II'}),
        }

        it('withholds the guide camera\'s chevron from the imaging camera\'s picture', () => {
            const wrapper = mountLiveView({...CAMERAS, pushDirection: SOLVED_ON_GUIDE, currentTarget: TARGET})

            expect(wrapper.findComponent({name: 'GuideArrow'}).props('otherCamera'))
                .toEqual({name: 'Neptune-C II', connected: true})
            expect(wrapper.find('.other-camera').text()).toBe('Arrow on Neptune-C II')
        })

        it('names no guide camera once it has disconnected', () => {
            const wrapper = mountLiveView({
                hasGuideCamera: false, mainCamera: CAMERAS.mainCamera, guideCamera: null,
                pushDirection: SOLVED_ON_GUIDE, currentTarget: TARGET,
            })

            expect(wrapper.findComponent({name: 'GuideArrow'}).props('otherCamera'))
                .toEqual({name: 'Neptune-C II', connected: false})
            expect(wrapper.find('.other-camera').exists()).toBe(false)
            expect(wrapper.find('.arrow-wrapper').exists()).toBe(false)
        })

        it('draws it over the guide camera\'s own picture', () => {
            const wrapper = mountLiveView({
                ...CAMERAS, viewedCamera: 'guide', pushDirection: SOLVED_ON_GUIDE, currentTarget: TARGET,
            })

            expect(wrapper.findComponent({name: 'GuideArrow'}).props('otherCamera')).toBeNull()
        })
    })
})
