<script setup lang="ts">
/**
 * One numbered step of the Push-To setup dialog: a header with the step's number (a check
 * once `done`) over whatever the step shows. The `:slotted` rules style the body classes
 * the installers put inside it.
 */
withDefaults(defineProps<{
  number: number | string
  title: string
  done?: boolean
}>(), {
  done: false,
})
</script>

<template>
  <div class="setup-section" :class="{ completed: done }">
    <div class="section-header">
      <span class="section-number" :class="{ done }">{{ done ? '✓' : number }}</span>
      <h3>{{ title }}</h3>
    </div>
    <slot/>
  </div>
</template>

<style scoped>
.setup-section {
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  margin-bottom: 1rem;
  overflow: hidden;
}

.setup-section.completed {
  border-color: var(--success);
}

.section-header {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.75rem 1rem;
  background: var(--surface-elevated);
  border-bottom: 1px solid var(--border);
}

.setup-section.completed .section-header {
  background: var(--success-bg);
  border-bottom-color: var(--success);
}

.section-number {
  width: 24px;
  height: 24px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: var(--fs-sm);
  font-weight: 600;
  background: var(--primary);
  color: var(--on-primary);
}

.section-number.done {
  background: var(--success);
}

.section-header h3 {
  margin: 0;
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text-primary);
}

:slotted(.section-content),
:slotted(.section-installing) {
  padding: 1rem;
}

:slotted(.section-content.loading),
:slotted(.section-content.error) {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  font-size: var(--fs-base);
  color: var(--text-secondary);
}

:slotted(.section-content.error p) {
  color: var(--error);
  margin: 0;
}

:slotted(.section-status) {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: var(--fs-base);
  color: var(--success);
}

:slotted(.status-check) {
  font-weight: bold;
}

:slotted(.section-status.padded) {
  padding: 0.75rem 1rem;
}

:slotted(.section-hint) {
  font-size: var(--fs-sm);
  color: var(--text-muted);
  margin: 0 0 0.75rem;
  line-height: 1.4;
}
</style>
