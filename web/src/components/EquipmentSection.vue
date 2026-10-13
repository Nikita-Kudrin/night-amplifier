<script setup lang="ts">
import {ref, computed, useTemplateRef, watch} from 'vue'
import {useError} from '../composables/useError.js'
import {useTelescopeSetup} from '../composables/useTelescopeSetup.js'
import {useClickOutside} from '../composables/useClickOutside.js'
import {formatFov} from '../utils/format.js'
import {CAMERA_DATABASE} from '../constants/cameras.js'
import {TELESCOPE_LIMITS, HELP_TEXTS} from '../constants'
import {BaseAlert, BaseToggle, BaseInfoIcon, CollapseToggle} from './ui'
import SelectedCameraTag from './SelectedCameraTag.vue'
import type {CameraOptics, FieldOfView} from '../composables/useTelescopeSetup.js'
import type {InstalledDatabaseInfo} from '../composables/api.types.js'

const props = withDefaults(defineProps<{
  /** The selected camera's info, `null` with none selected. */
  connectedCameraInfo?: CameraOptics | null
  installedDatabases?: readonly InstalledDatabaseInfo[]
  activeDatabaseType?: string | null
}>(), {
  connectedCameraInfo: null,
  installedDatabases: () => [],
  activeDatabaseType: null,
})

const emit = defineEmits<{
  'fov-changed': [fov: FieldOfView | null]
  'database-select': [id: string]
}>()

const {error, clearError, withErrorHandling} = useError()

const equipmentCollapsed = ref(true)
const manualSensorExpanded = ref(false)

const {
  focalLength, pixelSizeX, pixelSizeY,
  sensorWidthPx, sensorHeightPx, barlowCoeff,
  manualPixelSize, calculatedFov, autoFillFromCamera, selectCamera, profileName,
} = useTelescopeSetup({withErrorHandling, connectedCameraInfo: computed(() => props.connectedCameraInfo)})

// Emit FOV changes to parent (for FOV warning)
watch(calculatedFov, (fov) => emit('fov-changed', fov), {immediate: true})

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

function selectCameraEntry(entry: (typeof CAMERA_DATABASE)[number]) {
  selectCamera(entry)
  cameraSearchQuery.value = `${entry.brand} ${entry.model}`
  showCameraResults.value = false
}

function fillFromConnectedCamera() {
  if (props.connectedCameraInfo) {
    autoFillFromCamera(props.connectedCameraInfo)
    cameraSearchQuery.value = props.connectedCameraInfo.name || ''
  }
}

const cameraSearchRef = useTemplateRef<HTMLDivElement>('cameraSearch')
useClickOutside([cameraSearchRef], () => {
  showCameraResults.value = false
})

const hasMultipleDatabases = computed(() => props.installedDatabases.length > 1)

defineExpose({calculatedFov})
</script>

<template>
  <div class="section equipment-section">
    <div class="equipment-header" @click="equipmentCollapsed = !equipmentCollapsed">
      <CollapseToggle :model-value="equipmentCollapsed" title="Toggle equipment" :size="10"/>
      <h3 class="eyebrow">Equipment <SelectedCameraTag :fallback-name="profileName"/></h3>
      <span v-if="calculatedFov && equipmentCollapsed" class="fov-badge">
        {{ formatFov(calculatedFov.x) }} &times; {{ formatFov(calculatedFov.y) }}
      </span>
    </div>

    <!-- Outside the collapsible part: a save refused while the section is folded still shows. -->
    <BaseAlert v-if="error" type="error" @dismiss="clearError">
      {{ error }}
    </BaseAlert>

    <div v-show="!equipmentCollapsed" class="equipment-content">
      <!-- Focal Length -->
      <div class="eq-row">
        <label class="eq-label">
          Focal length
          <BaseInfoIcon :message="HELP_TEXTS.telescope_focal_length"/>
        </label>
        <div class="input-with-unit eq-input-area">
          <input
              v-model.number="focalLength"
              type="number"
              :min="TELESCOPE_LIMITS.focalLength.min"
              :max="TELESCOPE_LIMITS.focalLength.max"
              placeholder="1000"
              class="input input-dense telescope-input"
          />
          <span class="input-unit">mm</span>
        </div>
      </div>

      <!-- Camera Sensor -->
      <div class="eq-row eq-row-camera">
        <label class="eq-label">
          Camera
          <BaseInfoIcon :message="HELP_TEXTS.telescope_camera_sensor"/>
        </label>
        <div ref="cameraSearch" class="camera-search-container eq-input-area">
          <input
              v-model="cameraSearchQuery"
              type="text"
              placeholder="Search..."
              class="input input-dense telescope-input"
              @focus="showCameraResults = true"
              @input="showCameraResults = true"
          />
          <div v-if="showCameraResults && filteredCameras.length > 0" class="dropdown-menu camera-results">
            <div
                v-for="cam in filteredCameras"
                :key="`${cam.brand}-${cam.model}`"
                class="dropdown-item camera-result-item"
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
          <CollapseToggle :model-value="!manualSensorExpanded" title="Toggle manual sensor specs" :size="9"/>
          <span class="manual-sensor-label">Manual sensor specs</span>
          <!-- A click on the switch is not one on the header, which folds the specs. -->
          <span class="manual-sensor-switch" @click.stop>
            <BaseToggle
                :model-value="manualPixelSize"
                size="small"
                @update:model-value="manualPixelSize = $event; if ($event) manualSensorExpanded = true"
            />
          </span>
        </div>
        <div v-show="manualSensorExpanded" class="manual-sensor-content">
          <div class="eq-row">
            <label class="eq-label">Pixel size</label>
            <div class="eq-dual-input">
              <div class="input-with-unit">
                <input
                    v-model.number="pixelSizeX"
                    type="number"
                    :min="TELESCOPE_LIMITS.pixelSize.min"
                    :max="TELESCOPE_LIMITS.pixelSize.max"
                    :step="TELESCOPE_LIMITS.pixelSize.step"
                    placeholder="X"
                    class="input input-dense telescope-input pixel-input"
                    :disabled="!manualPixelSize"
                />
                <span class="input-unit">&micro;m</span>
              </div>
              <span class="dual-sep">&times;</span>
              <div class="input-with-unit">
                <input
                    v-model.number="pixelSizeY"
                    type="number"
                    :min="TELESCOPE_LIMITS.pixelSize.min"
                    :max="TELESCOPE_LIMITS.pixelSize.max"
                    :step="TELESCOPE_LIMITS.pixelSize.step"
                    placeholder="Y"
                    class="input input-dense telescope-input pixel-input"
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
                  class="input input-dense telescope-input pixel-input"
                  :disabled="!manualPixelSize"
              />
              <span class="dual-sep">&times;</span>
              <input
                  v-model.number="sensorHeightPx"
                  type="number"
                  min="1"
                  placeholder="H"
                  class="input input-dense telescope-input pixel-input"
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
          <BaseInfoIcon :message="HELP_TEXTS.telescope_barlow"/>
        </label>
        <div class="input-with-unit eq-input-area">
          <input
              v-model.number="barlowCoeff"
              type="number"
              :min="TELESCOPE_LIMITS.barlow.min"
              :max="TELESCOPE_LIMITS.barlow.max"
              :step="TELESCOPE_LIMITS.barlow.step"
              placeholder="1.0"
              class="input input-dense telescope-input"
          />
          <span class="input-unit">&times;</span>
        </div>
      </div>

      <!-- Calculated FOV -->
      <div v-if="calculatedFov" class="fov-display">
        <span class="fov-label">FOV</span>
        <span class="fov-value">{{ formatFov(calculatedFov.x) }} &times; {{ formatFov(calculatedFov.y) }}</span>
      </div>

      <!-- Active Database selector (shown when multiple databases are installed) -->
      <div v-if="hasMultipleDatabases" class="eq-row database-select-row">
        <label class="eq-label">Database</label>
        <div class="eq-input-area">
          <select
              :value="activeDatabaseType"
              class="input input-dense telescope-input"
              @change="emit('database-select', ($event.target as HTMLSelectElement).value)"
          >
            <option v-for="db in installedDatabases" :key="db.id" :value="db.id">
              {{ db.id }} ({{ db.min_fov_deg }}°–{{ db.max_fov_deg }}°)
            </option>
          </select>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.section {
  margin-top: 0.25rem;
}

.equipment-header {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  cursor: pointer;
  margin-bottom: 0.25rem;
  user-select: none;
}

.equipment-header .eyebrow {
  margin-bottom: 0;
  flex: 1;
}

.fov-badge {
  font-size: var(--fs-3xs);
  font-family: monospace;
  color: var(--text-secondary);
  white-space: nowrap;
}

.equipment-content {
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
}

.eq-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.eq-label {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  font-size: var(--fs-xs);
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
  padding: 0.3rem 0.375rem;
}

.input-with-unit {
  display: flex;
  align-items: center;
  gap: 0;
}

.input-with-unit .telescope-input {
  border-radius: var(--radius-sm) 0 0 var(--radius-sm);
  border-right: none;
}

.input-unit {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
  padding: 0.3rem 0.375rem;
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  white-space: nowrap;
}

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
  font-size: var(--fs-3xs);
  color: var(--text-muted);
  padding: 0 0.2rem;
  flex-shrink: 0;
}

.pixel-input {
  font-family: monospace;
  font-size: var(--fs-xs);
}

.camera-search-container {
  position: relative;
}

.camera-results {
  max-height: 180px;
}

.camera-result-item {
  padding: 0.375rem 0.5rem;
}

.camera-result-main {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  margin-bottom: 0.125rem;
}

.camera-brand {
  font-size: var(--fs-3xs);
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
}

.camera-model {
  font-size: var(--fs-xs);
  color: var(--text-primary);
}

.camera-result-details {
  display: flex;
  gap: 0.5rem;
  font-size: var(--fs-3xs);
  color: var(--text-muted);
}

.btn-autofill {
  background: none;
  border: 1px dashed var(--border);
  border-radius: var(--radius-sm);
  padding: 0.2rem 0.5rem;
  font-size: var(--fs-3xs);
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
  font-size: var(--fs-3xs);
  color: var(--text-secondary);
  font-family: monospace;
  padding: 0.125rem 0;
}

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

/* No box of its own: the switch sits in the header's row as before. */
.manual-sensor-switch {
  display: contents;
}

.manual-sensor-label {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  flex: 1;
}

.manual-sensor-content {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
  margin-top: 0.375rem;
}

.fov-display {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  background: var(--surface-elevated);
  border-radius: var(--radius-md);
  padding: 0.3rem 0.5rem;
  border-left: 3px solid var(--primary);
}

.fov-label {
  font-size: var(--fs-3xs);
  color: var(--text-muted);
  text-transform: uppercase;
  font-weight: 600;
}

.fov-value {
  font-size: var(--fs-sm);
  color: var(--text-primary);
  font-family: monospace;
}
</style>
