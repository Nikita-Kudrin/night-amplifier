import {mount} from '@vue/test-utils'
import {defineComponent, h, readonly, ref} from 'vue'
import {findCamera, isSimulator, useCapabilities, useSelectedCamera} from './appContext.js'
import {DEFAULT_CAPABILITIES} from '../constants'

/** Runs `use` inside a component given these provides, and hands back what it returned. */
function withProvides(provide, use) {
    let result
    const Probe = defineComponent({
        setup() {
            result = use()
            return () => h('div')
        },
    })
    mount(Probe, {global: {provide}})
    return result
}

const PRO = {
    has_pro: true,
    deep_sky: {advanced_rejection: true, rbf_background: true, saturation_boost: true, denoise: true, ai_denoise: true},
    planetary: {advanced_stacking: true},
    push_to: {astap_solver: true},
    comet: {pro_stacking: true},
    debug_logging: true,
}

const FLAGS = [
    'advancedRejection', 'rbfBackground', 'saturationBoost', 'denoise', 'aiDenoise',
    'advancedPlanetary', 'astapSolver', 'cometPro', 'debugLogging',
]

describe('useCapabilities', () => {
    it('locks every Pro feature when nothing is provided', () => {
        const caps = withProvides({}, useCapabilities)
        expect(caps.capabilities.value).toBe(DEFAULT_CAPABILITIES)
        for (const flag of FLAGS) expect(caps[flag].value, flag).toBe(false)
    })

    it('reads a read-only ref the way App provides it', () => {
        const caps = withProvides({capabilities: readonly(ref(PRO))}, useCapabilities)
        for (const flag of FLAGS) expect(caps[flag].value, flag).toBe(true)
    })

    it('follows the answer when it arrives after mount', () => {
        const source = ref(DEFAULT_CAPABILITIES)
        const caps = withProvides({capabilities: source}, useCapabilities)
        expect(caps.cometPro.value).toBe(false)

        source.value = PRO
        expect(caps.cometPro.value).toBe(true)
    })

    it('treats a group an older server leaves out as locked', () => {
        const caps = withProvides({capabilities: ref({has_pro: true, deep_sky: {}})}, useCapabilities)
        expect(caps.cometPro.value).toBe(false)
        expect(caps.denoise.value).toBe(false)
    })
})

describe('useSelectedCamera', () => {
    const cameras = [{id: 'a', name: 'A'}, {id: 'b', name: 'B'}]

    it('names the entry for the selected id and follows the selection', () => {
        const selected = ref('a')
        const camera = withProvides({cameras: ref(cameras), selectedCamera: selected}, useSelectedCamera)
        expect(camera.value.name).toBe('A')

        selected.value = 'b'
        expect(camera.value.name).toBe('B')

        selected.value = 'gone'
        expect(camera.value).toBeNull()
    })

    it('is null without a camera list or a selection', () => {
        expect(withProvides({}, useSelectedCamera).value).toBeNull()
    })
})

describe('findCamera / isSimulator', () => {
    it('finds by id and tolerates a missing list', () => {
        expect(findCamera([{id: 'x'}], 'x')).toEqual({id: 'x'})
        expect(findCamera(null, 'x')).toBeNull()
    })

    it('recognises the simulator by provider', () => {
        expect(isSimulator({provider: 'Simulator'})).toBe(true)
        expect(isSimulator({provider: 'ZWO'})).toBe(false)
        expect(isSimulator(null)).toBe(false)
    })
})
