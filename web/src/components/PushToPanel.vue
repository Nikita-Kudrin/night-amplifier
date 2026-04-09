<script setup>
import {ref, onMounted, onUnmounted, inject, computed} from 'vue'
import {useError} from '../composables/useError.js'
import {useCatalogSearch, getCatalogClass} from '../composables/useCatalogSearch.js'
import {usePushToTarget} from '../composables/usePushToTarget.js'
import {useCoordinateInput, formatRA, formatDec} from '../composables/useCoordinates.js'
import {useTelescopeSetup} from '../composables/useTelescopeSetup.js'
import {CAMERA_DATABASE} from '../constants/cameras.js'
import {TELESCOPE_LIMITS, HELP_TEXTS} from '../constants'
import {BaseAlert, BaseToggle, BaseProLock, BaseInfoIcon} from './ui'
import AstapInstallOverlay from './AstapInstallOverlay.vue'

const {error, clearError, withErrorHandling} = useError()

// Panel state
const collapsed = ref(false)
const manualCoordsEnabled = ref(false)
const equipmentCollapsed = ref(false)
const manualSensorExpanded = ref(false)
const showDatabaseManager = ref(false)

// Catalog search
const {searchQuery, searchResults, searching, showResults, clearSearch, hideResults, revealResults} =
    useCatalogSearch()

const eventStream = inject('eventStream')
const cameras = inject('cameras', ref([]))
const selectedCamera = inject('selectedCamera', ref(null))
const capabilities = inject('capabilities', {
  has_pro: false,
  deep_sky: {advanced_rejection: false, rbf_background: false},
  planetary: {advanced_stacking: false},
  push_to: {astap_solver: false},
})

const hasProSolver = computed(() => capabilities?.value?.push_to?.astap_solver ?? false)
const showProOverlay = computed(() => !hasProSolver.value)

// Target management
const {currentTarget, selectTargetByName, clearTarget, isSolving, cancelSolve} = usePushToTarget({
  withErrorHandling,
  eventStream,
})

// Coordinate input
const {raInput, decInput, coordError, validateCoordinates, clearInputs} = useCoordinateInput()

// Telescope setup
// Connected camera info for auto-fill and profile switching
const connectedCameraInfo = computed(() => {
  if (!selectedCamera?.value || !cameras?.value) return null
  const cam = cameras.value.find(c => c.id === selectedCamera.value)
  return cam?.info || null
})

const {
  focalLength,
  pixelSizeX,
  pixelSizeY,
  sensorWidthPx,
  sensorHeightPx,
  barlowCoeff,
  manualPixelSize,
  calculatedFov,
  autoFillFromCamera,
  selectCamera,
} = useTelescopeSetup({withErrorHandling, connectedCameraInfo})

// Camera sensor search
const cameraSearchQuery = ref('')
const showCameraResults = ref(false)

const filteredCameras = computed(() => {
  const q = cameraSearchQuery.value.toLowerCase().trim()
  if (!q) return []
  return CAMERA_DATABASE.filter(c =>
    c.brand.toLowerCase().includes(q) ||
    c.model.toLowerCase().includes(q) ||
    c.sensor.toLowerCase().includes(q)
  ).slice(0, 15)
})

function selectCameraEntry(entry) {
  selectCamera(entry)
  cameraSearchQuery.value = `${entry.brand} ${entry.model}`
  showCameraResults.value = false
}

function fillFromConnectedCamera() {
  if (connectedCameraInfo.value) {
    autoFillFromCamera(connectedCameraInfo.value)
    cameraSearchQuery.value = connectedCameraInfo.value.name || ''
  }
}

function formatFov(deg) {
  if (deg >= 1) return `${deg.toFixed(2)}\u00B0`
  return `${(deg * 60).toFixed(1)}'`
}

async function selectTarget(entry) {
  // Clear search first (sets skipNextSearch flag), then set query
  clearSearch()
  searchQuery.value = entry.designation
  await selectTargetByName(entry.designation)
}

async function setCoordinateTarget() {
  const coords = validateCoordinates()
  if (!coords) return

  await withErrorHandling(async () => {
    const {setTargetByCoordinates} = await import('../composables/api.js')
    const result = await setTargetByCoordinates(coords.ra, coords.dec)
    currentTarget.value = result.target
    clearInputs()
  })
}

function handleClickOutside(event) {
  if (!event.target.closest('.search-container')) {
    hideResults()
  }
  if (!event.target.closest('.camera-search-container')) {
    showCameraResults.value = false
  }
}

onMounted(() => {
  document.addEventListener('click', handleClickOutside)
})

onUnmounted(() => {
  document.removeEventListener('click', handleClickOutside)
})
</script>

<template>
  <div class="panel">
    <div class="panel-header">
      <button class="collapse-toggle" title="Toggle Push-To panel" @click="collapsed = !collapsed">
        <svg
            :class="{ collapsed }"
            viewBox="0 0 24 24"
            width="12"
            height="12"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
        >
          <path d="M6 9l6 6 6-6"/>
        </svg>
      </button>
      <h2>Push-To Navigation</h2>
    </div>

    <BaseAlert v-if="error" type="error" @dismiss="clearError">
      {{ error }}
    </BaseAlert>

    <div v-show="!collapsed" class="push-to-content">
      <!-- Pro Only Overlay -->
      <div v-if="showProOverlay" class="pro-overlay">
        <div class="pro-message">
          <BaseProLock feature="Plate Solving" size="32px" style="margin-bottom: 0.5rem"/>
          <h3>Pro Feature</h3>
          <p>Plate solving and Push-To navigation require Night Amplifier Pro.</p>
          <a href="https://skycontrast.com/software/night-amplifier-pro" target="_blank" class="btn btn-primary btn-sm">Upgrade
            to Pro</a>
        </div>
      </div>

      <!-- Current Target Display -->
      <div v-if="currentTarget" class="current-target">
        <div class="target-header">
          <span class="target-label">Target:</span>
          <button class="btn-clear" title="Clear target" @click="clearTarget">&times;</button>
        </div>
        <div class="target-info">
          <span class="target-name">{{ currentTarget.name || 'Custom' }}</span>
          <span v-if="currentTarget.designation" class="target-designation">{{
              currentTarget.designation
            }}</span>
        </div>
        <div class="target-coords">
          <span>RA: {{ formatRA(currentTarget.ra_degrees) }}</span>
          <span>Dec: {{ formatDec(currentTarget.dec_degrees) }}</span>
        </div>
      </div>


      <!-- Object Search -->
      <div class="search-container">
        <input
            v-model="searchQuery"
            type="text"
            placeholder="Search Messier, NGC, IC..."
            class="search-input"
            :disabled="isSolving"
            @focus="revealResults"
        />
        <div v-if="searching || isSolving" class="search-spinner"></div>
        <button 
          v-if="isSolving" 
          class="btn-cancel-solve" 
          title="Cancel solving" 
          @click="cancelSolve"
        >
          Cancel
        </button>

        <!-- Search Results Dropdown -->
        <div v-if="showResults && searchResults.length > 0" class="search-results">
          <div
              v-for="entry in searchResults"
              :key="entry.designation"
              class="search-result-item"
              @click="selectTarget(entry)"
          >
            <div class="result-main">
              <span :class="['catalog-badge', getCatalogClass(entry.catalog_type)]">
                {{ entry.designation }}
              </span>
              <span class="result-name">{{ entry.name }}</span>
            </div>
            <div class="result-details">
              <span class="result-type">{{ entry.object_type }}</span>
              <span class="result-constellation">{{ entry.constellation }}</span>
            </div>
          </div>
        </div>
      </div>

      <!-- Manual Coordinates -->
      <div class="section manual-coords-section">
        <div class="manual-coords-header">
          <h3 class="section-title">Manual Coordinates</h3>
          <BaseToggle
              :model-value="manualCoordsEnabled"
              size="small"
              @update:model-value="manualCoordsEnabled = $event"
          />
        </div>
        <div v-if="manualCoordsEnabled" class="manual-coords-content">
          <div class="coord-inputs">
            <div class="coord-field">
              <label>RA</label>
              <input
                  v-model="raInput"
                  type="text"
                  placeholder="HH:MM:SS or degrees"
                  class="coord-input"
              />
            </div>
            <div class="coord-field">
              <label>Dec</label>
              <input
                  v-model="decInput"
                  type="text"
                  placeholder="DD:MM:SS or degrees"
                  class="coord-input"
              />
            </div>
          </div>
          <div v-if="coordError" class="coord-error">{{ coordError }}</div>
          <button
              class="btn btn-sm btn-primary set-coords-btn"
              :disabled="!raInput || !decInput"
              @click="setCoordinateTarget"
          >
            Set Target
          </button>
        </div>
      </div>

      <!-- Equipment -->
      <div class="section equipment-section">
        <div class="equipment-header" @click="equipmentCollapsed = !equipmentCollapsed">
          <svg
              :class="['collapse-chevron', { collapsed: equipmentCollapsed }]"
              viewBox="0 0 24 24"
              width="10"
              height="10"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
          >
            <path d="M6 9l6 6 6-6"/>
          </svg>
          <h3 class="section-title">Equipment</h3>
          <span v-if="calculatedFov && equipmentCollapsed" class="fov-badge">
            {{ formatFov(calculatedFov.x) }} &times; {{ formatFov(calculatedFov.y) }}
          </span>
        </div>

        <div v-show="!equipmentCollapsed" class="equipment-content">
          <!-- Focal Length -->
          <div class="eq-row">
            <label class="eq-label">
              Focal length
              <BaseInfoIcon :message="HELP_TEXTS.telescope_focal_length" />
            </label>
            <div class="input-with-unit eq-input-area">
              <input
                  v-model.number="focalLength"
                  type="number"
                  :min="TELESCOPE_LIMITS.focal_length_min"
                  :max="TELESCOPE_LIMITS.focal_length_max"
                  placeholder="1000"
                  class="telescope-input"
              />
              <span class="input-unit">mm</span>
            </div>
          </div>

          <!-- Camera Sensor -->
          <div class="eq-row eq-row-camera">
            <label class="eq-label">
              Camera
              <BaseInfoIcon :message="HELP_TEXTS.telescope_camera_sensor" />
            </label>
            <div class="camera-search-container eq-input-area">
              <input
                  v-model="cameraSearchQuery"
                  type="text"
                  placeholder="Search..."
                  class="telescope-input"
                  @focus="showCameraResults = true"
                  @input="showCameraResults = true"
              />
              <div v-if="showCameraResults && filteredCameras.length > 0" class="camera-results">
                <div
                    v-for="cam in filteredCameras"
                    :key="`${cam.brand}-${cam.model}`"
                    class="camera-result-item"
                    @click="selectCameraEntry(cam)"
                >
                  <div class="camera-result-main">
                    <span class="camera-brand">{{ cam.brand }}</span>
                    <span class="camera-model">{{ cam.model }}</span>
                  </div>
                  <div class="camera-result-details">
                    <span>{{ cam.sensor }}</span>
                    <span>{{ cam.pixel_size_x }}&micro;m</span>
                    <span>{{ cam.width }}&times;{{ cam.height }}</span>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <!-- Auto-fill from connected camera -->
          <button
              v-if="connectedCameraInfo"
              class="btn-autofill"
              @click="fillFromConnectedCamera"
          >
            Use connected: {{ connectedCameraInfo.name }}
          </button>

          <!-- Sensor info (when selected from DB or auto-filled, not manual) -->
          <div v-if="!manualPixelSize && pixelSizeX && sensorWidthPx" class="sensor-info">
            {{ pixelSizeX }}&micro;m &middot; {{ sensorWidthPx }}&times;{{ sensorHeightPx }}px
          </div>

          <!-- Manual Sensor Specs -->
          <div class="manual-sensor-section">
            <div class="manual-sensor-header" @click="manualSensorExpanded = !manualSensorExpanded">
              <svg
                  :class="['collapse-chevron', { collapsed: !manualSensorExpanded }]"
                  viewBox="0 0 24 24"
                  width="9"
                  height="9"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
              >
                <path d="M6 9l6 6 6-6"/>
              </svg>
              <span class="manual-sensor-label">Manual sensor specs</span>
              <BaseToggle
                  :model-value="manualPixelSize"
                  size="small"
                  @click.stop
                  @update:model-value="manualPixelSize = $event; if ($event) manualSensorExpanded = true"
              />
            </div>
            <div v-show="manualSensorExpanded" class="manual-sensor-content">
              <div class="eq-row">
                <label class="eq-label">Pixel size</label>
                <div class="eq-dual-input">
                  <div class="input-with-unit">
                    <input
                        v-model.number="pixelSizeX"
                        type="number"
                        :min="TELESCOPE_LIMITS.pixel_size_min"
                        :max="TELESCOPE_LIMITS.pixel_size_max"
                        :step="TELESCOPE_LIMITS.pixel_size_step"
                        placeholder="X"
                        class="telescope-input pixel-input"
                        :disabled="!manualPixelSize"
                    />
                    <span class="input-unit">&micro;m</span>
                  </div>
                  <span class="dual-sep">&times;</span>
                  <div class="input-with-unit">
                    <input
                        v-model.number="pixelSizeY"
                        type="number"
                        :min="TELESCOPE_LIMITS.pixel_size_min"
                        :max="TELESCOPE_LIMITS.pixel_size_max"
                        :step="TELESCOPE_LIMITS.pixel_size_step"
                        placeholder="Y"
                        class="telescope-input pixel-input"
                        :disabled="!manualPixelSize"
                    />
                    <span class="input-unit">&micro;m</span>
                  </div>
                </div>
              </div>
              <div class="eq-row">
                <label class="eq-label">Resolution</label>
                <div class="eq-dual-input">
                  <input
                      v-model.number="sensorWidthPx"
                      type="number"
                      min="1"
                      placeholder="W"
                      class="telescope-input pixel-input"
                      :disabled="!manualPixelSize"
                  />
                  <span class="dual-sep">&times;</span>
                  <input
                      v-model.number="sensorHeightPx"
                      type="number"
                      min="1"
                      placeholder="H"
                      class="telescope-input pixel-input"
                      :disabled="!manualPixelSize"
                  />
                </div>
              </div>
            </div>
          </div>

          <!-- Barlow/Reducer -->
          <div class="eq-row">
            <label class="eq-label">
              Barlow / Reducer
              <BaseInfoIcon :message="HELP_TEXTS.telescope_barlow" />
            </label>
            <div class="input-with-unit eq-input-area">
              <input
                  v-model.number="barlowCoeff"
                  type="number"
                  :min="TELESCOPE_LIMITS.barlow_min"
                  :max="TELESCOPE_LIMITS.barlow_max"
                  :step="TELESCOPE_LIMITS.barlow_step"
                  placeholder="1.0"
                  class="telescope-input"
              />
              <span class="input-unit">&times;</span>
            </div>
          </div>

          <!-- Calculated FOV -->
          <div v-if="calculatedFov" class="fov-display">
            <span class="fov-label">FOV</span>
            <span class="fov-value">{{ formatFov(calculatedFov.x) }} &times; {{ formatFov(calculatedFov.y) }}</span>
          </div>
        </div>
      </div>

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
  </div>
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
  border-radius: 6px;
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
  font-size: 0.65rem;
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
  color: var(--danger);
}

.target-info {
  display: flex;
  align-items: baseline;
  gap: 0.5rem;
  margin-bottom: 0.25rem;
}

.target-name {
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--text-primary);
}

.target-designation {
  font-size: 0.7rem;
  color: var(--primary);
}

.target-coords {
  display: flex;
  gap: 1rem;
  font-size: 0.7rem;
  color: var(--text-secondary);
  font-family: monospace;
}

.section {
  margin-top: 0.25rem;
}

.section-title {
  font-size: 0.7rem;
  color: var(--text-muted);
  text-transform: uppercase;
  margin-bottom: 0.375rem;
  padding-bottom: 0;
  border-bottom: none;
}

.manual-coords-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 0.375rem;
}

.manual-coords-header .section-title {
  margin-bottom: 0;
}

.manual-coords-content {
  margin-top: 0.375rem;
}

.search-container {
  position: relative;
}

.search-input {
  width: 100%;
  background: var(--surface-elevated);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 0.5rem;
  font-size: 0.8rem;
  color: var(--text-primary);
}

.search-input:focus {
  outline: none;
  border-color: var(--primary);
}

.search-input::placeholder {
  color: var(--text-muted);
}

.search-spinner {
  position: absolute;
  right: 0.5rem;
  top: 50%;
  transform: translateY(-50%);
  width: 14px;
  height: 14px;
  border: 2px solid var(--border);
  border-top-color: var(--primary);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  to {
    transform: translateY(-50%) rotate(360deg);
  }
}

.search-results {
  position: absolute;
  top: 100%;
  left: 0;
  right: 0;
  background: var(--surface-elevated);
  border: 1px solid var(--border);
  border-radius: 6px;
  margin-top: 0.25rem;
  max-height: 200px;
  overflow-y: auto;
  z-index: 100;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
}

.search-result-item {
  padding: 0.5rem;
  cursor: pointer;
  border-bottom: 1px solid var(--border);
}

.search-result-item:last-child {
  border-bottom: none;
}

.search-result-item:hover {
  background: var(--surface-hover);
}

.result-main {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.125rem;
}

.catalog-badge {
  font-size: 0.65rem;
  font-weight: 600;
  padding: 0.125rem 0.375rem;
  border-radius: 4px;
}

.badge-messier {
  background: #4a9eff30;
  color: #4a9eff;
}

.badge-ngc {
  background: #ff9f4a30;
  color: #ff9f4a;
}

.badge-ic {
  background: #9f4aff30;
  color: #9f4aff;
}

.badge-other {
  background: var(--surface);
  color: var(--text-secondary);
}

.result-name {
  font-size: 0.75rem;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.result-details {
  display: flex;
  gap: 0.5rem;
  font-size: 0.65rem;
  color: var(--text-muted);
}

.coord-inputs {
  display: flex;
  gap: 0.5rem;
  margin-bottom: 0.375rem;
}

.coord-field {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 0.125rem;
}

.coord-field label {
  font-size: 0.65rem;
  color: var(--text-muted);
}

.coord-input {
  width: 100%;
  background: var(--surface-elevated);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0.375rem;
  font-size: 0.75rem;
  color: var(--text-primary);
  font-family: monospace;
}

.coord-input:focus {
  outline: none;
  border-color: var(--primary);
}

.coord-input::placeholder {
  color: var(--text-muted);
  font-family: inherit;
}

.coord-error {
  font-size: 0.65rem;
  color: var(--danger);
  margin-bottom: 0.25rem;
}

.set_coords-btn {
  width: 100%;
}

.pro-overlay {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  z-index: 10;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.6);
  backdrop-filter: blur(6px);
  border-radius: 8px;
  padding: 1.5rem 1rem;
  text-align: center;
  border: 1px dashed var(--border);
}

.pro-message h3 {
  font-size: 1rem;
  margin: 0.5rem 0;
  color: var(--text-primary);
}

.pro-message p {
  font-size: 0.8rem;
  color: var(--text-secondary);
  margin-bottom: 1rem;
}

.btn-cancel-solve {
  position: absolute;
  right: 2rem;
  top: 50%;
  transform: translateY(-50%);
  background: var(--surface);
  border: 1px solid var(--border);
  color: var(--text-muted);
  font-size: 0.65rem;
  padding: 0.2rem 0.5rem;
  border-radius: 4px;
  cursor: pointer;
  z-index: 5;
}

.btn-cancel-solve:hover {
  background: var(--surface-hover);
  color: var(--danger);
  border-color: var(--danger);
}

/* Equipment section */
.equipment-header {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  cursor: pointer;
  margin-bottom: 0.25rem;
  user-select: none;
}

.equipment-header .section-title {
  margin-bottom: 0;
  flex: 1;
}

.collapse-chevron {
  transition: transform 0.15s;
  color: var(--text-muted);
  flex-shrink: 0;
}

.collapse-chevron.collapsed {
  transform: rotate(-90deg);
}

.fov-badge {
  font-size: 0.6rem;
  font-family: monospace;
  color: var(--text-secondary);
  white-space: nowrap;
}

.equipment-content {
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
}

/* Inline row: label left, input right */
.eq-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.eq-label {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  font-size: 0.7rem;
  color: var(--text-muted);
  white-space: nowrap;
  flex-shrink: 0;
  min-width: 0;
}

.eq-input-area {
  flex: 1;
  min-width: 0;
}

.eq-row-camera {
  position: relative;
}

.telescope-input {
  width: 100%;
  background: var(--surface-elevated);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0.3rem 0.375rem;
  font-size: 0.75rem;
  color: var(--text-primary);
}

.telescope-input:focus {
  outline: none;
  border-color: var(--primary);
}

.telescope-input::placeholder {
  color: var(--text-muted);
}

.telescope-input:disabled {
  opacity: 0.4;
}

.input-with-unit {
  display: flex;
  align-items: center;
  gap: 0;
}

.input-with-unit .telescope-input {
  border-radius: 4px 0 0 4px;
  border-right: none;
}

.input-unit {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 0 4px 4px 0;
  padding: 0.3rem 0.375rem;
  font-size: 0.65rem;
  color: var(--text-muted);
  white-space: nowrap;
}

/* Dual input (pixel X × Y, resolution W × H) */
.eq-dual-input {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 0;
  min-width: 0;
}

.eq-dual-input .input-with-unit {
  flex: 1;
  min-width: 0;
}

.eq-dual-input > input {
  flex: 1;
  min-width: 0;
}

.dual-sep {
  font-size: 0.6rem;
  color: var(--text-muted);
  padding: 0 0.2rem;
  flex-shrink: 0;
}

.pixel-input {
  font-family: monospace;
  font-size: 0.7rem;
}

/* Camera search dropdown */
.camera-search-container {
  position: relative;
}

.camera-results {
  position: absolute;
  top: 100%;
  left: 0;
  right: 0;
  background: var(--surface-elevated);
  border: 1px solid var(--border);
  border-radius: 6px;
  margin-top: 0.25rem;
  max-height: 180px;
  overflow-y: auto;
  z-index: 100;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
}

.camera-result-item {
  padding: 0.375rem 0.5rem;
  cursor: pointer;
  border-bottom: 1px solid var(--border);
}

.camera-result-item:last-child {
  border-bottom: none;
}

.camera-result-item:hover {
  background: var(--surface-hover);
}

.camera-result-main {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  margin-bottom: 0.125rem;
}

.camera-brand {
  font-size: 0.6rem;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
}

.camera-model {
  font-size: 0.7rem;
  color: var(--text-primary);
}

.camera-result-details {
  display: flex;
  gap: 0.5rem;
  font-size: 0.6rem;
  color: var(--text-muted);
}

.btn-autofill {
  background: none;
  border: 1px dashed var(--border);
  border-radius: 4px;
  padding: 0.2rem 0.5rem;
  font-size: 0.6rem;
  color: var(--primary);
  cursor: pointer;
  width: 100%;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.btn-autofill:hover {
  border-color: var(--primary);
  background: var(--surface);
}

.sensor-info {
  font-size: 0.6rem;
  color: var(--text-secondary);
  font-family: monospace;
  padding: 0.125rem 0;
}

/* Manual sensor specs sub-section */
.manual-sensor-section {
  border-top: 1px solid var(--border);
  padding-top: 0.375rem;
}

.manual-sensor-header {
  display: flex;
  align-items: center;
  gap: 0.3rem;
  cursor: pointer;
  user-select: none;
}

.manual-sensor-label {
  font-size: 0.7rem;
  color: var(--text-muted);
  flex: 1;
}

.manual-sensor-content {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
  margin-top: 0.375rem;
}

/* FOV display */
.fov-display {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  background: var(--surface-elevated);
  border-radius: 6px;
  padding: 0.3rem 0.5rem;
  border-left: 3px solid var(--primary);
}

.fov-label {
  font-size: 0.6rem;
  color: var(--text-muted);
  text-transform: uppercase;
  font-weight: 600;
}

.fov-value {
  font-size: 0.75rem;
  color: var(--text-primary);
  font-family: monospace;
}

.btn-manage-databases {
  background: none;
  border: none;
  color: var(--text-muted);
  font-size: 0.75rem;
  cursor: pointer;
  text-decoration: underline;
  padding: 0.25rem 0;
  text-align: center;
}

.btn-manage-databases:hover {
  color: var(--primary);
}
</style>
