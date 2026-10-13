<script setup lang="ts">
import {computed, ref, watch} from 'vue'
import {useCatalogInstall} from '../composables/useCatalogInstall.js'
import {BaseSpinner, InstallProgressBar, InstallStatusList, SetupSection} from './ui'
import type {CatalogStatus} from '../composables/api.types.js'

const emit = defineEmits<{installed: [status: CatalogStatus]; 'state-change': [installing: boolean]}>()

const includeStars = ref(true)

const {loading, installing, status, error, progress, canInstall, progressText, loadStatus, startInstall} =
    useCatalogInstall()

// Every load that finds the catalog there says so: the first, and the one after an install.
watch(status, (data) => {
  if (data?.installed) emit('installed', data)
})
watch(installing, (busy) => emit('state-change', busy))

const statusItems = computed(() => [
  {label: 'NGC.csv', ok: Boolean(status.value?.ngc_file_exists)},
  {label: 'addendum.csv', ok: Boolean(status.value?.addendum_file_exists)},
  {label: 'hyg_stars.csv', ok: Boolean(status.value?.hyg_file_exists)},
])
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
      <InstallProgressBar compact :progress-text="progressText" :progress-percent="progress.percent"/>
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
          @click="startInstall(includeStars)"
      >
        Install Catalog
      </button>
    </div>
  </SetupSection>
</template>

<style scoped>
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
