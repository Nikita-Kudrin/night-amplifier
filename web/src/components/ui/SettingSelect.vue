<script setup lang="ts" generic="T extends string | number">
/**
 * A settings row: label, help, and a compact select. A Pro option (`pro: true`) stays
 * listed but disabled and marked while `locked`, with the lock beside the label — Pro
 * controls stay visible and locked, never hidden.
 */
import BaseInfoIcon from './BaseInfoIcon.vue'
import BaseProLock from './BaseProLock.vue'
import type {Option} from '../../constants/tables.js'

const model = defineModel<NoInfer<T>>({required: true})

const props = withDefaults(defineProps<{
  label: string
  help?: string
  /** The choices, in order. */
  options: readonly Option<T>[]
  id?: string
  /** Whether the Pro options are unavailable; `proFeature` names them on the lock. */
  locked?: boolean
  proFeature?: string
  disabled?: boolean
  selectClass?: string | unknown[] | Record<string, boolean>
}>(), {
  help: '',
  id: undefined,
  locked: false,
  proFeature: '',
  disabled: false,
  selectClass: '',
})

/** The chosen option's value. */
const emit = defineEmits<{change: [value: T]}>()

/** A `<select>` holds every value as a string; the option it names has the value itself. */
function emitChange(event: Event) {
  const value = (event.target as HTMLSelectElement).value
  const chosen = props.options.find((opt) => String(opt.value) === value)
  if (chosen) emit('change', chosen.value)
}
</script>

<template>
  <div class="control-row">
    <label class="control-label control-label-inline" :for="id">
      {{ label }}
      <BaseProLock v-if="locked && proFeature" :feature="proFeature"/>
      <BaseInfoIcon v-if="help" :message="help"/>
    </label>
    <select
        :id="id"
        v-model="model"
        class="select select-compact"
        :class="selectClass"
        :disabled="disabled"
        @change="emitChange"
    >
      <option
          v-for="opt in options"
          :key="opt.value"
          :value="opt.value"
          :disabled="opt.pro && locked"
      >
        {{ opt.label }}{{ opt.pro && locked ? ' 🔒' : '' }}
      </option>
    </select>
  </div>
</template>
