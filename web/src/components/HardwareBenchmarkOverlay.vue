<script setup>
import {computed, nextTick, ref, watch} from 'vue'
import {useAiCompute} from '../composables/useAiCompute.js'
import {BaseSpinner} from './ui'

/**
 * Covers the whole app while the server measures which unit runs AI denoising (once per
 * computer). App marks everything else `inert`, so neither pointer nor keyboard reaches it;
 * the server refuses to start a capture meanwhile.
 */
const {benchmarking, progress} = useAiCompute()
const dialog = ref(null)

const step = computed(() => {
  const p = progress.value
  if (!p?.total) return ''
  if (!p.testing) return p.done >= p.total ? 'Finishing…' : ''
  return `Testing ${p.testing} (${Math.min(p.done + 1, p.total)} of ${p.total})`
})

const percent = computed(() => {
  const p = progress.value
  if (!p?.total) return 0
  return Math.round((100 * Math.min(p.done, p.total)) / p.total)
})

watch(benchmarking, async (on) => {
  if (!on) return
  await nextTick()
  dialog.value?.focus()
}, {immediate: true})
</script>

<template>
  <div
      v-if="benchmarking"
      ref="dialog"
      class="benchmark-overlay"
      role="alertdialog"
      aria-modal="true"
      aria-busy="true"
      aria-labelledby="benchmark-title"
      aria-describedby="benchmark-text"
      tabindex="-1"
      data-test="benchmark-overlay"
  >
    <div class="benchmark-panel">
      <BaseSpinner size="md"/>
      <h2 id="benchmark-title" class="benchmark-title">Benchmarking hardware…</h2>
      <p id="benchmark-text" class="benchmark-text">
        Night Amplifier is measuring which processor runs AI denoising fastest on this computer.
        This happens once; the result is remembered.
      </p>
      <p v-if="step" class="benchmark-step" data-test="benchmark-step">{{ step }}</p>
      <div class="benchmark-bar" role="progressbar" aria-valuemin="0" aria-valuemax="100" :aria-valuenow="percent">
        <div class="benchmark-bar-fill" :style="{width: `${percent}%`}"></div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.benchmark-overlay {
  position: fixed;
  inset: 0;
  z-index: 2000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 1rem;
  background: rgba(0, 0, 0, 0.8);
  backdrop-filter: blur(4px);
  outline: none;
}

.benchmark-panel {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.75rem;
  width: 100%;
  max-width: 420px;
  padding: 1.5rem;
  border: 1px solid var(--border);
  border-radius: 12px;
  background: var(--surface-elevated);
  text-align: center;
}

.benchmark-title {
  margin: 0;
  font-size: 1.1rem;
  color: var(--text-primary);
}

.benchmark-text,
.benchmark-step {
  margin: 0;
  font-size: 0.85rem;
  color: var(--text-secondary);
}

.benchmark-step {
  color: var(--text-primary);
}

.benchmark-bar {
  width: 100%;
  height: 6px;
  border-radius: 3px;
  overflow: hidden;
  background: var(--border);
}

.benchmark-bar-fill {
  height: 100%;
  background: var(--primary);
  transition: width 0.3s ease;
}
</style>
