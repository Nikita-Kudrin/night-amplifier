import {computed, defineComponent, h, nextTick, ref} from 'vue'
import type {Ref} from 'vue'
import {mount} from '@vue/test-utils'
import {mountWithDefaults} from '../test/mount.js'
import {camera} from '../test/fixtures.js'
import {useCaptureTarget} from './useCaptureTarget.js'
import {contextProvides} from './appContext.js'
import type {CaptureState} from './api.events.js'
import type {Camera, CameraPhase, CameraRole} from './api.types.js'
import type {ByCameraName} from './cameraStore.js'

interface Setup {
    role?: CameraRole
    captureState?: CaptureState
    phases?: ByCameraName<CameraPhase>
    selectedCamera?: string | null
    guide?: Camera | null
}

const GUIDE = camera({name: 'Guide Cam'})

/** A component running `useCaptureTarget(options)`; `target` is what it returned, once mounted. */
function hostOf(options: Parameters<typeof useCaptureTarget>[0]) {
    const target = {} as ReturnType<typeof useCaptureTarget>
    const Host = defineComponent({
        setup() {
            Object.assign(target, useCaptureTarget(options))
            return () => h('div')
        },
    })
    return {Host, target}
}

function mountTarget({role = 'main', captureState = 'Idle', phases = {}, selectedCamera = 'main1', guide = GUIDE}: Setup = {}) {
    const roleRef = ref<CameraRole>(role)
    const busy = ref(false)
    const state = ref<CaptureState>(captureState)
    const cameraPhase = ref(phases)
    const {Host, target} = hostOf({role: roleRef, busy})
    mountWithDefaults(Host, {
        provide: {
            eventStream: {captureState: state},
            cameraPhase,
            selectedCamera: ref(selectedCamera),
            guideCamera: computed(() => guide),
        },
    })
    return {target, role: roleRef, busy, state, cameraPhase}
}

describe('useCaptureTarget', () => {
    it.each<[CaptureState, boolean, boolean]>([
        ['Idle', false, false],
        ['Starting', true, false],
        ['Capturing', true, false],
        ['Recovering', true, false],
        ['Stopping', false, true],
    ])('reads the imaging camera from the capture state (%s)', (captureState, capturing, stopping) => {
        const {target} = mountTarget({captureState})

        expect(target.isCapturing.value).toBe(capturing)
        expect(target.isStopping.value).toBe(stopping)
        expect(target.canStart.value).toBe(!capturing && !stopping)
    })

    it('reads the guide loop from its camera phase, not the capture session', async () => {
        const {target, cameraPhase} = mountTarget({role: 'guide', captureState: 'Capturing'})
        expect(target.isCapturing.value).toBe(false)
        expect(target.startLabel.value).toBe('Start guide')

        cameraPhase.value = {'Guide Cam': 'guiding'}
        await nextTick()

        expect(target.isCapturing.value).toBe(true)
        expect(target.stopLabel.value).toBe('Stop guide')
    })

    it('reads a guide stop from the request in flight', async () => {
        const {target, busy} = mountTarget({role: 'guide', phases: {'Guide Cam': 'guiding'}})
        expect(target.isStopping.value).toBe(false)

        busy.value = true
        await nextTick()

        expect(target.isStopping.value).toBe(true)
    })

    it('cannot start a camera that is not there', () => {
        expect(mountTarget({selectedCamera: null}).target.canStart.value).toBe(false)
        expect(mountTarget({role: 'guide', guide: null}).target.canStart.value).toBe(false)
    })

    it('addresses a camera role', () => {
        expectTypeOf(useCaptureTarget).parameter(0).toHaveProperty('role').toEqualTypeOf<Readonly<Ref<CameraRole>>>()
    })

    it('reads the imaging camera as idle without an event stream', () => {
        const {Host, target} = hostOf({role: ref<CameraRole>('main'), busy: ref(false)})
        mount(Host, {global: {provide: contextProvides({selectedCamera: ref('main1')})}})

        expect(target.isCapturing.value).toBe(false)
        expect(target.canStart.value).toBe(true)
    })
})
