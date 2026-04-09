<script setup>
defineProps({
  status: {
    type: Object,
    required: true,
    // { binary_installed: boolean, database_installed: boolean, installed_databases: Array<{id, database_path}> }
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
      <template v-if="status?.installed_databases?.length > 0">
        <div v-for="db in status.installed_databases" :key="db.id" class="status-item">
          <span class="status-icon installed">&#10003;</span>
          <span>{{ db.id }} Database</span>
        </div>
      </template>
      <div v-else class="status-item">
        <span class="status-icon">&#10007;</span>
        <span>No Star Database</span>
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
