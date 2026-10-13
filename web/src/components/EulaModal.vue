<script setup>
import {ref, computed} from 'vue'
import {useApplySettings} from '../composables/appContext.js'
import {useError} from '../composables/useError.js'
import {useSoftwareLicenses} from '../composables/useSoftwareLicenses.js'
import {BaseAlert, BaseModal, BaseSpinner} from './ui'
import LicenseTexts from './LicenseTexts.vue'

const emit = defineEmits(['accepted'])
const applySettings = useApplySettings()

const {
  coreLicense,
  thirdPartyLicenses,
  loading: isLoading,
  error: loadError,
  load: fetchLicenses,
} = useSoftwareLicenses()
const {error: acceptError, loading: isAccepting, withErrorHandling} = useError()
const agreed = ref(false)
// Agreeing to a text never shown is no agreement.
const canAccept = computed(() => agreed.value && !!coreLicense.value && !isAccepting.value)

async function handleAccept() {
  if (!canAccept.value) return
  await withErrorHandling(async () => {
    await applySettings({eula_accepted: true})
    emit('accepted')
  })
}
</script>

<template>
  <BaseModal 
    max-width="680px" 
    no-padding
    persistent
  >
    <template #header>
      <div class="eula-logo">
        <svg class="eula-logo-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
          <circle cx="12" cy="12" r="10" />
          <path d="M12 2a14.5 14.5 0 000 20 14.5 14.5 0 000-20" />
          <path d="M2 12h20" />
        </svg>
        <div>
          <h1 class="eula-title">NightAmplifier</h1>
          <p class="eula-subtitle">End User License Agreement</p>
        </div>
      </div>
    </template>

    <template #default>
      <!-- Loading State -->
      <div v-if="isLoading" class="loading-state eula-loading">
        <BaseSpinner size="md" />
        <p>Loading license information...</p>
      </div>

      <div v-else-if="loadError" class="eula-body">
        <BaseAlert bordered :dismissible="false">
          Could not load the license agreement: {{ loadError }}
        </BaseAlert>
        <button id="eula-retry-button" class="btn btn-primary eula-accept-btn" @click="fetchLicenses">
          Retry
        </button>
      </div>

      <!-- License Content -->
      <div v-else class="eula-body">
        <p class="eula-intro">
          Please review and accept the following license agreements to use NightAmplifier.
        </p>

        <LicenseTexts
            :core="coreLicense"
            :third-party="thirdPartyLicenses"
            core-title="Software License"
            id-prefix="eula"
        />

        <!-- Error -->
        <BaseAlert v-if="acceptError" bordered :dismissible="false">
          {{ acceptError }}
        </BaseAlert>

        <!-- Agreement -->
        <div class="eula-agreement">
          <label id="eula-agree-label" class="eula-checkbox-label">
            <input
              id="eula-agree-checkbox"
              v-model="agreed"
              type="checkbox"
              class="eula-checkbox"
            />
            <span class="eula-checkbox-custom"></span>
            <span>I have read and agree to the End User License Agreement</span>
          </label>

          <button
            id="eula-accept-button"
            class="btn btn-primary eula-accept-btn"
            :disabled="!canAccept"
            @click="handleAccept"
          >
            <BaseSpinner v-if="isAccepting" size="sm" light />
            <span v-else>Accept & Continue</span>
          </button>
        </div>
      </div>
    </template>
  </BaseModal>
</template>

<style scoped>
.eula-logo {
  display: flex;
  align-items: center;
  gap: 0.75rem;
}

.eula-logo-icon {
  width: 36px;
  height: 36px;
  color: var(--primary);
  flex-shrink: 0;
}

.eula-title {
  margin: 0;
  font-size: 1.25rem;
  font-weight: 700;
  color: var(--text-primary);
  line-height: 1.2;
}

.eula-subtitle {
  margin: 0.125rem 0 0;
  font-size: var(--fs-base);
  color: var(--text-secondary);
}

/* Loading */
.eula-loading {
  padding: 4rem 2rem;
}

/* Body */
.eula-body {
  padding: 1.25rem 1.5rem 1.5rem;
  overflow-y: auto;
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.eula-intro {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--fs-base);
  line-height: 1.5;
}

/* Agreement footer */
.eula-agreement {
  display: flex;
  flex-direction: column;
  gap: 1rem;
  padding-top: 0.75rem;
  border-top: 1px solid var(--border);
}

.eula-checkbox-label {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  cursor: pointer;
  font-size: var(--fs-base);
  color: var(--text-primary);
  user-select: none;
}

.eula-checkbox {
  position: absolute;
  opacity: 0;
  width: 0;
  height: 0;
}

.eula-checkbox-custom {
  width: 18px;
  height: 18px;
  border: 2px solid var(--border);
  border-radius: var(--radius-sm);
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all var(--transition-fast);
  background: var(--surface);
}

.eula-checkbox:checked + .eula-checkbox-custom {
  background: var(--primary);
  border-color: var(--primary);
}

.eula-checkbox:checked + .eula-checkbox-custom::after {
  content: '';
  display: block;
  width: 5px;
  height: 9px;
  border: solid var(--on-primary);
  border-width: 0 2px 2px 0;
  transform: rotate(45deg);
  margin-top: -1px;
}

.eula-checkbox:focus-visible + .eula-checkbox-custom {
  outline: 2px solid var(--primary);
  outline-offset: 2px;
}

.eula-accept-btn {
  align-self: flex-end;
  padding: 0.625rem 2rem;
  font-weight: 600;
  font-size: var(--fs-lg);
  border-radius: var(--radius-lg);
}

/* Mobile */
@media (max-width: 768px) {
  .eula-accept-btn {
    align-self: stretch;
    text-align: center;
  }
}
</style>
