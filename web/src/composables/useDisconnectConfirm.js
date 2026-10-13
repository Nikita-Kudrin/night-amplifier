import {computed, ref, watch} from 'vue'

/**
 * A Disconnect the user must confirm first: one that stops a running capture, or one
 * that cuts a warm-up short. Disconnect is always available — the server ends any session
 * in bounded time — but those two cost something the user should agree to.
 *
 * The question is dropped once it no longer describes the camera — it finished warming
 * up and went, or its capture ended, while the dialog was open: confirming then answered
 * "not connected", or asked about a cost that no longer applied.
 *
 * @param {Object} options
 * @param {import('vue').Ref<Object[]>} options.cameras - The camera list
 * @param {(camera: Object) => boolean} options.isWarmingUp
 * @param {(camera: Object) => boolean} options.capturesOn - A capture runs on it
 * @param {(camera: Object, options: {skipWarmup: boolean}) => Promise} options.disconnect
 */
export function useDisconnectConfirm({cameras, isWarmingUp, capturesOn, disconnect}) {
    /** `{camera, kind: 'capture'|'skip_warmup'}`, or null while no question is open. */
    const pending = ref(null)

    function request(camera) {
        if (isWarmingUp(camera)) {
            pending.value = {camera, kind: 'skip_warmup'}
            return undefined
        }
        if (capturesOn(camera)) {
            pending.value = {camera, kind: 'capture'}
            return undefined
        }
        return disconnect(camera, {skipWarmup: false})
    }

    async function confirm() {
        const asked = pending.value
        pending.value = null
        if (!asked) return
        await disconnect(asked.camera, {skipWarmup: asked.kind === 'skip_warmup'})
    }

    function cancel() {
        pending.value = null
    }

    function stillApplies(asked) {
        const camera = cameras.value.find((c) => c.id === asked.camera.id && c.connected)
        if (!camera) return false
        return asked.kind === 'skip_warmup' ? isWarmingUp(camera) : capturesOn(camera)
    }

    watch(
        () => pending.value && stillApplies(pending.value),
        (applies) => {
            if (pending.value && !applies) pending.value = null
        }
    )

    const copy = computed(() => {
        const asked = pending.value
        if (!asked) return null
        const name = asked.camera.name
        if (asked.kind === 'capture') {
            return {
                title: 'Stop the capture and disconnect?',
                body: `The capture running on ${name} stops first and its stack is saved, as on Stop. The sub being exposed is discarded.`,
                confirm: 'Stop and disconnect',
            }
        }
        return {
            title: 'Disconnect without warming up?',
            body: `${name} is warming up slowly so its sensor is not shocked by a sudden temperature change. Disconnecting now switches the cooler off at once.`,
            confirm: 'Disconnect now',
        }
    })

    return {pending, copy, request, confirm, cancel}
}
