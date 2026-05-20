<script setup>
import {ref, computed, onMounted, onUnmounted, inject, watch} from 'vue'
import {getCatalogStatus, installCatalog} from '../composables/api.js'

const emit = defineEmits(['installed', 'state-change'])

// State
const loading = ref(true)
const error = ref(null)
const status = ref(null)
const installing = ref(false)
const includeStars = ref(true)

// Installation progress
const progress = ref({
  fileName: '',
  percent: null,
  bytesDownloaded: 0,
  totalBytes: null,
  stage: '', // 'starting', 'downloading', 'completed', 'failed'
  error: null,
})

// Event stream for receiving WebSocket events
const eventStream = inject('eventStream', null)

// Computed
const canInstall = computed(() => !installing.value)

const progressText = computed(() => {
  const p = progress.value
  if (p.stage === 'starting') return 'Starting download...'
  if (p.stage === 'downloading') {
    if (p.percent !== null && p.totalBytes !== null) {
      const downloadedKb = (p.bytesDownloaded / 1024).toFixed(0)
      const totalKb = (p.totalBytes / 1024).toFixed(0)
      return `Downloading ${p.fileName}: ${downloadedKb} / ${totalKb} KB`
    }
    const kb = (p.bytesDownloaded / 1024).toFixed(0)
    return `Downloading ${p.fileName}: ${kb} KB`
  }
  if (p.stage === 'completed') return 'Catalog installed'
  if (p.stage === 'failed') return `Failed: ${p.error}`
  return ''
})

// Methods
async function loadStatus() {
  loading.value = true
  error.value = null
  try {
    const data = await getCatalogStatus()
    status.value = data
    if (data.installed) {
      emit('installed', data)
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
    fileName: '',
    percent: null,
    bytesDownloaded: 0,
    totalBytes: null,
    stage: 'starting',
    error: null,
  }

  try {
    await installCatalog(includeStars.value)
  } catch (e) {
    error.value = e.message
    installing.value = false
    emit('state-change', false)
  }
}

function handleProgress(p) {
  if (!p) return

  progress.value = {
    fileName: p.file_name || p.fileName || '',
    percent: p.percent,
    bytesDownloaded: p.bytes_downloaded || p.bytesDownloaded || 0,
    totalBytes: p.total_bytes || p.totalBytes,
    stage: p.stage || 'downloading',
    error: p.error,
  }

  if (p.stage === 'completed' || p.object_count !== undefined) {
    progress.value.stage = 'completed'
    setTimeout(async () => {
      await loadStatus()
      installing.value = false
      emit('state-change', false)
    }, 500)
  } else if (p.stage === 'failed' || p.error) {
    error.value = `Installation failed: ${p.error}`
    installing.value = false
    emit('state-change', false)
  }
}

// Watch for progress updates
watch(
    () => eventStream?.catalogInstallProgress?.value,
    (p) => p && handleProgress(p),
    {deep: true}
)

// Lifecycle
onMounted(loadStatus)
onUnmounted(() => eventStream?.clearCatalogInstallProgress?.())
</script>

<template>
  <div class="setup-section" :class="{ completed: status?.installed }">
    <div class="section-header">
      <span class="section-number" :class="{ done: status?.installed }">
        {{ status?.installed ? '✓' : '2' }}
      </span>
      <h3>Target Catalogs</h3>
    </div>

    <!-- Loading -->
    <div v-if="loading && !status" class="section-content loading">
      <div class="spinner-small"></div>
      <span>Checking catalog status...</span>
    </div>

    <!-- Error -->
    <div v-else-if="error && !installing" class="section-content error">
      <p>{{ error }}</p>
      <button class="btn btn-sm" @click="loadStatus">Retry</button>
    </div>

    <!-- Installed -->
    <div v-else-if="status?.installed" class="section-status installed">
      <span class="status-check">✓</span>
      <span>Installed (Target Catalogs)</span>
    </div>

    <!-- Installing -->
    <div v-else-if="installing" class="section-installing">
      <div class="install-progress-compact">
        <div class="spinner-small"></div>
        <span class="progress-label">{{ progressText }}</span>
      </div>
      <div v-if="progress.percent !== null" class="progress-bar">
        <div class="progress-fill" :style="{ width: progress.percent + '%' }"></div>
      </div>
    </div>

    <!-- Ready to install -->
    <div v-else class="section-content">
      <div class="status-items">
        <div class="status-item">
          <span class="status-icon" :class="{ installed: status?.ngc_file_exists }">
            {{ status?.ngc_file_exists ? '✓' : '✗' }}
          </span>
          <span>NGC.csv</span>
        </div>
        <div class="status-item">
          <span class="status-icon" :class="{ installed: status?.addendum_file_exists }">
            {{ status?.addendum_file_exists ? '✓' : '✗' }}
          </span>
          <span>addendum.csv</span>
        </div>
        <div class="status-item">
          <span class="status-icon" :class="{ installed: status?.hyg_file_exists }">
            {{ status?.hyg_file_exists ? '✓' : '✗' }}
          </span>
          <span>hyg_stars.csv</span>
        </div>
      </div>

      <p class="section-hint">
        Downloads the OpenNGC and HYG target catalogs (~15MB compressed) with Messier, NGC, IC and other deep sky objects + 120k stars.
      </p>

      <div class="checkbox-container">
        <label class="checkbox-label">
          <input type="checkbox" v-model="includeStars" :disabled="!canInstall" />
          <span>Include HYG star catalog (~15MB)</span>
        </label>
      </div>

      <button
          class="btn btn-primary btn-block"
          :disabled="!canInstall"
          @click="startInstall"
      >
        Install Catalog
      </button>
    </div>
  </div>
</template>

<style scoped>
/* Reuse styles (will move to common or keep for now) */
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

.section-hint {
  font-size: 0.75rem;
  color: var(--text-muted);
  margin: 0 0 0.75rem;
  line-height: 1.4;
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

.btn-primary:hover:not(:disabled) {
  background: var(--primary-hover, #3a8ee8);
}

.checkbox-container {
  margin-bottom: 1rem;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.85rem;
  color: var(--text-primary);
  cursor: pointer;
}

.checkbox-label input[type="checkbox"] {
  width: 16px;
  height: 16px;
  cursor: pointer;
  accent-color: var(--primary);
}

.checkbox-label input[type="checkbox"]:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
