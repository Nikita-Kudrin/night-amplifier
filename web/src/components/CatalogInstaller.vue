<script setup>
import {ref, computed, onMounted, onUnmounted, inject, watch} from 'vue'
import {getCatalogStatus, installCatalog} from '../composables/api.js'
import {formatBytes} from '../utils/format.js'
import {useTimeouts} from '../composables/useTimeouts.js'
import {BaseSpinner, InstallStatusList, SetupSection} from './ui'

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
const {later} = useTimeouts()

// Computed
const canInstall = computed(() => !installing.value)

const progressText = computed(() => {
  const p = progress.value
  if (p.stage === 'starting') return 'Starting download...'
  if (p.stage === 'downloading') {
    if (p.percent !== null && p.totalBytes !== null) {
      return `Downloading ${p.fileName}: ${formatBytes(p.bytesDownloaded)} / ${formatBytes(p.totalBytes)}`
    }
    return `Downloading ${p.fileName}: ${formatBytes(p.bytesDownloaded)}`
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
    later(async () => {
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

const statusItems = computed(() => [
  {label: 'NGC.csv', ok: Boolean(status.value?.ngc_file_exists)},
  {label: 'addendum.csv', ok: Boolean(status.value?.addendum_file_exists)},
  {label: 'hyg_stars.csv', ok: Boolean(status.value?.hyg_file_exists)},
])

// Lifecycle
onMounted(loadStatus)
onUnmounted(() => eventStream?.clearCatalogInstallProgress?.())
</script>

<template>
  <SetupSection number="2" title="Target Catalogs" :done="Boolean(status?.installed)">
    <!-- Loading -->
    <div v-if="loading && !status" class="section-content loading">
      <BaseSpinner size="sm" />
      <span>Checking catalog status...</span>
    </div>

    <!-- Error -->
    <div v-else-if="error && !installing" class="section-content error">
      <p>{{ error }}</p>
      <button class="btn btn-sm" @click="loadStatus">Retry</button>
    </div>

    <!-- Installed -->
    <div v-else-if="status?.installed" class="section-status padded">
      <span class="status-check">✓</span>
      <span>Installed (Target Catalogs)</span>
    </div>

    <!-- Installing -->
    <div v-else-if="installing" class="section-installing">
      <div class="install-progress-compact">
        <BaseSpinner size="sm" />
        <span>{{ progressText }}</span>
      </div>
      <div v-if="progress.percent !== null" class="progress-bar">
        <div class="progress-fill" :style="{ width: progress.percent + '%' }"></div>
      </div>
    </div>

    <!-- Ready to install -->
    <div v-else class="section-content">
      <InstallStatusList :items="statusItems"/>

      <p class="section-hint">
        Downloads the OpenNGC and HYG target catalogs (~15MB compressed) with Messier, NGC, IC and other deep sky objects + 120k stars.
      </p>

      <label class="checkbox-label">
        <input v-model="includeStars" type="checkbox" :disabled="!canInstall" />
        <span>Include HYG star catalog (~15MB)</span>
      </label>

      <button
          class="btn btn-primary btn-block"
          :disabled="!canInstall"
          @click="startInstall"
      >
        Install Catalog
      </button>
    </div>
  </SetupSection>
</template>

<style scoped>
.status-check {
  font-weight: bold;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 1rem;
  font-size: var(--fs-base);
  color: var(--text-primary);
  cursor: pointer;
}
</style>
