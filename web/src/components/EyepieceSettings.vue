<script setup lang="ts">
/**
 * Settings → Eyepiece: the view's layout (binoview, circular), its stream resolution and
 * the screen geometry binoview needs.
 *
 * Mirrors its fields of the `eyepiece` group and emits `apply('eyepiece', fields)` with only
 * the fields it changed: the panel sends them over its own copy of the group, whose tone
 * fields Processing edits. A typed screen field is held out of syncs until it is sent.
 */
import {BaseToggle, BaseInfoIcon, SettingSelect} from './ui'
import {useDebouncedCall} from '../composables/useDebouncedCall'
import {useSectionMirror} from '../composables/useSectionMirror'
import {EYEPIECE_STREAM_RESOLUTION_OPTIONS, HELP_TEXTS, SETTINGS_DEBOUNCE_MS} from '../constants'
import type {EyepieceFields, SettingApply} from '../utils/settings'
import type {EyepieceSettings} from '../composables/api.types'

const props = defineProps<{
  eyepiece: EyepieceSettings
}>()

const emit = defineEmits<{apply: SettingApply}>()

const HELP = HELP_TEXTS

/** The fields typed digit by digit. */
type TypedField = 'screen_width' | 'screen_height' | 'screen_resolution_x' | 'screen_resolution_y'

const held = new Set<TypedField>()
const local = useSectionMirror(() => props.eyepiece, [
  'binoview',
  'circular_view',
  'stream_resolution',
  'screen_width',
  'screen_height',
  'screen_measurement',
  'screen_resolution_x',
  'screen_resolution_y',
], held)

/** Screen sizes are typed digit by digit, so they are sent a moment after the last edit. */
const {schedule, cancel} = useDebouncedCall(send, SETTINGS_DEBOUNCE_MS)

/** Sends `fields` with every typed field still waiting, in one request. */
function send(fields: EyepieceFields = {}) {
  const typed: EyepieceFields = {}
  for (const key of held) typed[key] = local[key]
  held.clear()
  cancel()
  emit('apply', 'eyepiece', {...typed, ...fields})
}

const sendField = (key: keyof typeof local) => send({[key]: local[key]})

const isSize = (n: number) => Number.isFinite(n) && n > 0
const isPixels = (n: number) => Number.isInteger(n) && n > 0

/**
 * An emptied or invalid field reads its last value again, and nothing is sent: '' or a
 * fractional resolution made the server refuse the whole group.
 */
function commitTyped(key: TypedField, event: Event, valid: (n: number) => boolean) {
  const input = event.target as HTMLInputElement
  const typed = input.value === '' ? NaN : Number(input.value)
  if (!valid(typed)) {
    input.value = String(local[key])
    return
  }
  local[key] = typed
  held.add(key)
  schedule()
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
            @update:model-value="sendField('binoview')"
        />
        <BaseToggle
            v-model="local.circular_view"
            label="Circular view"
            size="small"
            :help="HELP.eyepiece_circular_view"
            @update:model-value="sendField('circular_view')"
        />
      </div>
    </div>

    <div class="control-group">
      <SettingSelect
          id="eyepiece-stream-resolution-select"
          v-model="local.stream_resolution"
          label="Eyepiece Streaming Resolution"
          :help="HELP.eyepiece_stream_resolution"
          :options="EYEPIECE_STREAM_RESOLUTION_OPTIONS"
          @change="sendField('stream_resolution')"
      />
    </div>

    <div
        v-if="local.binoview"
        class="control-group control-group-offset"
    >
      <label class="control-label screen-label">
        Screen settings
        <BaseInfoIcon :message="HELP.eyepiece_screen_settings"/>
      </label>

      <div class="control-row screen-row screen-size-row">
        <input
            :value="local.screen_width"
            type="number"
            min="1"
            step="0.1"
            class="screen-field screen-number"
            title="Width"
            @change="commitTyped('screen_width', $event, isSize)"
        />
        <span class="screen-times">x</span>
        <input
            :value="local.screen_height"
            type="number"
            min="1"
            step="0.1"
            class="screen-field screen-number"
            title="Height"
            @change="commitTyped('screen_height', $event, isSize)"
        />
        <select
            v-model="local.screen_measurement"
            class="screen-field screen-unit"
            @change="sendField('screen_measurement')"
        >
          <option value="mm">mm</option>
          <option value="inches">inches</option>
        </select>
      </div>

      <div class="control-row screen-row">
        <label class="screen-row-label">Resolution</label>
        <input
            :value="local.screen_resolution_x"
            type="number"
            min="1"
            step="1"
            class="screen-field screen-number"
            title="Resolution X"
            @change="commitTyped('screen_resolution_x', $event, isPixels)"
        />
        <span class="screen-times">x</span>
        <input
            :value="local.screen_resolution_y"
            type="number"
            min="1"
            step="1"
            class="screen-field screen-number"
            title="Resolution Y"
            @change="commitTyped('screen_resolution_y', $event, isPixels)"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.screen-field {
  background: var(--surface);
  color: var(--text-primary);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 4px;
}

.screen-number {
  width: 70px;
}

.screen-label {
  margin-bottom: 0.5rem;
}

.screen-row {
  justify-content: flex-start;
}

.screen-size-row {
  margin-bottom: 0.5rem;
}

.screen-times {
  margin: 0 4px;
}

.screen-unit {
  margin-left: 8px;
}

.screen-row-label {
  margin-right: 8px;
  font-size: 0.9em;
  color: var(--text-secondary);
}
</style>
