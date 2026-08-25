<script setup>
import { ref, onMounted } from 'vue'
import { getSoftwareLicenses, updateSettings } from '../composables/api.js'
import { BaseModal, BaseSpinner } from './ui'

const emit = defineEmits(['accepted'])

const coreLicense = ref('')
const thirdPartyLicenses = ref('')
const isLoading = ref(true)
const isAccepting = ref(false)
const agreed = ref(false)
const acceptError = ref(null)

async function fetchLicenses() {
  try {
    isLoading.value = true
    const res = await getSoftwareLicenses()
    coreLicense.value = res.core_license
    thirdPartyLicenses.value = res.third_party_licenses
  } catch (err) {
    console.error('Failed to load licenses:', err)
  } finally {
    isLoading.value = false
  }
}

async function handleAccept() {
  if (!agreed.value || isAccepting.value) return
  try {
    isAccepting.value = true
    acceptError.value = null
    await updateSettings({ eula_accepted: true })
    emit('accepted')
  } catch (err) {
    acceptError.value = err.message
  } finally {
    isAccepting.value = false
  }
}

onMounted(() => {
  fetchLicenses()
})
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

      <!-- License Content -->
      <div v-else class="eula-body">
        <p class="eula-intro">
          Please review and accept the following license agreements to use NightAmplifier.
        </p>

        <div class="eula-sections">
          <!-- Core License -->
          <div class="eula-section">
            <h3 class="eula-section-title">Software License</h3>
            <pre id="eula-core-license" class="eula-text">{{ coreLicense }}</pre>
          </div>

          <!-- Third Party -->
          <div v-if="thirdPartyLicenses" class="eula-section">
            <h3 class="eula-section-title">Third-Party Software</h3>
            <pre id="eula-third-party" class="eula-text">{{ thirdPartyLicenses }}</pre>
          </div>
        </div>

        <!-- Error -->
        <div v-if="acceptError" class="eula-error">
          {{ acceptError }}
        </div>

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
            :disabled="!agreed || isAccepting"
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
  font-size: 0.85rem;
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
  font-size: 0.9rem;
  line-height: 1.5;
}

/* Sections */
.eula-sections {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.eula-section-title {
  margin: 0 0 0.5rem;
  font-size: 0.95rem;
  font-weight: 600;
  color: var(--text-primary);
}

.eula-text {
  margin: 0;
  padding: 1rem;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  color: var(--text-secondary);
  font-family: var(--font-mono);
  font-size: 0.75rem;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 250px;
  overflow-y: auto;
  line-height: 1.6;
}

/* Error */
.eula-error {
  padding: 0.75rem 1rem;
  border-radius: 6px;
  font-size: 0.9rem;
  background: rgba(239, 68, 68, 0.15);
  color: #ef4444;
  border: 1px solid rgba(239, 68, 68, 0.3);
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
  font-size: 0.9rem;
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
  border-radius: 4px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s ease;
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
  border: solid white;
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
  font-size: 0.95rem;
  border-radius: 8px;
}

.eula-accept-btn {
  align-self: flex-end;
  padding: 0.625rem 2rem;
  font-weight: 600;
  font-size: 0.95rem;
  border-radius: 8px;
}

/* Mobile */
@media (max-width: 768px) {
  .eula-text {
    max-height: 180px;
  }

  .eula-accept-btn {
    align-self: stretch;
    text-align: center;
  }
}
</style>
