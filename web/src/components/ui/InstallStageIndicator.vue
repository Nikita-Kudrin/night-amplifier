<script setup>
defineProps({
  stages: {
    type: Array,
    required: true,
    // Each stage: { label: string, completed: boolean, active: boolean, showSpinner: boolean }
  },
})
</script>

<template>
  <div class="stage-indicators">
    <template v-for="(stage, index) in stages" :key="stage.label">
      <div
          class="stage-item"
          :class="{ active: stage.active, completed: stage.completed }"
      >
        <span class="stage-icon">
          <template v-if="stage.completed">&#10003;</template>
          <template v-else-if="stage.showSpinner">
            <div class="mini-spinner"></div>
          </template>
          <template v-else>{{ index + 1 }}</template>
        </span>
        <span class="stage-label">{{ stage.label }}</span>
      </div>
      <div
          v-if="index < stages.length - 1"
          class="stage-connector"
          :class="{ completed: stage.completed }"
      ></div>
    </template>
  </div>
</template>

<style scoped>
.stage-indicators {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0;
  margin-bottom: 1.5rem;
  width: 100%;
}

.stage-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.5rem;
  opacity: 0.5;
}

.stage-item.active,
.stage-item.completed {
  opacity: 1;
}

.stage-icon {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 0.875rem;
  font-weight: 600;
  background: var(--surface-elevated);
  border: 2px solid var(--border);
  color: var(--text-muted);
}

.stage-item.active .stage-icon {
  border-color: var(--primary);
  color: var(--primary);
}

.stage-item.completed .stage-icon {
  background: var(--success, #22c55e);
  border-color: var(--success, #22c55e);
  color: white;
}

.stage-label {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.stage-item.active .stage-label {
  color: var(--text-primary);
}

.stage-item.completed .stage-label {
  color: var(--success, #22c55e);
}

.stage-connector {
  width: 40px;
  height: 2px;
  background: var(--border);
  margin: 0 0.5rem;
  margin-bottom: 1.5rem;
}

.stage-connector.completed {
  background: var(--success, #22c55e);
}

.mini-spinner {
  width: 16px;
  height: 16px;
  border: 2px solid var(--border);
  border-top-color: var(--primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
