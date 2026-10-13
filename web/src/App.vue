<script setup>
import {ref, onMounted} from 'vue'
import {useEventStream} from './composables/useEventStream.js'
import {useAppState} from './composables/useAppState.js'
import {provideAppContext} from './composables/appContext.js'
import {useAiCompute} from './composables/useAiCompute.js'
import {useSidebarTabs} from './composables/useSidebarTabs.js'
import {getAstapStatus, getCatalogStatus} from './composables/api.js'
import {isEyepiecePage, SIDEBAR_TABS} from './constants'

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
import HardwareBenchmarkOverlay from './components/HardwareBenchmarkOverlay.vue'
import { BaseSpinner } from './components/ui'

// Routing
const isEyepieceRoute = ref(isEyepiecePage(window.location.pathname))

// Centralized state management
const appState = useAppState()
const {loading, globalError: error, settings, initializeState, capabilities} = appState

const showAbout = ref(false)

const sidebar = ref(null)
const {
  activeTab,
  visited,
  checkingSetup,
  showPushToSetup,
  toggle: toggleTab,
  setupComplete,
  closeSetup,
} = useSidebarTabs({capabilities, getAstapStatus, getCatalogStatus, scroller: sidebar})

// Event stream for real-time updates
const eventStream = useEventStream()

// The one-time AI compute benchmark blocks the whole UI while it measures.
const aiCompute = useAiCompute()
const benchmarking = aiCompute.benchmarking

provideAppContext(appState, eventStream)

// Every store follows the server through the one event stream.
appState.connectEvents(eventStream)
aiCompute.connectEvents({stream: eventStream, settings})

// Initialize on mount
onMounted(() => {
  initializeState()
  aiCompute.refresh()
})
</script>

<template>
  <EyepieceView v-if="isEyepieceRoute" :inert="benchmarking"/>

  <!-- EULA gate - blocks entire app until accepted -->
  <EulaModal
    v-else-if="!loading && !error && settings?.eula_accepted === false"
    :inert="benchmarking"
  />

  <div v-else class="app" :inert="benchmarking">
    <!-- Header -->
    <header class="header">
      <h1 class="logo">NightAmplifier</h1>
      <button
          class="btn btn-icon btn-header-icon"
          title="About"
          @click="showAbout = true"
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10"/>
          <path d="M12 16v-4"/>
          <path d="M12 8h.01"/>
        </svg>
      </button>
      <div class="header-spacer"></div>
      <button
          class="btn btn-icon btn-header-icon"
          :class="{ active: activeTab === SIDEBAR_TABS.PUSH_TO, loading: checkingSetup }"
          title="Push-To Navigation"
          data-test="tab-push-to"
          :aria-pressed="activeTab === SIDEBAR_TABS.PUSH_TO"
          :disabled="checkingSetup"
          @click="toggleTab(SIDEBAR_TABS.PUSH_TO)"
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
        <BaseSpinner v-else size="xs" />
      </button>
      <button
          class="btn btn-icon btn-header-icon"
          :class="{ active: activeTab === SIDEBAR_TABS.SETTINGS }"
          title="Settings"
          data-test="tab-settings"
          :aria-pressed="activeTab === SIDEBAR_TABS.SETTINGS"
          @click="toggleTab(SIDEBAR_TABS.SETTINGS)"
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
      <BaseSpinner size="md" />
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
        <BaseSpinner size="sm" />
        Retrying automatically...
      </p>
    </div>

    <!-- Main content -->
    <main v-else class="main">
      <!-- Left panel: one tab at a time, each kept mounted once visited (useSidebarTabs) -->
      <aside ref="sidebar" class="sidebar" :class="{ expanded: activeTab !== SIDEBAR_TABS.MAIN }">
        <div v-show="activeTab === SIDEBAR_TABS.MAIN" class="sidebar-tab" data-test="sidebar-main">
          <CameraPanel/>
          <CaptureControls/>
        </div>
        <div
            v-if="visited.has(SIDEBAR_TABS.PUSH_TO)"
            v-show="activeTab === SIDEBAR_TABS.PUSH_TO"
            class="sidebar-tab"
            data-test="sidebar-push-to"
        >
          <PushToPanel/>
        </div>
        <div
            v-if="visited.has(SIDEBAR_TABS.SETTINGS)"
            v-show="activeTab === SIDEBAR_TABS.SETTINGS"
            class="sidebar-tab"
            data-test="sidebar-settings"
        >
          <SettingsPanel/>
        </div>
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
        @close="closeSetup"
        @installed="setupComplete"
    />

    <!-- About / License Dialog -->
    <AboutDialog 
        v-if="showAbout" 
        @close="showAbout = false" 
    />
  </div>

  <HardwareBenchmarkOverlay/>
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
    width: var(--sidebar-width);
  }
}

@media (max-width: 768px) and (orientation: landscape) {
  .header {
    width: var(--sidebar-width-landscape);
  }
}

.header-spacer {
  flex: 1;
}

.logo {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text-secondary);
  margin: 0;
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
  color: var(--warning);
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
  color: var(--text-muted);
  font-size: var(--fs-base);
  margin-top: 0.5rem;
}

.main {
  flex: 1;
  display: flex;
  overflow: hidden;
}

.sidebar {
  width: var(--sidebar-width);
  max-width: 100%;
  background: var(--surface);
  border-right: 1px solid var(--border);
  overflow-y: auto;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

/* Lays the tab's panels out as the sidebar's own children, keeping its gap between them. */
.sidebar-tab {
  display: contents;
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

  .sidebar.expanded {
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
    width: var(--sidebar-width-landscape);
    border-right: 1px solid var(--border);
    border-bottom: none;
  }

  .main {
    flex-direction: row;
  }
}
</style>
