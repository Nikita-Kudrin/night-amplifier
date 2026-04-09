<script setup>
import {computed, watch} from 'vue'
import {useAstapInstall} from '../composables/useAstapInstall.js'
import AstapStatusSection from './AstapStatusSection.vue'
import DatabaseSelector from './ui/DatabaseSelector.vue'
import InstallProgressBar from './ui/InstallProgressBar.vue'
import InstallStageIndicator from './ui/InstallStageIndicator.vue'

const props = defineProps({
  allowManage: {
    type: Boolean,
    default: false,
  },
})

const emit = defineEmits(['close', 'installed'])

const {
  loading, installing, error, status, databases, selectedDatabases,
  installProgress, stageCompletion,
  canInstall, hasUninstalledDatabases, progressText, overallProgressText, progressPercent,
  loadStatus, startInstall, isInstallationComplete,
} = useAstapInstall()

// Handle initial load: if already installed and not managing, close immediately
watch(loading, async (isLoading, wasLoading) => {
  if (wasLoading && !isLoading && status.value?.ready && !props.allowManage) {
    emit('installed')
    emit('close')
  }
})

// Handle installation completion
watch(
    () => isInstallationComplete(),
    (complete) => {
      if (!complete) return
      setTimeout(async () => {
        await loadStatus()
        installing.value = false
        if (status.value?.ready) {
          emit('installed')
          if (!props.allowManage) {
            emit('close')
          }
        }
      }, 500)
    }
)

// Overlay-specific computed
const stages = computed(() => {
  const result = []

  if (!status.value?.binary_installed) {
    result.push({
      label: 'ASTAP CLI',
      completed: stageCompletion.value.cliCompleted,
      active: !stageCompletion.value.cliCompleted,
      showSpinner: !stageCompletion.value.cliCompleted && installProgress.value.component === 'ASTAP CLI',
    })
  }

  for (const dbId of selectedDatabases.value) {
    const dbComponent = `${dbId} Database`
    const isCompleted = stageCompletion.value.completedDatabases.has(dbComponent)
    result.push({
      label: `${dbId} Database`,
      completed: isCompleted,
      active: (status.value?.binary_installed || stageCompletion.value.cliCompleted) && !isCompleted,
      showSpinner: installProgress.value.component === dbComponent,
    })
  }

  return result
})

const installButtonText = computed(() => {
  return status.value?.ready ? 'Download Selected' : 'Install ASTAP'
})
</script>

<template>
  <div class="overlay-backdrop" @click.self="!installing && emit('close')">
    <div class="overlay-content">
      <div class="overlay-header">
        <h2>{{ allowManage ? 'Manage Star Databases' : 'ASTAP Plate Solver Setup' }}</h2>
        <button
            v-if="!installing"
            class="btn-close"
            title="Close"
            @click="emit('close')"
        >
          &times;
        </button>
      </div>

      <!-- Loading state -->
      <div v-if="loading" class="overlay-body loading-state">
        <div class="spinner"></div>
        <p>Checking installation status...</p>
      </div>

      <!-- Error state -->
      <div v-else-if="error && !installing" class="overlay-body error-state">
        <div class="error-icon">!</div>
        <p class="error-message">{{ error }}</p>
        <button class="btn btn-primary" @click="loadStatus">Retry</button>
      </div>

      <!-- Installation in progress -->
      <div v-else-if="installing" class="overlay-body installing-state">
        <InstallStageIndicator :stages="stages"/>

        <InstallProgressBar
            :progress-text="progressText"
            :progress-percent="progressPercent"
            :overall-progress-text="overallProgressText"
            hint="This may take several minutes depending on your connection."
        />
      </div>

      <!-- Ready to install / manage databases -->
      <div v-else class="overlay-body">
        <p v-if="!allowManage" class="intro-text">
          ASTAP plate solver is required for Push-To navigation. It will be downloaded and installed
          automatically.
        </p>
        <p v-else class="intro-text">
          Download additional star databases to support different fields of view.
        </p>

        <AstapStatusSection v-if="!allowManage" :status="status"/>

        <div v-if="hasUninstalledDatabases" class="database-section">
          <h3>Select Star Databases</h3>
          <DatabaseSelector
              v-model="selectedDatabases"
              :databases="databases"
              hint="Choose based on your telescope's field of view. You can install multiple databases."
          />
        </div>
        <div v-else class="all-installed-message">
          All available databases are installed.
        </div>

        <div class="actions">
          <button class="btn btn-secondary" @click="emit('close')">
            {{ status?.ready ? 'Close' : 'Cancel' }}
          </button>
          <button
              v-if="hasUninstalledDatabases"
              class="btn btn-primary"
              :disabled="!canInstall"
              @click="startInstall"
          >
            {{ installButtonText }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.overlay-backdrop {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.8);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  padding: 1rem;
}

.overlay-content {
  background: var(--surface);
  border-radius: 12px;
  max-width: 480px;
  width: 100%;
  max-height: 90vh;
  overflow-y: auto;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
}

.overlay-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 1rem 1.25rem;
  border-bottom: 1px solid var(--border);
}

.overlay-header h2 {
  margin: 0;
  font-size: 1rem;
  font-weight: 600;
  color: var(--text-primary);
}

.btn-close {
  background: none;
  border: none;
  color: var(--text-muted);
  font-size: 1.5rem;
  cursor: pointer;
  padding: 0;
  line-height: 1;
  width: 32px;
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 6px;
}

.btn-close:hover {
  background: var(--surface-elevated);
  color: var(--text-primary);
}

.overlay-body {
  padding: 1.25rem;
}

.loading-state,
.error-state,
.installing-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  min-height: 200px;
  text-align: center;
  gap: 1rem;
}

.spinner {
  width: 32px;
  height: 32px;
  border: 3px solid var(--border);
  border-top-color: var(--primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.error-icon {
  width: 48px;
  height: 48px;
  border-radius: 50%;
  background: var(--danger);
  color: white;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 1.5rem;
  font-weight: bold;
}

.error-message {
  color: var(--danger);
  margin: 0;
}

.intro-text {
  color: var(--text-secondary);
  font-size: 0.875rem;
  margin: 0 0 1.25rem;
  line-height: 1.5;
}

.database-section {
  margin-bottom: 1.25rem;
}

.database-section h3 {
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  margin: 0 0 0.75rem;
}

.all-installed-message {
  color: var(--success, #22c55e);
  font-size: 0.875rem;
  text-align: center;
  padding: 1.5rem 0;
}

.actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.75rem;
  margin-top: 1.5rem;
  padding-top: 1rem;
  border-top: 1px solid var(--border);
}

.btn {
  padding: 0.5rem 1rem;
  border-radius: 6px;
  font-size: 0.875rem;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s, opacity 0.2s;
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-primary {
  background: var(--primary);
  color: white;
  border: none;
}

.btn-primary:hover:not(:disabled) {
  background: var(--primary-hover, #3a8ee8);
}

.btn-secondary {
  background: transparent;
  color: var(--text-secondary);
  border: 1px solid var(--border);
}

.btn-secondary:hover:not(:disabled) {
  background: var(--surface-elevated);
}
</style>
