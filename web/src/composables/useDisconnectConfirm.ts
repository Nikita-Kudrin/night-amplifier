import {computed, ref, watch} from 'vue'
import type {Ref} from 'vue'
import type {Camera} from './api.types'

/** What a confirmation reads of a camera. */
type ConfirmedCamera = Pick<Camera, 'id' | 'name' | 'connected'>

/** The open question: the camera, and what disconnecting it now would cost. */
export interface DisconnectQuestion<C extends ConfirmedCamera = ConfirmedCamera> {
    camera: C
    kind: 'capture' | 'skip_warmup'
}

export interface DisconnectConfirmOptions<C extends ConfirmedCamera> {
    /** The camera list. */
    cameras: Readonly<Ref<readonly C[]>>
    isWarmingUp: (camera: C) => boolean
    /** A capture runs on it. */
    capturesOn: (camera: C) => boolean
    disconnect: (camera: C, options: {skipWarmup: boolean}) => Promise<unknown>
}

/**
 * A Disconnect the user must confirm first: one that stops a running capture, or one
 * that cuts a warm-up short. Disconnect is always available — the server ends any session
 * in bounded time — but those two cost something the user should agree to.
 *
 * The question is dropped once it no longer describes the camera — it finished warming
 * up and went, or its capture ended, while the dialog was open: confirming then answered
 * "not connected", or asked about a cost that no longer applied.
 */
export function useDisconnectConfirm<C extends ConfirmedCamera>(
    {cameras, isWarmingUp, capturesOn, disconnect}: DisconnectConfirmOptions<C>,
) {
    /** Null while no question is open. */
    const pending: Ref<DisconnectQuestion<C> | null> = ref(null)

    function request(camera: C) {
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

    function stillApplies(asked: DisconnectQuestion<C>) {
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
