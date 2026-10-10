<script setup>
/**
 * Settings → Eyepiece: the view's layout (binoview, circular), its stream resolution and
 * the screen geometry binoview needs.
 *
 * Mirrors the `eyepiece` group and emits `apply('eyepiece', value)`, as
 * `ImageQualitySettings` does. The Processing section edits the same group's tone fields
 * in place, so the mirror follows the prop deeply: a stale copy would undo them.
 */
import {reactive, watch} from 'vue'
import {BaseToggle, BaseInfoIcon} from './ui'
import {EYEPIECE_STREAM_RESOLUTION_OPTIONS, HELP_TEXTS} from '../constants'

const props = defineProps({
  eyepiece: {type: Object, required: true},
})

const emit = defineEmits(['apply'])

const HELP = HELP_TEXTS

const local = reactive({...props.eyepiece})

watch(
    () => props.eyepiece,
    (eyepiece) => Object.assign(local, eyepiece),
    {deep: true}
)

function apply() {
  emit('apply', 'eyepiece', {...local})
}

/** Screen sizes are typed digit by digit, so they are sent 300 ms after the last edit. */
let debounceTimer = null

function applyDebounced() {
  clearTimeout(debounceTimer)
  debounceTimer = setTimeout(apply, 300)
}
</script>

<template>
  <div class="settings-section">
    <h3 class="section-title">Eyepiece</h3>

    <div class="control-group">
      <div class="control-row">
        <BaseToggle
            v-model="local.binoview"
            label="Binoview"
            size="small"
            :help="HELP.eyepiece_binoview"
            @update:model-value="apply"
        />
        <BaseToggle
            v-model="local.circular_view"
            label="Circular view"
            size="small"
            :help="HELP.eyepiece_circular_view"
            @update:model-value="apply"
        />
      </div>
    </div>

    <div class="control-group">
      <div class="control-row">
        <label class="control-label" style="margin-bottom: 0; flex: 1">
          Eyepiece Streaming Resolution
          <BaseInfoIcon :message="HELP.eyepiece_stream_resolution"/>
        </label>
        <select
            id="eyepiece-stream-resolution-select"
            v-model="local.stream_resolution"
            class="select"
            style="width: 150px; padding: 0.25rem 2rem 0.25rem 0.5rem; height: 32px"
            @change="apply"
        >
          <option
              v-for="opt in EYEPIECE_STREAM_RESOLUTION_OPTIONS"
              :key="opt.value"
              :value="opt.value"
          >
            {{ opt.label }}
          </option>
        </select>
      </div>
    </div>

    <div
        v-if="local.binoview"
        class="control-group"
        style="flex-direction: column; align-items: stretch; margin-top: 0.5rem"
    >
      <label class="control-label" style="margin-bottom: 0.5rem">
        Screen settings
        <BaseInfoIcon :message="HELP.eyepiece_screen_settings"/>
      </label>

      <div class="control-row" style="justify-content: flex-start; margin-bottom: 0.5rem">
        <input
            v-model.number="local.screen_width"
            type="number"
            min="1"
            step="0.1"
            class="screen-field screen-number"
            title="Width"
            @change="applyDebounced"
        />
        <span style="margin: 0 4px">x</span>
        <input
            v-model.number="local.screen_height"
            type="number"
            min="1"
            step="0.1"
            class="screen-field screen-number"
            title="Height"
            @change="applyDebounced"
        />
        <select
            v-model="local.screen_measurement"
            class="screen-field"
            style="margin-left: 8px"
            @change="apply"
        >
          <option value="mm">mm</option>
          <option value="inches">inches</option>
        </select>
      </div>

      <div class="control-row" style="justify-content: flex-start">
        <label style="margin-right: 8px; font-size: 0.9em; color: var(--text-secondary)">Resolution</label>
        <input
            v-model.number="local.screen_resolution_x"
            type="number"
            min="1"
            step="1"
            class="screen-field screen-number"
            title="Resolution X"
            @change="applyDebounced"
        />
        <span style="margin: 0 4px">x</span>
        <input
            v-model.number="local.screen_resolution_y"
            type="number"
            min="1"
            step="1"
            class="screen-field screen-number"
            title="Resolution Y"
            @change="applyDebounced"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-section {
  margin-bottom: 0.625rem;
}

.screen-field {
  background: var(--surface);
  color: var(--text-primary);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 4px;
}

.screen-number {
  width: 70px;
}
</style>
