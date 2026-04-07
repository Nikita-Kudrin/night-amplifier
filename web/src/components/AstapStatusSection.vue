<script setup>
defineProps({
  status: {
    type: Object,
    required: true,
    // { binary_installed: boolean, database_installed: boolean, database_type: string }
  },
})
</script>

<template>
  <div class="status-section">
    <h3>Current Status</h3>
    <div class="status-items">
      <div class="status-item">
        <span class="status-icon" :class="{ installed: status?.binary_installed }">
          {{ status?.binary_installed ? '&#10003;' : '&#10007;' }}
        </span>
        <span>ASTAP CLI</span>
      </div>
      <div class="status-item">
        <span class="status-icon" :class="{ installed: status?.database_installed }">
          {{ status?.database_installed ? '&#10003;' : '&#10007;' }}
        </span>
        <span>Star Database {{ status?.database_type ? `(${status.database_type})` : '' }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.status-section {
  margin-bottom: 1.25rem;
}

.status-section h3 {
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  margin: 0 0 0.75rem;
}

.status-items {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.status-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.875rem;
  color: var(--text-secondary);
}

.status-icon {
  width: 20px;
  height: 20px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 0.75rem;
  background: var(--danger);
  color: white;
}

.status-icon.installed {
  background: var(--success, #22c55e);
}
</style>
