<script setup>
/**
 * The chevron that folds a section: down while open, turned left while folded.
 * `v-model` is whether it is folded. Its click does not reach a header that toggles on
 * click itself, so one click folds once.
 */
const collapsed = defineModel({type: Boolean, default: false})

defineProps({
  title: {type: String, default: 'Toggle section'},
  size: {type: Number, default: 12},
})
</script>

<template>
  <button
      type="button"
      class="collapse-toggle"
      :title="title"
      :aria-expanded="!collapsed"
      @click.stop="collapsed = !collapsed"
  >
    <svg
        :class="{ collapsed }"
        viewBox="0 0 24 24"
        :width="size"
        :height="size"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
    >
      <path d="M6 9l6 6 6-6"/>
    </svg>
  </button>
</template>
