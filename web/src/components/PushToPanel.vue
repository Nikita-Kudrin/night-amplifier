<script setup>
import {ref, onMounted, inject, computed, watch} from 'vue'
import {useError} from '../composables/useError.js'
import {usePushToTarget} from '../composables/usePushToTarget.js'
import {useCapabilities, useSelectedCamera} from '../composables/appContext.js'
import {formatDec, formatRA} from '../utils/coordinates.js'
import {fovWarning as warnOnFov} from '../utils/fovWarning.js'
import {BaseAlert, BaseProLock, BaseInfoIcon, BasePanel, CollapseToggle} from './ui'
import AstapInstallOverlay from './AstapInstallOverlay.vue'
import CatalogSearch from './CatalogSearch.vue'
import {HELP_TEXTS, PRO_UPGRADE_URL} from '../constants'
import EquipmentSection from './EquipmentSection.vue'
import ManualCoordinates from './ManualCoordinates.vue'
import {ignoreProOnly, updatePushToConfig} from '../composables/api.js'
import {fetchAstapState} from '../composables/useAstapInstall.js'

const {error, clearError, withErrorHandling} = useError()

// Panel state
const collapsed = ref(false)
const showDatabaseManager = ref(false)
const calculatedFov = ref(null)

// FOV warning state
const astapStatus = ref(null)
const availableDatabases = ref([])
const showFovWarning = ref(false)

async function fetchAstapStatus() {
  try {
    const state = await fetchAstapState()
    astapStatus.value = state.status
    availableDatabases.value = state.databases
  } catch (e) {
    ignoreProOnly(e, 'ASTAP status')
  }
}

// Re-fetch status when database manager is closed (user may have installed a new DB)
watch(showDatabaseManager, (visible) => {
  if (!visible) fetchAstapStatus()
})

const installedDatabases = computed(() => astapStatus.value?.installed_databases ?? [])

async function selectDatabase(dbId) {
  const db = installedDatabases.value.find(d => d.id === dbId)
  if (!db || dbId === astapStatus.value?.database_type) return

  await withErrorHandling(async () => {
    await updatePushToConfig({database_path: db.database_path})
    astapStatus.value = {...astapStatus.value, database_type: dbId}
  })
}

// Compute FOV warning: check height FOV (used by ASTAP) against the active database range
const fovWarning = computed(() => warnOnFov({
  fovDeg: calculatedFov.value?.y ?? null,
  activeId: astapStatus.value?.database_type ?? null,
  installed: installedDatabases.value,
  available: availableDatabases.value,
}))

const eventStream = inject('eventStream')
const selectedCamera = useSelectedCamera()
const {astapSolver: hasProSolver} = useCapabilities()
const showProOverlay = computed(() => !hasProSolver.value)

// Target management
const {
  currentTarget,
  selectTargetByName,
  selectTargetByCoordinates,
  clearTarget,
  isSolving,
  cancelSolve,
} = usePushToTarget({
  withErrorHandling,
  eventStream,
})

// Connected camera info for auto-fill and profile switching
const connectedCameraInfo = computed(() => selectedCamera.value?.info || null)

/** Resolves `true` once the target took, so `ManualCoordinates` can clear its fields. */
async function setCoordinateTarget(coords) {
  return (await selectTargetByCoordinates(coords.ra, coords.dec)) !== undefined
}

onMounted(fetchAstapStatus)
</script>

<template>
  <BasePanel class="push-to-panel" :bordered="false">
    <template #header>
      <CollapseToggle v-model="collapsed" title="Toggle Push-To panel"/>
      <h2>Push-To Navigation</h2>
      <button
          v-if="fovWarning"
          class="fov-warning-btn"
          :title="`FOV warning: ${fovWarning.current_fov} vs ${fovWarning.database_id} database`"
          @click.stop="showFovWarning = !showFovWarning"
      >
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
          <line x1="12" y1="9" x2="12" y2="13"/>
          <line x1="12" y1="17" x2="12.01" y2="17"/>
        </svg>
      </button>

      <BaseInfoIcon
          v-if="!calculatedFov && hasProSolver"
          type="warning"
          :message="HELP_TEXTS.push_to_without_optics"
          class="header-warning-icon"
      />
    </template>

    <BaseAlert v-if="showFovWarning && fovWarning" type="warning" @dismiss="showFovWarning = false">
      {{ fovWarning.message }}
      <strong v-if="fovWarning.suggestion"> {{ fovWarning.suggestion }}.</strong>
    </BaseAlert>

    <BaseAlert v-if="error" type="error" @dismiss="clearError">
      {{ error }}
    </BaseAlert>

    <div v-show="!collapsed" class="push-to-content">

      <!-- Pro Only Overlay -->
      <div v-if="showProOverlay" class="pro-overlay">
        <div class="pro-message">
          <BaseProLock feature="Plate Solving" size="32px" class="pro-message-lock"/>
          <h3>Pro Feature</h3>
          <p>{{ HELP_TEXTS.push_to_pro }}</p>
          <a :href="PRO_UPGRADE_URL" target="_blank" rel="noopener" class="btn btn-primary btn-sm">Upgrade to Pro</a>
        </div>
      </div>

      <!-- Current Target Display -->
      <div v-if="currentTarget" class="current-target">
        <div class="target-header">
          <span class="target-label">Target:</span>
          <button class="btn-clear" title="Clear target" @click="clearTarget">&times;</button>
        </div>
        <div class="target-info">
          <span class="target-name">{{ currentTarget.name || currentTarget.designation || 'Custom' }}</span>
          <span v-if="currentTarget.designation" class="target-designation">{{
              currentTarget.designation
            }}</span>
        </div>
        <div class="target-coords">
          <span>RA: {{ formatRA(currentTarget.ra_degrees) }}</span>
          <span>Dec: {{ formatDec(currentTarget.dec_degrees) }}</span>
        </div>
      </div>

      <CatalogSearch :solving="isSolving" @select="selectTargetByName" @cancel="cancelSolve"/>

      <ManualCoordinates :submit="setCoordinateTarget"/>

      <!-- Equipment -->
      <EquipmentSection
          :connected-camera-info="connectedCameraInfo"
          :installed-databases="installedDatabases"
          :active-database-type="astapStatus?.database_type"
          @fov-changed="calculatedFov = $event"
          @database-select="selectDatabase"
      />

      <!-- Manage Databases -->
      <button
          v-if="hasProSolver"
          class="btn-manage-databases"
          @click="showDatabaseManager = true"
      >
        Manage Star Databases
      </button>

      <AstapInstallOverlay
          v-if="showDatabaseManager"
          :allow-manage="true"
          @close="showDatabaseManager = false"
          @installed="showDatabaseManager = false"
      />
    </div>
  </BasePanel>
</template>

<style scoped>
.push-to-content {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  min-height: 180px;
}

.current-target {
  background: var(--surface-elevated);
  border-radius: var(--radius-md);
  padding: 0.5rem;
  border-left: 3px solid var(--primary);
}

.target-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 0.25rem;
}

.target-label {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  text-transform: uppercase;
}

.btn-clear {
  background: none;
  border: none;
  color: var(--text-muted);
  font-size: 1.2rem;
  cursor: pointer;
  padding: 0;
  line-height: 1;
}

.btn-clear:hover {
  color: var(--error);
}

.target-info {
  display: flex;
  align-items: baseline;
  gap: 0.5rem;
  margin-bottom: 0.25rem;
}

.target-name {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text-primary);
}

.target-designation {
  font-size: var(--fs-xs);
  color: var(--primary);
}

.target-coords {
  display: flex;
  gap: 1rem;
  font-size: var(--fs-xs);
  color: var(--text-secondary);
  font-family: monospace;
}

.section {
  margin-top: 0.25rem;
}

.pro-overlay {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  z-index: var(--z-overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--backdrop);
  backdrop-filter: blur(6px);
  border-radius: var(--radius-lg);
  padding: 1.5rem 1rem;
  text-align: center;
  border: 1px dashed var(--border);
}

.pro-message-lock {
  margin-bottom: 0.5rem;
}

.pro-message h3 {
  font-size: var(--fs-xl);
  margin: 0.5rem 0;
  color: var(--text-primary);
}

.pro-message p {
  font-size: var(--fs-md);
  color: var(--text-secondary);
  margin-bottom: 1rem;
}

.btn-manage-databases {
  background: none;
  border: none;
  color: var(--text-muted);
  font-size: var(--fs-sm);
  cursor: pointer;
  text-decoration: underline;
  padding: 0.25rem 0;
  text-align: center;
}

.btn-manage-databases:hover {
  color: var(--primary);
}

.fov-warning-btn {
  background: none;
  border: none;
  padding: 0.1rem 0.2rem;
  cursor: pointer;
  color: var(--warning);
  display: flex;
  align-items: center;
  flex-shrink: 0;
  margin-left: auto;
}

.fov-warning-btn:hover {
  color: var(--warning-hover);
}

.header-warning-icon {
  margin-left: auto;
  flex-shrink: 0;
}

/* If both are present, only the first one should have margin-left: auto */
.fov-warning-btn + .header-warning-icon {
  margin-left: 0.5rem;
}
</style>
