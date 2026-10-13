<script setup lang="ts">
/**
 * The chevron that folds a section: down while open, turned left while folded.
 * `v-model` is whether it is folded. Its click bubbles on, so `useClickOutside` popovers
 * close when a panel folds; inside a header that folds on click itself, bind only
 * `:model-value` and let the header fold, or one click folds twice.
 */
const collapsed = defineModel<boolean>({default: false})

withDefaults(defineProps<{
  title?: string
  size?: number
}>(), {
  title: 'Toggle section',
  size: 12,
})
</script>

<template>
  <button
      type="button"
      class="collapse-toggle"
      :title="title"
      :aria-expanded="!collapsed"
      @click="collapsed = !collapsed"
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
