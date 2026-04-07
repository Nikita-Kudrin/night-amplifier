<script setup>
import {ref, computed, onMounted, onUnmounted, inject, watch} from 'vue'
import {getAstapStatus, getAstapDatabases, installAstap} from '../composables/api.js'

const emit = defineEmits(['installed', 'state-change'])

// State
const loading = ref(true)
const error = ref(null)
const status = ref(null)
const databases = ref([])
const selectedDatabase = ref('D80')
const installing = ref(false)

// Installation progress
const progress = ref({
  component: '',
  percent: null,
  bytesDownloaded: 0,
  totalBytes: null,
  stage: '',
  stageName: '',
  overallPercent: null,
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

const progressText = computed(() => {
  const p = progress.value
  if (p.stage === 'starting') {
    if (p.component === 'ASTAP CLI') return 'Installing ASTAP...'
    if (p.component && p.component.includes('Database')) {
      return `Downloading ${p.component.replace('Database', 'star database')}...`
    }
    return `Starting ${p.component}...`
  }
  if (p.stage === 'downloading') {
    if (p.percent !== null && p.totalBytes !== null) {
      const downloadedMb = (p.bytesDownloaded / (1024 * 1024)).toFixed(1)
      const totalMb = (p.totalBytes / (1024 * 1024)).toFixed(1)
      return `Downloading ${p.component}: ${downloadedMb} / ${totalMb} MB`
    }
    const mb = (p.bytesDownloaded / (1024 * 1024)).toFixed(1)
    return `Downloading ${p.component}: ${mb} MB`
  }
  if (p.stage === 'extracting') return `Extracting ${p.component}...`
  if (p.stage === 'completed') return `${p.component} installed`
  if (p.stage === 'failed') return `Failed: ${p.error}`
  return ''
})

// Methods
async function loadStatus() {
  loading.value = true
  error.value = null
  try {
    const [statusData, dbData] = await Promise.all([
      getAstapStatus(),
      getAstapDatabases(),
    ])
    status.value = statusData
    databases.value = dbData

    if (statusData.ready) {
      emit('installed', statusData)
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
  emit('state-change', true)
  error.value = null
  progress.value = {
    component: '',
    percent: null,
    bytesDownloaded: 0,
    totalBytes: null,
    stage: 'starting',
    error: null,
  }
  stageCompletion.value = {
    cliDownloaded: false,
    cliExtracted: false,
    cliCompleted: false,
    dbDownloaded: false,
    dbExtracted: false,
    dbCompleted: false,
  }

  try {
    await installAstap(selectedDatabase.value)
  } catch (e) {
    error.value = e.message
    installing.value = false
    emit('state-change', false)
  }
}

function handleProgress(p) {
  if (!p) return

  const stage = p.stage
  progress.value = {
    component: p.component || '',
    percent: p.percent,
    bytesDownloaded: p.bytesDownloaded || 0,
    totalBytes: p.totalBytes,
    stage: stage,
    stageName: p.stageName || '',
    overallPercent: p.overallPercent,
    error: p.error,
  }

  // Track stage completion
  if (stage === 'downloading') {
    if (p.stageName === 'Downloading ASTAP CLI') {
      stageCompletion.value.cliDownloaded = false
    } else if (p.stageName === 'Downloading Database') {
      stageCompletion.value.cliCompleted = true
      stageCompletion.value.dbDownloaded = false
    }
  } else if (stage === 'extracting') {
    if (p.stageName === 'Extracting ASTAP CLI') {
      stageCompletion.value.cliDownloaded = true
    } else if (p.stageName === 'Extracting Database') {
      stageCompletion.value.dbDownloaded = true
    }
  } else if (stage === 'completed') {
    if (p.stageName === 'ASTAP CLI Installed') {
      stageCompletion.value.cliExtracted = true
      stageCompletion.value.cliCompleted = true
    } else if (p.stageName === 'Database Installed') {
      stageCompletion.value.dbExtracted = true
      stageCompletion.value.dbCompleted = true
    }

    if (p.component && p.component.includes('Database')) {
      setTimeout(async () => {
        await loadStatus()
        installing.value = false
        emit('state-change', false)
      }, 500)
    }
  } else if (stage === 'failed') {
    error.value = `Installation failed: ${p.error}`
    installing.value = false
    emit('state-change', false)
  }
}

// Watch for progress updates
watch(
    () => eventStream?.astapInstallProgress?.value,
    (p) => p && handleProgress(p),
    {deep: true}
)

// Lifecycle
onMounted(loadStatus)
onUnmounted(() => eventStream?.clearAstapInstallProgress?.())
</script>

<template>
  <div class="setup-section" :class="{ completed: status?.ready }">
    <div class="section-header">
      <span class="section-number" :class="{ done: status?.ready }">
        {{ status?.ready ? '✓' : '1' }}
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
    <div v-else-if="status?.ready" class="section-status installed">
      <span class="status-check">✓</span>
      <span>Installed ({{ status.database_type }} database)</span>
    </div>

    <!-- Installing -->
    <div v-else-if="installing" class="section-installing">
      <div class="install-progress-compact">
        <div class="spinner-small"></div>
        <span class="progress-label">{{ progressText }}</span>
      </div>
      <div v-if="progress.overallPercent !== null" class="progress-bar">
        <div class="progress-fill" :style="{ width: progress.overallPercent + '%' }"></div>
      </div>
    </div>

    <!-- Ready to install -->
    <div v-else class="section-content">
      <div class="status-items">
        <div class="status-item">
          <span class="status-icon" :class="{ installed: status?.binary_installed }">
            {{ status?.binary_installed ? '✓' : '✗' }}
          </span>
          <span>ASTAP CLI</span>
        </div>
        <div class="status-item">
          <span class="status-icon" :class="{ installed: status?.database_installed }">
            {{ status?.database_installed ? '✓' : '✗' }}
          </span>
          <span>Star Database</span>
        </div>
      </div>

      <div class="database-selector">
        <label class="selector-label">Database:</label>
        <select v-model="selectedDatabase" class="selector-select">
          <option v-for="db in databases" :key="db.id" :value="db.id">
            {{ db.description }} ({{ db.fov_range }}, {{ db.size }})
          </option>
        </select>
      </div>

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
/* Reuse styles from PushToSetupOverlay.vue (will move to common or keep for now) */
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
  padding: 0.75rem 1rem;
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

.database-selector {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.75rem;
}

.selector-label {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.selector-select {
  flex: 1;
  padding: 0.4rem 0.6rem;
  font-size: 0.8rem;
  border: 1px solid var(--border);
  border-radius: 4px;
  background: var(--surface);
  color: var(--text-primary);
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
}

.btn-primary {
  background: var(--primary);
  color: white;
  border: none;
}
</style>
