<script setup>
/**
 * A settings row: label, help, and a compact select. A Pro option (`pro: true`) stays
 * listed but disabled and marked while `locked`, with the lock beside the label — Pro
 * controls stay visible and locked, never hidden.
 */
import BaseInfoIcon from './BaseInfoIcon.vue'
import BaseProLock from './BaseProLock.vue'

const model = defineModel({type: String, default: ''})

defineProps({
  label: {type: String, required: true},
  help: {type: String, default: ''},
  /** `{value, label, pro?}`: the choices, in order. */
  options: {type: Array, required: true},
  id: {type: String, default: undefined},
  /** Whether the Pro options are unavailable; `proFeature` names them on the lock. */
  locked: {type: Boolean, default: false},
  proFeature: {type: String, default: ''},
  disabled: {type: Boolean, default: false},
  selectClass: {type: [String, Array, Object], default: ''},
})

const emit = defineEmits(['change'])
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
        @change="emit('change', $event.target.value)"
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
