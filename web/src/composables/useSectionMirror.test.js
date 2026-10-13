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
})
