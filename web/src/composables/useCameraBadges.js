import {ref, watch, onUnmounted, unref} from 'vue'

/**
 * The badges a camera row carries: its role, resolution, phase (with the warm-up
 * countdown), sensor mode and temperature. All read from the shared stores the panel is
 * given; nothing here talks to the server.
 *
 * @param {object} sources
 * @param {import('vue').Ref<object>} sources.cameraStatus - Status by camera name.
 * @param {import('vue').Ref<object>} sources.cameraPhase - Phase by camera name.
 * @param {import('vue').Ref<object>} sources.warmupEndsAt - Warm-up deadline (ms) by camera name.
 * @param {import('vue').Ref<object|null>} sources.settings - The server settings.
 */
export function useCameraBadges({cameraStatus, cameraPhase, warmupEndsAt, settings}) {
    function roleLabel(cam) {
        if (cam.role === 'guide') return 'Guide'
        if (cam.role === 'main') return 'Main'
        return null
    }

    function formatResolution(cam) {
        const {max_width: width, max_height: height} = cam?.info ?? {}
        // A camera listed without being opened (held elsewhere, or it failed to open) reports 0x0.
        return width && height ? `${width}x${height}` : '—'
    }

    function temperaturePill(cam) {
        if (!cam?.info?.has_cooler) return null
        const status = unref(cameraStatus)?.[cam.name]
        if (!status) return null
        return `${status.temperature_c.toFixed(1)}°C`
    }

    function phaseOf(cam) {
        return unref(cameraPhase)?.[cam?.name] || null
    }

    function isWarmingUp(cam) {
        return phaseOf(cam) === 'warming_up'
    }

    // Minute resolution is enough for a countdown of a few minutes; ticks only while a
    // warm-up has a known deadline.
    const now = ref(Date.now())
    let clock = null
    watch(
        () => Object.keys(warmupEndsAt.value ?? {}).length > 0,
        (counting) => {
            clearInterval(clock)
            clock = counting ? setInterval(() => (now.value = Date.now()), 15000) : null
        },
        {immediate: true}
    )
    onUnmounted(() => clearInterval(clock))

    /** "up to N min" until the server cuts the warm-up short, when it said when. */
    function warmupLeft(cam) {
        const endsAt = warmupEndsAt.value?.[cam?.name]
        if (!endsAt) return null
        return `up to ${Math.max(1, Math.ceil((endsAt - now.value) / 60000))} min`
    }

    function phaseLabel(cam) {
        const phase = phaseOf(cam)
        if (phase === 'precooling') return 'Precooling'
        if (phase === 'warming_up') {
            const left = warmupLeft(cam)
            return left ? `Warming up, ${left}` : 'Warming up'
        }
        // 'guiding' deliberately gets no pill: a connected guide camera is always guiding,
        // and the green role badge next to it already says so.
        return null
    }

    function sensorModePill(cam) {
        const modes = cam?.info?.sensor_modes
        if (!modes || modes.length === 0) return null
        const isGuide = cam?.role === 'guide'
        const override = isGuide
            ? settings.value?.guide_camera?.sensor_mode_override
            : settings.value?.sensor_mode_override
        // Mirrors the backend's `is_actively_stacking` gate in
        // `to_capture_config_with()` (session/state/settings.rs): Low Noise only
        // pays its frame-rate cost while frames are being integrated.
        // `stacking_type !== 'planetary'` stands for "DeepSky or Comet" since
        // `StackingType::supports_stacking()` is true for every variant today.
        // `wanderer_mode` need not appear: the UI always pairs `stacking: true`
        // with it (see `applyStackingMode` in CaptureControls.vue). Never true
        // for the guide camera: nothing it produces is stacked.
        const isActivelyStacking =
            !isGuide && settings.value?.stacking && settings.value?.stacking_type !== 'planetary'
        const desired = override ?? (isActivelyStacking ? 'low_readout_noise' : 'normal')
        const needle = desired === 'low_readout_noise' ? /lrn|low/i : /normal/i
        const match = modes.find((m) => needle.test(m.name))
        return (match ?? modes[0]).name
    }

    return {roleLabel, formatResolution, temperaturePill, isWarmingUp, phaseLabel, sensorModePill}
}
