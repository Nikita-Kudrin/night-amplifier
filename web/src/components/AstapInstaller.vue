<script setup>
import {ref, computed, watch} from 'vue'
import {useAstapInstall} from '../composables/useAstapInstall.js'
import DatabaseSelector from './ui/DatabaseSelector.vue'

const emit = defineEmits(['installed', 'state-change'])

const {
  loading, installing, error, status, databases, selectedDatabases,
  installProgress, canInstall, hasUninstalledDatabases, progressText,
  loadStatus, startInstall, isInstallationComplete,
} = useAstapInstall()

const showAddDatabases = ref(false)

// Emit 'installed' when status loads as ready
watch(loading, (isLoading, wasLoading) => {
  if (wasLoading && !isLoading && status.value?.ready) {
    emit('installed', status.value)
  }
})

// Emit state-change when installing changes
watch(installing, (val) => emit('state-change', val))

// Handle installation completion
watch(
    () => isInstallationComplete(),
    (complete) => {
      if (!complete) return
      setTimeout(async () => {
        await loadStatus()
        installing.value = false
        showAddDatabases.value = false
      }, 500)
    }
)

// Installer-specific computed
const installedDbIds = computed(() => {
  if (!status.value?.installed_databases) return []
  return status.value.installed_databases.map(d => d.id)
})
</script>

<template>
  <div class="setup-section" :class="{ completed: status?.ready }">
    <div class="section-header">
      <span class="section-number" :class="{ done: status?.ready }">
        {{ status?.ready ? '&#10003;' : '1' }}
      </span>
      <h3>ASTAP Plate Solver</h3>
    </div>

    <!-- Loading -->
    <div v-if="loading && !status" class="section-content loading">
      <div class="spinner-small"></div>
      <span>Checking ASTAP status...</span>
    </div>

    <!-- Error -->
    <div v-else-if="error && !installing" class="section-content error">
      <p>{{ error }}</p>
      <button class="btn btn-sm" @click="loadStatus">Retry</button>
    </div>

    <!-- Installed -->
    <div v-else-if="status?.ready && !installing && !showAddDatabases" class="section-content">
      <div class="section-status installed">
        <span class="status-check">&#10003;</span>
        <span>Installed ({{ installedDbIds.join(', ') }})</span>
      </div>
      <button
          v-if="hasUninstalledDatabases"
          class="btn btn-link"
          @click="showAddDatabases = true"
      >
        Add more databases
      </button>
    </div>

    <!-- Add databases (post-setup) -->
    <div v-else-if="showAddDatabases && !installing" class="section-content">
      <DatabaseSelector
          v-model="selectedDatabases"
          :databases="databases"
          hint="Select additional databases to download."
      />
      <div class="add-db-actions">
        <button class="btn btn-sm" @click="showAddDatabases = false">Cancel</button>
        <button
            class="btn btn-primary btn-sm"
            :disabled="!canInstall"
            @click="startInstall"
        >
          Download Selected
        </button>
      </div>
    </div>

    <!-- Installing -->
    <div v-else-if="installing" class="section-installing">
      <div class="install-progress-compact">
        <div class="spinner-small"></div>
        <span class="progress-label">{{ progressText }}</span>
      </div>
      <div v-if="installProgress.overallPercent !== null" class="progress-bar">
        <div class="progress-fill" :style="{ width: installProgress.overallPercent + '%' }"></div>
      </div>
    </div>

    <!-- Ready to install (fresh) -->
    <div v-else class="section-content">
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
          <span>Star Database</span>
        </div>
      </div>

      <DatabaseSelector
          v-model="selectedDatabases"
          :databases="databases"
          hint="Choose based on your telescope's field of view."
      />

      <button
          class="btn btn-primary btn-block"
          :disabled="!canInstall"
          @click="startInstall"
      >
        Install ASTAP
      </button>
    </div>
  </div>
</template>

<style scoped>
.setup-section {
  border: 1px solid var(--border);
  border-radius: 8px;
  margin-bottom: 1rem;
  overflow: hidden;
}

.setup-section.completed {
  border-color: var(--success, #22c55e);
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
  background: rgba(34, 197, 94, 0.1);
  border-bottom-color: var(--success, #22c55e);
}

.section-number {
  width: 24px;
  height: 24px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 0.75rem;
  font-weight: 600;
  background: var(--primary);
  color: white;
}

.section-number.done {
  background: var(--success, #22c55e);
}

.section-header h3 {
  margin: 0;
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--text-primary);
}

.section-content {
  padding: 1rem;
}

.section-content.loading, .section-content.error {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  font-size: 0.875rem;
  color: var(--text-secondary);
}

.section-content.error p {
  color: var(--danger);
  margin: 0;
}

.section-status {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.875rem;
}

.section-status.installed {
  color: var(--success, #22c55e);
}

.status-check {
  font-weight: bold;
}

.section-installing {
  padding: 1rem;
}

.install-progress-compact {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.5rem;
}

.spinner-small {
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

.progress-label {
  font-size: 0.8rem;
  color: var(--text-secondary);
}

.status-items {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  margin-bottom: 0.75rem;
}

.status-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.8rem;
  color: var(--text-secondary);
}

.status-icon {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 0.7rem;
  background: var(--danger);
  color: white;
}

.status-icon.installed {
  background: var(--success, #22c55e);
}

.progress-bar {
  width: 100%;
  height: 6px;
  background: var(--surface-elevated);
  border-radius: 3px;
  overflow: hidden;
}

.progress-fill {
  height: 100%;
  background: var(--primary);
  transition: width 0.3s ease;
}

.btn {
  padding: 0.5rem 1rem;
  border-radius: 6px;
  font-size: 0.875rem;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s, opacity 0.2s;
}

.btn-sm {
  padding: 0.25rem 0.5rem;
  font-size: 0.75rem;
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-block {
  width: 100%;
  margin-top: 0.75rem;
}

.btn-primary {
  background: var(--primary);
  color: white;
  border: none;
}

.btn-link {
  background: none;
  border: none;
  color: var(--primary);
  font-size: 0.8rem;
  padding: 0.25rem 0;
  cursor: pointer;
  text-decoration: underline;
  margin-top: 0.5rem;
}

.btn-link:hover {
  color: var(--primary-hover, #3a8ee8);
}

.add-db-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
  margin-top: 0.75rem;
}
</style>
