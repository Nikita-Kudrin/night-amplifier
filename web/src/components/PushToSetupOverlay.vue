<script setup lang="ts">
import {ref, computed} from 'vue'
import AstapInstaller from './AstapInstaller.vue'
import CatalogInstaller from './CatalogInstaller.vue'
import { BaseModal } from './ui'

const emit = defineEmits<{close: []; installed: []}>()

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
          <button v-if="!anyInstalling" class="btn btn-ghost" @click="emit('close')">
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

.intro-text {
  color: var(--text-secondary);
  font-size: var(--fs-base);
  margin: 0 0 1.25rem;
  line-height: 1.5;
}

.actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.75rem;
  margin-top: 1rem;
  padding-top: 1rem;
  border-top: 1px solid var(--border);
}
</style>
