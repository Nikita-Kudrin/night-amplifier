<script setup lang="ts">
import {ref, computed, watch} from 'vue'
import {useAstapInstall} from '../composables/useAstapInstall.js'
import {BaseSpinner, DatabaseSelector, InstallProgressBar, InstallStatusList, SetupSection} from './ui'
import type {AstapStatus} from '../composables/api.types.js'

const emit = defineEmits<{installed: [status: AstapStatus]; 'state-change': [installing: boolean]}>()

const showAddDatabases = ref(false)

const {
  loading, installing, error, status, databases, selectedDatabases,
  installProgress, canInstall, hasUninstalledDatabases, progressText,
  loadStatus, startInstall,
} = useAstapInstall({onComplete: () => (showAddDatabases.value = false)})

// Emit 'installed' when status loads as ready
watch(loading, (isLoading, wasLoading) => {
  if (wasLoading && !isLoading && status.value?.ready) {
    emit('installed', status.value)
  }
})

// Emit state-change when installing changes
watch(installing, (val) => emit('state-change', val))

// Installer-specific computed
const installedDbIds = computed(() => status.value?.installed_databases.map(d => d.id) ?? [])

const statusItems = computed(() => [
  {label: 'ASTAP CLI', ok: Boolean(status.value?.binary_installed)},
  {label: 'Star Database', ok: Boolean(status.value?.database_installed)},
])
</script>

<template>
  <SetupSection number="1" title="ASTAP Plate Solver" :done="Boolean(status?.ready)">
    <!-- Loading -->
    <div v-if="loading && !status" class="section-content loading">
      <BaseSpinner size="sm" />
      <span>Checking ASTAP status...</span>
    </div>

    <!-- Error -->
    <div v-else-if="error && !installing" class="section-content error">
      <p>{{ error }}</p>
      <button class="btn btn-sm" @click="loadStatus">Retry</button>
    </div>

    <!-- Installed -->
    <div v-else-if="status?.ready && !installing && !showAddDatabases" class="section-content">
      <div class="section-status">
        <span class="status-check">&#10003;</span>
        <span>Installed ({{ installedDbIds.join(', ') }})</span>
      </div>
      <button
          v-if="hasUninstalledDatabases"
          class="btn btn-link add-databases"
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
      <InstallProgressBar
          compact
          :progress-text="progressText"
          :progress-percent="installProgress.overallPercent"
      />
    </div>

    <!-- Ready to install (fresh) -->
    <div v-else class="section-content">
      <InstallStatusList :items="statusItems"/>

      <DatabaseSelector
          v-model="selectedDatabases"
          :databases="databases"
          hint="Choose based on your telescope's field of view."
      />

      <button
          class="btn btn-primary btn-block install-btn"
          :disabled="!canInstall"
          @click="startInstall"
      >
        Install ASTAP
      </button>
    </div>
  </SetupSection>
</template>

<style scoped>
.add-databases {
  margin-top: 0.5rem;
}

.install-btn {
  margin-top: 0.75rem;
}

.add-db-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
  margin-top: 0.75rem;
}
</style>
