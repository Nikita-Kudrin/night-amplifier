<script setup>
import {computed} from 'vue'

const props = defineProps({
  feature: {
    type: String,
    default: 'This specialized'
  },
  size: {
    type: String,
    default: '12px'
  }
})

const tooltipText = computed(() => `${props.feature} feature requires Night Amplifier Pro`)
</script>

<template>
  <span class="pro-lock-container" :title="tooltipText">
    <svg
        viewBox="0 0 24 24"
        :width="size"
        :height="size"
        fill="currentColor"
        class="pro-lock-icon"
    >
      <path
          d="M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zM9 6c0-1.66 1.34-3 3-3s3 1.34 3 3v2H9V6zm9 14H6V10h12v10zm-6-3c1.1 0 2-.9 2-2s-.9-2-2-2-2 .9-2 2 .9 2 2 2z"/>
    </svg>
  </span>
</template>

<style scoped>
.pro-lock-container {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  margin-left: 4px;
  color: var(--primary, #4a9eff);
  cursor: help;
  vertical-align: middle;
}

.pro-lock-icon {
  opacity: 0.8;
  transition: transform 0.2s;
}

.pro-lock-container:hover .pro-lock-icon {
  transform: scale(1.2);
  opacity: 1;
}
</style>
