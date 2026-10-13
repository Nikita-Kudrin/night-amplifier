import {nextTick, ref} from 'vue'
import {useDisconnectConfirm} from './useDisconnectConfirm.js'

function setup({warming = false, capturing = false} = {}) {
    const camera = {id: 'cam1', name: 'ZWO ASI533', connected: true}
    const cameras = ref([camera])
    const state = {warming, capturing}
    const disconnect = vi.fn(async () => {})
    const confirmation = useDisconnectConfirm({
        cameras,
        isWarmingUp: () => state.warming,
        capturesOn: () => state.capturing,
        disconnect,
    })
    return {camera, cameras, state, disconnect, ...confirmation}
}

describe('useDisconnectConfirm', () => {
    it('disconnects at once when nothing is lost', async () => {
        const {camera, request, disconnect, pending} = setup()

        await request(camera)

        expect(disconnect).toHaveBeenCalledWith(camera, {skipWarmup: false})
        expect(pending.value).toBeNull()
    })

    it('asks before stopping a capture, and stops it on confirm', async () => {
        const {camera, request, confirm, copy, disconnect} = setup({capturing: true})

        request(camera)
        expect(disconnect).not.toHaveBeenCalled()
        expect(copy.value.confirm).toBe('Stop and disconnect')
        expect(copy.value.body).toContain('ZWO ASI533')

        await confirm()
        expect(disconnect).toHaveBeenCalledWith(camera, {skipWarmup: false})
        expect(copy.value).toBeNull()
    })

    it('asks before cutting a warm-up short, and skips it on confirm', async () => {
        const {camera, request, confirm, copy, disconnect} = setup({warming: true, capturing: true})

        request(camera)
        expect(copy.value.title).toBe('Disconnect without warming up?')
        await confirm()

        expect(disconnect).toHaveBeenCalledWith(camera, {skipWarmup: true})
    })

    it('does nothing on cancel', async () => {
        const {camera, request, cancel, confirm, disconnect} = setup({capturing: true})

        request(camera)
        cancel()
        await confirm()

        expect(disconnect).not.toHaveBeenCalled()
    })

    it('drops the question once the camera has gone', async () => {
        const {camera, cameras, request, pending} = setup({warming: true})
        request(camera)

        cameras.value = []
        await nextTick()

        expect(pending.value).toBeNull()
    })
})
