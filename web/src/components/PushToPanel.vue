<script setup>
import {ref, onMounted, onUnmounted, inject, computed} from 'vue'
import {useError} from '../composables/useError.js'
import {useCatalogSearch, getCatalogClass} from '../composables/useCatalogSearch.js'
import {usePushToTarget} from '../composables/usePushToTarget.js'
import {useCoordinateInput, formatRA, formatDec} from '../composables/useCoordinates.js'
import {BaseAlert, BaseToggle, BaseProLock} from './ui'

const {error, clearError, withErrorHandling} = useError()

// Panel state
const collapsed = ref(false)
const manualCoordsEnabled = ref(false)

// Catalog search
const {searchQuery, searchResults, searching, showResults, clearSearch, hideResults, revealResults} =
    useCatalogSearch()

const eventStream = inject('eventStream')
const capabilities = inject('capabilities', {
  has_pro: false,
  deep_sky: {advanced_rejection: false, rbf_background: false},
  planetary: {advanced_stacking: false},
  push_to: {astap_solver: false},
})

const hasProSolver = computed(() => capabilities?.value?.push_to?.astap_solver ?? false)
const showProOverlay = computed(() => !hasProSolver.value)

// Target management
const {currentTarget, selectTargetByName, clearTarget} = usePushToTarget({
  withErrorHandling,
  eventStream,
})

// Coordinate input
const {raInput, decInput, coordError, validateCoordinates, clearInputs} = useCoordinateInput()

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
            @focus="revealResults"
        />
        <div v-if="searching" class="search-spinner"></div>

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
</style>
