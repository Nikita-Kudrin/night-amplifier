<script setup>
/**
 * Settings → Advanced, AI part: the AI denoising switch and the compute unit it runs on,
 * from the server's one-time benchmark.
 *
 * Takes `save` rather than emitting: a compute choice must reach the server before the
 * report is refetched, or the report still describes the old unit. `guard` is the panel's
 * error handler, so a refused "Measure again" lands in the panel's one error banner.
 */
import {computed, reactive, ref, watch} from 'vue'
import {remeasureAiCompute} from '../composables/api.js'
import {useAiCompute} from '../composables/useAiCompute.js'
import {aiComputeOptions, aiComputeSummary, isReady, SYSTEM_DEPENDENCIES_URL, unusableRungs} from '../utils/aiCompute.js'
import {BaseToggle, BaseInfoIcon, BaseProLock} from './ui'
import {HELP_TEXTS} from '../constants'

const props = defineProps({
  denoise: {type: Object, required: true},
  aiDenoiseAvailable: {type: Boolean, default: false},
  focusMode: {type: Boolean, default: false},
  /** `(key, value) => Promise`: persists a settings group. */
  save: {type: Function, required: true},
  /** `(operation) => Promise`: runs `operation`, reporting a failure to the panel. */
  guard: {type: Function, required: true},
})

const HELP = HELP_TEXTS

const local = reactive({...props.denoise})

watch(
    () => props.denoise,
    (denoise) => Object.assign(local, denoise),
    {deep: true}
)

/**
 * The network is Pro, needs the Denoise switch, and Focus/Finder mode holds it off. The
 * mode never changes the switch itself: it stays the observer's for when the mode ends.
 */
const aiDenoiseLocked = computed(() => !props.aiDenoiseAvailable || !local.enabled || props.focusMode)
const aiDenoiseHeldOff = computed(() => {
  if (!props.aiDenoiseAvailable || !local.ai) return ''
  if (!local.enabled) return 'Held off while Denoise is off.'
  if (props.focusMode) return 'Held off by Focus/Finder mode.'
  return ''
})

/**
 * A hardware choice, not a picture control: it stays usable while the AI switch is off,
 * so the observer can pick before switching it on.
 */
const {report, refresh} = useAiCompute()
const optionList = computed(() => aiComputeOptions(report.value))
const hint = computed(() => aiComputeSummary(report.value))
const unusable = computed(() => unusableRungs(report.value))
const computeLocked = computed(() => !props.aiDenoiseAvailable || !isReady(report.value))

function applyAi(value) {
  return props.save('denoise', {...local, ai: value})
}

async function applyCompute(value) {
  await props.save('denoise', {...local, ai_compute: value})
  refresh()
}

/**
 * Measure again: after a new driver or runtime, or to retry a unit recorded as crashing.
 * The server refuses while a capture runs.
 */
const remeasuring = ref(false)

async function remeasure() {
  remeasuring.value = true
  try {
    await props.guard(() => remeasureAiCompute())
    await refresh()
  } finally {
    remeasuring.value = false
  }
}
</script>

<template>
  <div class="control-group">
    <BaseToggle
        v-model="local.ai"
        label="AI denoising"
        data-test="ai-denoise-toggle"
        :help="HELP.denoise_ai"
        :disabled="aiDenoiseLocked"
        @update:model-value="applyAi"
    >
      <template #label-extra>
        <BaseProLock v-if="!aiDenoiseAvailable" feature="AI Denoising"/>
      </template>
    </BaseToggle>
    <span v-if="aiDenoiseHeldOff" class="hint">{{ aiDenoiseHeldOff }}</span>
  </div>

  <div class="control-group" data-test="ai-compute">
    <div class="control-row">
      <label class="control-label" for="ai-compute-select" style="margin-bottom: 0; flex: 1">
        AI compute
        <BaseProLock v-if="!aiDenoiseAvailable" feature="AI Denoising"/>
        <BaseInfoIcon :message="HELP.ai_compute"/>
      </label>
      <select
          id="ai-compute-select"
          v-model="local.ai_compute"
          class="select ai-compute-select"
          data-test="ai-compute-select"
          :disabled="computeLocked"
          @change="applyCompute($event.target.value)"
      >
        <option
            v-for="opt in optionList"
            :key="opt.value"
            :value="opt.value"
            :disabled="opt.disabled"
            :title="opt.reason || undefined"
        >
          {{ opt.label }}
        </option>
      </select>
    </div>
    <span v-if="aiDenoiseAvailable && hint" class="hint" data-test="ai-compute-hint">{{ hint }}</span>
    <div v-if="aiDenoiseAvailable && !computeLocked" class="ai-compute-remeasure">
      <button
          type="button"
          class="btn btn-sm btn-secondary"
          data-test="ai-compute-remeasure"
          :disabled="remeasuring"
          @click="remeasure"
      >
        Measure again
      </button>
      <BaseInfoIcon :message="HELP.ai_compute_remeasure"/>
    </div>
    <ul v-if="aiDenoiseAvailable && unusable.length" class="hint ai-compute-reasons" data-test="ai-compute-reasons">
      <li v-for="rung in unusable" :key="rung.rung">
        <strong>{{ rung.label }}:</strong>
        {{ rung.device ? `${rung.device} — ` : '' }}{{ rung.reason }}
        <a v-if="rung.installable" :href="SYSTEM_DEPENDENCIES_URL" target="_blank" rel="noopener">How to install</a>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.ai-compute-select {
  width: 190px;
  padding: 0.25rem 2rem 0.25rem 0.5rem;
  height: 32px;
}

.ai-compute-remeasure {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-top: 0.375rem;
}

.ai-compute-reasons {
  margin: 0.375rem 0 0;
  padding-left: 1rem;
}

.ai-compute-reasons a {
  color: var(--primary);
}
</style>
