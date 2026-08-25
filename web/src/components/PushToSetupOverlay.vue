<script setup>
import {ref, computed} from 'vue'
import AstapInstaller from './AstapInstaller.vue'
import CatalogInstaller from './CatalogInstaller.vue'
import { BaseModal } from './ui'

const emit = defineEmits(['close', 'installed'])

// State
const installingAstap = ref(false)
const installingCatalog = ref(false)
const astapReady = ref(false)
const catalogReady = ref(false)

// Computed
const isFullySetup = computed(() => astapReady.value && catalogReady.value)
const anyInstalling = computed(() => installingAstap.value || installingCatalog.value)

// Methods
function onAstapInstalled() {
  astapReady.value = true
  checkAllInstalled()
}

function onCatalogInstalled() {
  catalogReady.value = true
  checkAllInstalled()
}

function checkAllInstalled() {
  if (astapReady.value && catalogReady.value) {
    emit('installed')
    // Don't auto-close immediately to let user see success state
  }
}
</script>

<template>
  <BaseModal
    title="Push-To Navigation Setup"
    max-width="480px"
    :persistent="anyInstalling"
    :show-close-button="!anyInstalling"
    @close="emit('close')"
  >
    <template #default>
      <div class="overlay-body">
        <p class="intro-text">
          Push-To navigation requires ASTAP plate solver and the target catalogs.
        </p>

        <!-- Section 1: ASTAP Plate Solver -->
        <AstapInstaller
            @installed="onAstapInstalled"
            @state-change="installingAstap = $event"
        />

        <!-- Section 2: Objects Catalog -->
        <CatalogInstaller
            @installed="onCatalogInstalled"
            @state-change="installingCatalog = $event"
        />

        <!-- Actions -->
        <div class="actions">
          <button v-if="!anyInstalling" class="btn btn-secondary" @click="emit('close')">
            {{ isFullySetup ? 'Done' : 'Cancel' }}
          </button>
        </div>
      </div>
    </template>
  </BaseModal>
</template>

<style scoped>
.overlay-body {
  padding: 1.25rem 0 0 0;
}

.error-icon {
  width: 48px;
  height: 48px;
  border-radius: 50%;
  background: var(--danger);
  color: white;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 1.5rem;
  font-weight: bold;
}

.error-message {
  color: var(--danger);
  margin: 0;
}

.intro-text {
  color: var(--text-secondary);
  font-size: 0.875rem;
  margin: 0 0 1.25rem;
  line-height: 1.5;
}

/* Setup sections */
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

.database-selector {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.75rem;
}

.selector-label {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.selector-select {
  flex: 1;
  padding: 0.4rem 0.6rem;
  font-size: 0.8rem;
  border: 1px solid var(--border);
  border-radius: 4px;
  background: var(--surface);
  color: var(--text-primary);
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

.actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.75rem;
  margin-top: 1rem;
  padding-top: 1rem;
  border-top: 1px solid var(--border);
}

.btn {
  padding: 0.5rem 1rem;
  border-radius: 6px;
  font-size: 0.875rem;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s, opacity 0.2s;
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-block {
  width: 100%;
}

.btn-primary {
  background: var(--primary);
  color: white;
  border: none;
}

.btn-primary:hover:not(:disabled) {
  background: var(--primary-hover, #3a8ee8);
}

.btn-secondary {
  background: transparent;
  color: var(--text-secondary);
  border: 1px solid var(--border);
}

.btn-secondary:hover:not(:disabled) {
  background: var(--surface-elevated);
}
</style>
