<script setup>
import {ref, computed, onMounted, onUnmounted, inject, watch} from 'vue'
import {getAstapStatus, getAstapDatabases, installAstap} from '../composables/api.js'
import AstapStatusSection from './AstapStatusSection.vue'
import DatabaseSelector from './ui/DatabaseSelector.vue'
import InstallProgressBar from './ui/InstallProgressBar.vue'
import InstallStageIndicator from './ui/InstallStageIndicator.vue'

const emit = defineEmits(['close', 'installed'])

// State
const loading = ref(true)
const installing = ref(false)
const status = ref(null)
const databases = ref([])
const selectedDatabase = ref('D80')
const error = ref(null)

// Installation progress (local copy for UI state)
const installProgress = ref({
  component: '',
  percent: null,
  bytesDownloaded: 0,
  totalBytes: null,
  stage: '', // 'starting', 'downloading', 'extracting', 'completed', 'failed'
  stageName: '', // Human-readable stage name from server
  overallPercent: null, // Overall installation progress (0-100)
  error: null,
})

// Stage completion tracking
const stageCompletion = ref({
  cliDownloaded: false,
  cliExtracted: false,
  cliCompleted: false,
  dbDownloaded: false,
  dbExtracted: false,
  dbCompleted: false,
})

// Event stream for receiving WebSocket events
const eventStream = inject('eventStream', null)

// Computed
const canInstall = computed(() => !installing.value && selectedDatabase.value)

const stages = computed(() => [
  {
    label: 'ASTAP CLI',
    completed: stageCompletion.value.cliCompleted,
    active: !stageCompletion.value.cliCompleted,
    showSpinner: !stageCompletion.value.cliCompleted && installProgress.value.component === 'ASTAP CLI'
  },
  {
    label: 'Star Database',
    completed: stageCompletion.value.dbCompleted,
    active: stageCompletion.value.cliCompleted && !stageCompletion.value.dbCompleted,
    showSpinner: stageCompletion.value.cliCompleted && installProgress.value.component.includes('Database')
  }
])

const progressText = computed(() => {
  const p = installProgress.value
  if (p.stage === 'starting') {
    if (p.component === 'ASTAP CLI') {
      return 'Installing ASTAP...'
    }
    if (p.component && p.component.includes('Database')) {
      return `Downloading ${p.component.replace('Database', 'star database')}...`
    }
    return `Starting ${p.component}...`
  }
  if (p.stage === 'downloading') {
    if (p.percent !== null && p.totalBytes !== null) {
      const downloadedMb = (p.bytesDownloaded / (1024 * 1024)).toFixed(1)
      const totalMb = (p.totalBytes / (1024 * 1024)).toFixed(1)
      return `Downloading ${p.component}: ${downloadedMb} / ${totalMb} MB (${p.percent.toFixed(1)}%)`
    }
    if (p.percent !== null) {
      return `Downloading ${p.component}: ${p.percent.toFixed(1)}%`
    }
    const mb = (p.bytesDownloaded / (1024 * 1024)).toFixed(1)
    return `Downloading ${p.component}: ${mb} MB`
  }
  if (p.stage === 'extracting') {
    if (p.percent !== null) {
      return `Extracting ${p.component}: ${p.percent.toFixed(1)}%`
    }
    return `Extracting ${p.component}...`
  }
  if (p.stage === 'completed') {
    return `${p.component} installed successfully`
  }
  if (p.stage === 'failed') {
    return `Failed: ${p.error}`
  }
  return ''
})

const overallProgressText = computed(() => {
  const p = installProgress.value
  if (p.overallPercent !== null && p.overallPercent !== undefined) {
    return `Overall progress: ${p.overallPercent.toFixed(0)}%`
  }
  return ''
})

const progressPercent = computed(() => {
  const p = installProgress.value
  // Use overall percent if available, otherwise fall back to stage percent
  if (p.overallPercent !== null && p.overallPercent !== undefined) {
    return p.overallPercent
  }
  if (p.stage === 'downloading' && p.percent !== null && p.percent !== undefined) {
    return p.percent
  }
  if (p.stage === 'extracting' && p.percent !== null && p.percent !== undefined) {
    return p.percent
  }
  if (p.stage === 'completed') {
    return 100
  }
  return null
})

// Methods
async function loadStatus() {
  loading.value = true
  error.value = null
  try {
    const [statusData, dbData] = await Promise.all([getAstapStatus(), getAstapDatabases()])
    status.value = statusData
    databases.value = dbData

    // If already installed, emit and close
    if (statusData.ready) {
      emit('installed')
      emit('close')
    }
  } catch (e) {
    error.value = e.message
  } finally {
    loading.value = false
  }
}

async function startInstall() {
  if (!canInstall.value) return

  installing.value = true
  error.value = null
  installProgress.value = {
    component: '',
    percent: null,
    bytesDownloaded: 0,
    totalBytes: null,
    stage: '',
    error: null,
  }

  try {
    await installAstap(selectedDatabase.value)
    // Installation started - progress will come via WebSocket
  } catch (e) {
    error.value = e.message
    installing.value = false
  }
}

function handleProgressUpdate(progress) {
  if (!progress) return

  const stage = progress.stage

  installProgress.value = {
    component: progress.component || '',
    percent: progress.percent,
    bytesDownloaded: progress.bytesDownloaded || 0,
    totalBytes: progress.totalBytes,
    stage: stage,
    stageName: progress.stageName || '',
    overallPercent: progress.overallPercent,
    error: progress.error,
  }

  // Track stage completion based on stage name
  if (stage === 'downloading') {
    if (progress.stageName === 'Downloading ASTAP CLI') {
      stageCompletion.value.cliDownloaded = false
    } else if (progress.stageName === 'Downloading Database') {
      stageCompletion.value.cliCompleted = true
      stageCompletion.value.dbDownloaded = false
    }
  } else if (stage === 'extracting') {
    if (progress.stageName === 'Extracting ASTAP CLI') {
      stageCompletion.value.cliDownloaded = true
      stageCompletion.value.cliExtracted = false
    } else if (progress.stageName === 'Extracting Database') {
      stageCompletion.value.dbDownloaded = true
      stageCompletion.value.dbExtracted = false
    }
  } else if (stage === 'completed') {
    if (progress.stageName === 'ASTAP CLI Installed') {
      stageCompletion.value.cliExtracted = true
      stageCompletion.value.cliCompleted = true
    } else if (progress.stageName === 'Database Installed') {
      stageCompletion.value.dbExtracted = true
      stageCompletion.value.dbCompleted = true
    }
    // Check if all components are installed (database is last)
    if (progress.component && progress.component.includes('Database')) {
      setTimeout(async () => {
        await loadStatus()
        if (status.value?.ready) {
          emit('installed')
          emit('close')
        }
      }, 500)
    }
  } else if (stage === 'failed') {
    error.value = `Installation failed: ${progress.error}`
    installing.value = false
  }
}

// Watch for ASTAP install progress updates from eventStream
watch(
    () => eventStream?.astapInstallProgress?.value,
    (progress) => {
      if (progress) {
        handleProgressUpdate(progress)
      }
    },
    {deep: true}
)

// Lifecycle
onMounted(() => {
  loadStatus()
})

onUnmounted(() => {
  // Clear the install progress when overlay closes
  eventStream?.clearAstapInstallProgress?.()
})
</script>

<template>
  <div class="overlay-backdrop" @click.self="!installing && emit('close')">
    <div class="overlay-content">
      <div class="overlay-header">
        <h2>ASTAP Plate Solver Setup</h2>
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

      <!-- Ready to install -->
      <div v-else class="overlay-body">
        <p class="intro-text">
          ASTAP plate solver is required for Push-To navigation. It will be downloaded and installed
          automatically.
        </p>

        <AstapStatusSection :status="status"/>

        <div class="database-section">
          <h3>Select Star Database</h3>
          <DatabaseSelector
              v-model="selectedDatabase"
              :databases="databases"
              hint="Choose based on your telescope's field of view."
          />
        </div>

        <div class="actions">
          <button class="btn btn-secondary" @click="emit('close')">Cancel</button>
          <button
              class="btn btn-primary"
              :disabled="!canInstall"
              @click="startInstall"
          >
            Install ASTAP
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
