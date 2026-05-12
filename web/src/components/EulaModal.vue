<script setup>
import { ref, onMounted } from 'vue'
import { getSoftwareLicenses, updateSettings } from '../composables/api.js'

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
  <div id="eula-overlay" class="eula-overlay">
    <div class="eula-panel">
      <!-- Header -->
      <div class="eula-header">
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
      </div>

      <!-- Loading State -->
      <div v-if="isLoading" class="eula-loading">
        <div class="spinner"></div>
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
            <span v-if="isAccepting" class="btn-spinner"></span>
            <span v-else>Accept & Continue</span>
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.eula-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.85);
  backdrop-filter: blur(8px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 9999;
  padding: 1rem;
}

.eula-panel {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 12px;
  width: 100%;
  max-width: 680px;
  max-height: 90vh;
  display: flex;
  flex-direction: column;
  box-shadow:
    0 0 0 1px rgba(204, 68, 68, 0.1),
    0 20px 50px -10px rgba(0, 0, 0, 0.7);
  animation: eula-slide-in 0.3s ease-out;
}

@keyframes eula-slide-in {
  from {
    opacity: 0;
    transform: translateY(16px) scale(0.98);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}

/* Header */
.eula-header {
  padding: 1.5rem 1.5rem 1.25rem;
  border-bottom: 1px solid var(--border);
  background: var(--surface);
  border-radius: 12px 12px 0 0;
}

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
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 4rem 2rem;
  color: var(--text-secondary);
  gap: 1rem;
}

.spinner {
  width: 32px;
  height: 32px;
  border: 3px solid var(--border);
  border-top-color: var(--primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
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

.btn-spinner {
  display: inline-block;
  width: 16px;
  height: 16px;
  border: 2px solid rgba(255, 255, 255, 0.3);
  border-top-color: white;
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

/* Mobile */
@media (max-width: 768px) {
  .eula-panel {
    max-height: 95vh;
    border-radius: 8px;
  }

  .eula-header {
    border-radius: 8px 8px 0 0;
  }

  .eula-text {
    max-height: 180px;
  }

  .eula-accept-btn {
    align-self: stretch;
    text-align: center;
  }
}
</style>
