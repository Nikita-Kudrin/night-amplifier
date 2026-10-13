import {nextTick, reactive} from 'vue'
import {useSectionMirror} from './useSectionMirror.js'

describe('useSectionMirror', () => {
    it('copies only its keys, and follows a deep change to them', async () => {
        const source = reactive({eyepiece: {intensity: 0.3}, bin: 1, other: 'x'})
        const local = useSectionMirror(() => source, ['eyepiece', 'bin'])

        expect(local).toEqual({eyepiece: {intensity: 0.3}, bin: 1})
        source.eyepiece.intensity = 0.5
        await nextTick()

        expect(local.eyepiece.intensity).toBe(0.5)
    })

    it('keeps its edits off the source', () => {
        const source = reactive({eyepiece: {intensity: 0.3}})
        const local = useSectionMirror(() => source, ['eyepiece'])

        local.eyepiece.intensity = 0.9

        expect(source.eyepiece.intensity).toBe(0.3)
    })

    // A typed edit waiting out its debounce was overwritten by another section's save.
    it('leaves a held key to the section, following the rest', async () => {
        const source = reactive({screen_width: 120, intensity: 0.3})
        const held = new Set<'screen_width' | 'intensity'>()
        const local = useSectionMirror(() => source, ['screen_width', 'intensity'], held)

        local.screen_width = 150
        held.add('screen_width')
        Object.assign(source, {screen_width: 120, intensity: 0.6})
        await nextTick()

        expect(local).toEqual({screen_width: 150, intensity: 0.6})
    })

    it('takes only the source\'s keys, and gives back just those', () => {
        const source = reactive({bin: 1, other: 'x'})
        expectTypeOf(useSectionMirror(() => source, ['bin'])).toEqualTypeOf<{bin: number}>()
        // @ts-expect-error a key the source does not have
        useSectionMirror(() => source, ['missing'])
    })
})
