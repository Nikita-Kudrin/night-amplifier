<script setup lang="ts">
withDefaults(defineProps<{
  /** Turns the needle, not the ring: Push-To is plate solving. */
  spinning?: boolean
}>(), {
  spinning: false,
})
</script>

<template>
  <svg class="compass-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
    <circle cx="12" cy="12" r="10"/>
    <polygon
        class="needle"
        :class="{spinning}"
        points="16.24 7.76 14.12 14.12 7.76 16.24 9.88 9.88 16.24 7.76"
    />
  </svg>
</template>

<style scoped>
.needle.spinning {
  transform-box: view-box;
  transform-origin: center;
  animation: spin 1s linear infinite;
}

@media (prefers-reduced-motion: reduce) {
  .needle.spinning {
    animation: blink 1.5s ease-in-out infinite;
  }
}
</style>
