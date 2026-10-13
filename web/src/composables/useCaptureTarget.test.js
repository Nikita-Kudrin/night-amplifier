import {computed, defineComponent, h, nextTick, ref} from 'vue'
import {mountWithDefaults} from '../test/mount.js'
import {useCaptureTarget} from './useCaptureTarget.js'

function mountTarget({role = 'main', captureState = 'Idle', phases = {}, selectedCamera = 'main1', guide = {name: 'Guide Cam'}} = {}) {
    const roleRef = ref(role)
    const busy = ref(false)
    const state = ref(captureState)
    const cameraPhase = ref(phases)
    let target
    const Host = defineComponent({
        setup() {
            target = useCaptureTarget({role: roleRef, busy})
            return () => h('div')
        },
    })
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
    it.each([
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
})
