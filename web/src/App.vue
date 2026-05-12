<script setup>
import {ref, provide, onMounted, watch} from 'vue'
import {useEventStream} from './composables/useWebSocket.js'
import {useAppState} from './composables/useAppState.js'
import {getAstapStatus, getCatalogStatus} from './composables/api.js'

import CameraPanel from './components/CameraPanel.vue'
import CaptureControls from './components/CaptureControls.vue'
import SettingsPanel from './components/SettingsPanel.vue'
import PushToPanel from './components/PushToPanel.vue'
import LiveView from './components/LiveView.vue'
import StatusBar from './components/StatusBar.vue'
import PushToSetupOverlay from './components/PushToSetupOverlay.vue'
import EyepieceView from './components/EyepieceView.vue'
import AboutDialog from './components/AboutDialog.vue'
import EulaModal from './components/EulaModal.vue'

// Routing
const isEyepieceRoute = ref(window.location.pathname === '/eyepiece')

// Centralized state management
const {
  loading,
  globalError: error,
  simulatorEnabled,
  settings,
  refreshSettings,
  refreshCameras,
  initializeState,
  updateCameraStatus,
  updateCameraPhase,
  _settingsRef,
  _camerasRef,
  _selectedCameraIdRef,
  _cameraStatusRef,
  _cameraPhaseRef,
  capabilities,
} = useAppState()

const showSettings = ref(false)
const showPushTo = ref(false)
const showPushToSetup = ref(false)
const checkingSetup = ref(false)
const showAbout = ref(false)

// Event stream for real-time updates
const eventStream = useEventStream()

// Provide state to children (for backwards compatibility during migration)
provide('settings', _settingsRef)
provide('cameras', _camerasRef)
provide('selectedCamera', _selectedCameraIdRef)
provide('eventStream', eventStream)
provide('simulatorEnabled', simulatorEnabled)
provide('refreshSettings', refreshSettings)
provide('refreshCameras', refreshCameras)
provide('capabilities', capabilities)
provide('cameraStatus', _cameraStatusRef)
provide('cameraPhase', _cameraPhaseRef)

// Handle Push-To button click - check ASTAP and catalog status first
async function handlePushToClick() {
  // If already showing Push-To panel, toggle off
  if (showPushTo.value) {
    showPushTo.value = false
    return
  }

  // If user is not Pro, show the Push-To panel immediately (it handles the lock screen)
  // This avoids triggering ASTAP/catalog downloads for Community users
  if (!capabilities.value?.push_to?.astap_solver) {
    showPushTo.value = true
    return
  }

  // Check ASTAP and catalog status before showing Push-To panel
  checkingSetup.value = true
  try {
    const [astapStatus, catalogStatus] = await Promise.all([getAstapStatus(), getCatalogStatus()])

    if (astapStatus.ready && catalogStatus.installed) {
      // Everything is installed, show Push-To panel
      showPushTo.value = true
    } else {
      // Something is missing, show setup overlay
      showPushToSetup.value = true
    }
  } catch {
    // If status check fails, show setup overlay anyway
    showPushToSetup.value = true
  } finally {
    checkingSetup.value = false
  }
}

// Handle Push-To setup completion
function handleSetupComplete() {
  showPushToSetup.value = false
  showPushTo.value = true
}

// Watch for external settings updates to sync our state across instances
watch(
    () => eventStream.lastEvent.value,
    (event) => {
      if (event?.type === 'settings_updated') {
        refreshSettings()
      }
      if (event?.type === 'camera_status_updated') {
        updateCameraStatus(event.name, {
          temperature_c: event.temperature_c,
          cooler_power: event.cooler_power ?? null,
          cooler_on: event.cooler_on,
          dew_heater_on: event.dew_heater_on,
          target_temp_c: event.target_temp_c ?? null,
        })
      }
      if (event?.type === 'camera_phase_changed') {
        updateCameraPhase(event.name, event.phase)
      }
      if (event?.type === 'camera_disconnected') {
        updateCameraPhase(event.name, 'disconnected')
        refreshCameras()
      }
    }
)

function handleEulaAccepted() {
  refreshSettings()
}

// Initialize on mount
onMounted(() => {
  initializeState()
})
</script>

<template>
  <EyepieceView v-if="isEyepieceRoute"/>

  <!-- EULA gate - blocks entire app until accepted -->
  <EulaModal
    v-else-if="!loading && !error && settings?.eula_accepted === false"
    @accepted="handleEulaAccepted"
  />

  <div v-else class="app">
    <!-- Header -->
    <header class="header">
      <h1 class="logo">NightAmplifier</h1>
      <button
          class="btn btn-icon btn-header-icon"
          title="About"
          style="margin-left: 0.25rem;"
          @click="showAbout = true"
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="10"/>
          <path d="M12 16v-4"/>
          <path d="M12 8h.01"/>
        </svg>
      </button>
      <div class="header-spacer"></div>
      <button
          class="btn btn-icon btn-header-icon"
          :class="{ active: showPushTo, loading: checkingSetup }"
          title="Push-To Navigation"
          :disabled="checkingSetup"
          @click="handlePushToClick"
      >
        <svg
            v-if="!checkingSetup"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
        >
          <circle cx="12" cy="12" r="10"/>
          <polygon points="16.24 7.76 14.12 14.12 7.76 16.24 9.88 9.88 16.24 7.76"/>
        </svg>
        <span v-else class="btn-spinner"></span>
      </button>
      <button
          class="btn btn-icon btn-header-icon"
          :class="{ active: showSettings }"
          title="Settings"
          @click="showSettings = !showSettings"
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="3"/>
          <path
              d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-2 2 2 2 0 01-2-2v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83 0 2 2 0 010-2.83l.06-.06a1.65 1.65 0 00.33-1.82 1.65 1.65 0 00-1.51-1H3a2 2 0 01-2-2 2 2 0 012-2h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 010-2.83 2 2 0 012.83 0l.06.06a1.65 1.65 0 001.82.33H9a1.65 1.65 0 001-1.51V3a2 2 0 012-2 2 2 0 012 2v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 0 2 2 0 010 2.83l-.06.06a1.65 1.65 0 00-.33 1.82V9a1.65 1.65 0 001.51 1H21a2 2 0 012 2 2 2 0 01-2 2h-.09a1.65 1.65 0 00-1.51 1z"
          />
        </svg>
      </button>
    </header>

    <!-- Loading state -->
    <div v-if="loading" class="loading">
      <div class="spinner"></div>
      <p>Connecting to server...</p>
    </div>

    <!-- Error state - server unavailable -->
    <div v-else-if="error" class="error-screen">
      <div class="error-icon">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
          <rect x="2" y="4" width="20" height="16" rx="2"/>
          <line x1="8" y1="10" x2="8" y2="14"/>
          <line x1="12" y1="10" x2="12" y2="14"/>
          <line x1="16" y1="10" x2="16" y2="14"/>
        </svg>
      </div>
      <h2 class="error-title">Cannot Connect to Server</h2>
      <p class="error-message">{{ error }}</p>
      <p class="retry-status">
        <span class="retry-spinner"></span>
        Retrying automatically...
      </p>
    </div>

    <!-- Main content -->
    <main v-else class="main">
      <!-- Left panel (controls) -->
      <aside class="sidebar" :class="{ 'settings-open': showSettings, 'pushto-open': showPushTo }">
        <PushToPanel v-if="showPushTo"/>
        <CameraPanel/>
        <CaptureControls/>
        <SettingsPanel v-if="showSettings"/>
      </aside>

      <!-- Live view (center) -->
      <section class="content">
        <LiveView/>
      </section>
    </main>

    <!-- Status bar -->
    <StatusBar/>

    <!-- Push-To Setup Overlay -->
    <PushToSetupOverlay
        v-if="showPushToSetup"
        @close="showPushToSetup = false"
        @installed="handleSetupComplete"
    />

    <!-- About / License Dialog -->
    <AboutDialog 
        v-if="showAbout" 
        @close="showAbout = false" 
    />
  </div>
</template>

<style scoped>
.app {
  display: flex;
  flex-direction: column;
  height: 100vh;
  height: 100dvh;
  overflow: hidden;
}

.header {
  display: flex;
  align-items: center;
  padding: 0.375rem 0.75rem;
  background: var(--surface-elevated);
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}

@media (min-width: 769px) {
  .header {
    width: 320px;
  }
}

@media (max-width: 768px) and (orientation: landscape) {
  .header {
    width: 280px;
  }
}

.header-spacer {
  flex: 1;
}

.logo {
  font-size: 0.84rem;
  font-weight: 600;
  color: var(--text-secondary);
  margin: 0;
}

.btn-icon {
  width: 32px;
  height: 32px;
  padding: 0.375rem;
  border-radius: 6px;
}

.btn-icon svg {
  width: 100%;
  height: 100%;
}

.btn-icon.active {
  background: var(--primary);
  color: white;
}

.btn-icon.loading {
  opacity: 0.7;
}

.btn-spinner {
  width: 12px;
  height: 12px;
  border: 2px solid var(--border);
  border-top-color: var(--primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
  display: block;
}

.loading,
.error-screen {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 1rem;
  color: var(--text-secondary);
  padding: 2rem;
  text-align: center;
}

.error-icon {
  width: 64px;
  height: 64px;
  color: var(--warning, #f59e0b);
  margin-bottom: 0.5rem;
}

.error-icon svg {
  width: 100%;
  height: 100%;
}

.error-title {
  font-size: 1.25rem;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0;
}

.error-message {
  color: var(--text-secondary);
  margin: 0;
}

.retry-status {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  color: var(--text-muted, #6b7280);
  font-size: 0.875rem;
  margin-top: 0.5rem;
}

.retry-spinner {
  width: 14px;
  height: 14px;
  border: 2px solid var(--border);
  border-top-color: var(--primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

.spinner {
  width: 40px;
  height: 40px;
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

.main {
  flex: 1;
  display: flex;
  overflow: hidden;
}

.sidebar {
  width: 320px;
  max-width: 100%;
  background: var(--surface);
  border-right: 1px solid var(--border);
  overflow-y: auto;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.btn-header-icon {
  width: 24px;
  height: 24px;
  padding: 0.25rem;
  margin-left: 0.25rem;
}

.content {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg);
  overflow: hidden;
}

/* Mobile layout */
@media (max-width: 768px) {
  .main {
    flex-direction: column;
  }

  .sidebar {
    width: 100%;
    max-height: 40vh;
    border-right: none;
    border-bottom: 1px solid var(--border);
  }

  .sidebar.settings-open,
  .sidebar.pushto-open {
    max-height: 60vh;
  }

  .content {
    flex: 1;
    min-height: 0;
  }
}

/* Landscape mobile */
@media (max-width: 768px) and (orientation: landscape) {
  .sidebar {
    max-height: none;
    width: 280px;
    border-right: 1px solid var(--border);
    border-bottom: none;
  }

  .main {
    flex-direction: row;
  }
}
</style>
