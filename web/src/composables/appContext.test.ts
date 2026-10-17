import {mount} from '@vue/test-utils'
import {defineComponent, h, readonly, ref} from 'vue'
import {contextProvides, findCamera, isSimulator, useCapabilities, useSelectedCamera} from './appContext'
import type {AppContext} from './appContext'
import type {Capabilities} from './api.types'
import {DEFAULT_CAPABILITIES} from '../constants'
import {camera, proCapabilities} from '../test/fixtures'

/** Runs `use` inside a component given these provides (by name), and hands back what it returned. */
function withProvides<T>(provide: Partial<AppContext>, use: () => T): T {
    let result: T | undefined
    const Probe = defineComponent({
        setup() {
            result = use()
            return () => h('div')
        },
    })
    mount(Probe, {global: {provide: contextProvides(provide)}})
    return result as T
}

const PRO = {
    ...proCapabilities(),
    debug_logging: true,
}

const FLAGS = [
    'advancedRejection', 'rbfBackground', 'saturationBoost', 'denoise', 'aiDenoise',
    'advancedPlanetary', 'astapSolver', 'cometPro', 'debugLogging',
] as const

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
        const source = ref<Capabilities>(DEFAULT_CAPABILITIES)
        const caps = withProvides({capabilities: source}, useCapabilities)
        expect(caps.cometPro.value).toBe(false)

        source.value = PRO
        expect(caps.cometPro.value).toBe(true)
    })
})

describe('useSelectedCamera', () => {
    const cameras = [camera({id: 'a', name: 'A'}), camera({id: 'b', name: 'B'})]

    it('names the entry for the selected id and follows the selection', () => {
        const selected = ref<string | null>('a')
        const entry = withProvides({cameras: ref(cameras), selectedCamera: selected}, useSelectedCamera)
        expect(entry.value?.name).toBe('A')

        selected.value = 'b'
        expect(entry.value?.name).toBe('B')

        selected.value = 'gone'
        expect(entry.value).toBeNull()
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
