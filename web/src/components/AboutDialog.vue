<script setup>
import { ref, onMounted } from 'vue'
import { getLicenseStatus, updateLicense, getSoftwareLicenses } from '../composables/api.js'

const emit = defineEmits(['close'])

const activeTab = ref('license') // 'license' | 'software'

// License State
const licenseStatus = ref(null)
const licenseToken = ref('')
const isLoadingLicense = ref(true)
const isUpdatingLicense = ref(false)
const licenseError = ref(null)
const licenseSuccess = ref(false)

// Software Licenses State
const coreLicense = ref('')
const thirdPartyLicenses = ref('')
const isLoadingSoftware = ref(true)

async function fetchLicenseStatus() {
  try {
    isLoadingLicense.value = true
    licenseStatus.value = await getLicenseStatus()
  } catch (err) {
    licenseError.value = err.message
  } finally {
    isLoadingLicense.value = false
  }
}

async function fetchSoftwareLicenses() {
  try {
    isLoadingSoftware.value = true
    const res = await getSoftwareLicenses()
    coreLicense.value = res.core_license
    thirdPartyLicenses.value = res.third_party_licenses
  } catch (err) {
    console.error('Failed to load software licenses:', err)
  } finally {
    isLoadingSoftware.value = false
  }
}

async function handleUpdateLicense() {
  if (!licenseToken.value.trim()) return

  try {
    isUpdatingLicense.value = true
    licenseError.value = null
    licenseSuccess.value = false
    
    licenseStatus.value = await updateLicense(licenseToken.value.trim())
    licenseSuccess.value = true
    licenseToken.value = '' // Clear input on success
  } catch (err) {
    licenseError.value = err.message
  } finally {
    isUpdatingLicense.value = false
  }
}

function formatDate(dateString) {
  if (!dateString) return ''
  const date = new Date(dateString)
  return new Intl.DateTimeFormat('en-US', {
    year: 'numeric',
    month: 'long',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit'
  }).format(date)
}

function getDaysLeft(expiresAt) {
  if (!expiresAt) return 0
  const diff = new Date(expiresAt).getTime() - new Date().getTime()
  return Math.ceil(diff / (1000 * 60 * 60 * 24))
}

onMounted(() => {
  fetchLicenseStatus()
  fetchSoftwareLicenses()
})
</script>

<template>
  <div class="overlay-container" @click.self="$emit('close')">
    <div class="overlay-panel about-panel">
      <!-- Header -->
      <div class="overlay-header">
        <h2 class="overlay-title">About NightAmplifier</h2>
        <button class="btn btn-icon close-btn" @click="$emit('close')" title="Close">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>

      <!-- Tabs -->
      <div class="tabs">
        <button 
          class="tab-btn" 
          :class="{ active: activeTab === 'license' }"
          @click="activeTab = 'license'"
        >
          Pro License
        </button>
        <button 
          class="tab-btn" 
          :class="{ active: activeTab === 'software' }"
          @click="activeTab = 'software'"
        >
          Legal & Software
        </button>
      </div>

      <!-- Content: License -->
      <div v-if="activeTab === 'license'" class="tab-content">
        <div v-if="isLoadingLicense" class="loading-state">
          <div class="spinner"></div>
          <p>Checking license status...</p>
        </div>
        
        <div v-else class="license-info">
          <!-- Active License Display -->
          <div v-if="licenseStatus?.active && licenseStatus?.details" class="active-license-card">
            <div class="status-badge success">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" />
              </svg>
              Pro License Active
            </div>
            
            <div class="license-details">
              <div class="detail-row">
                <span class="detail-label">Licensed to:</span>
                <span class="detail-value">{{ licenseStatus.details.name }}</span>
              </div>
              <div class="detail-row">
                <span class="detail-label">Email:</span>
                <span class="detail-value">{{ licenseStatus.details.email }}</span>
              </div>
              <div class="detail-row">
                <span class="detail-label">Expires:</span>
                <span class="detail-value">
                  {{ formatDate(licenseStatus.details.expires_at) }}
                  <span class="days-left">({{ getDaysLeft(licenseStatus.details.expires_at) }} days left)</span>
                </span>
              </div>
            </div>
          </div>
          
          <!-- No License Display -->
          <div v-else class="no-license-card">
            <div class="status-badge warning">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <circle cx="12" cy="12" r="10"></circle>
                <line x1="12" y1="8" x2="12" y2="12"></line>
                <line x1="12" y1="16" x2="12.01" y2="16"></line>
              </svg>
              Pro License Not Active
            </div>
            <p class="community-text">
              You are currently using the Community Edition. Upgrade to Pro for advanced features like 
              Push-To plate solving, advanced rejection stacking, RBF background extraction, and more.
            </p>
          </div>

          <!-- Update License Form -->
          <div class="update-license-section">
            <h3 class="section-subtitle">Update License Key</h3>
            
            <div v-if="licenseError" class="alert error-alert">
              {{ licenseError }}
            </div>
            
            <div v-if="licenseSuccess" class="alert success-alert">
              License updated successfully! Enjoy NightAmplifier Pro.
            </div>

            <textarea 
              v-model="licenseToken" 
              class="license-input" 
              placeholder="Paste your license here..."
              rows="4"
            ></textarea>
            
            <button 
              class="btn btn-primary update-btn" 
              @click="handleUpdateLicense"
              :disabled="!licenseToken.trim() || isUpdatingLicense"
            >
              <span v-if="isUpdatingLicense" class="btn-spinner"></span>
              <span v-else>Update License</span>
            </button>
          </div>
        </div>
      </div>

      <!-- Content: Software Licenses -->
      <div v-if="activeTab === 'software'" class="tab-content software-tab">
        <div v-if="isLoadingSoftware" class="loading-state">
          <div class="spinner"></div>
          <p>Loading license information...</p>
        </div>
        
        <div v-else class="licenses-container">
          <div class="license-block">
            <h3 class="license-heading">NightAmplifier License</h3>
            <pre class="license-text">{{ coreLicense }}</pre>
          </div>
          
          <div v-if="thirdPartyLicenses" class="license-block">
            <h3 class="license-heading">Third-Party Software</h3>
            <pre class="license-text">{{ thirdPartyLicenses }}</pre>
          </div>
        </div>
      </div>

    </div>
  </div>
</template>

<style scoped>
/* Base overlay styles matching AstapInstallOverlay */
.overlay-container {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.7);
  backdrop-filter: blur(4px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  padding: 1rem;
}

.overlay-panel {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 12px;
  width: 100%;
  max-width: 600px;
  max-height: 90vh;
  display: flex;
  flex-direction: column;
  box-shadow: 0 20px 25px -5px rgba(0, 0, 0, 0.5), 0 10px 10px -5px rgba(0, 0, 0, 0.3);
}

.overlay-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 1.25rem 1.5rem;
  border-bottom: 1px solid var(--border);
  background: var(--surface);
  border-radius: 12px 12px 0 0;
}

.overlay-title {
  margin: 0;
  font-size: 1.25rem;
  font-weight: 600;
  color: var(--text-primary);
}

.close-btn {
  color: var(--text-secondary);
}

.close-btn:hover {
  color: var(--text-primary);
  background: var(--surface-hover);
}

/* Tabs */
.tabs {
  display: flex;
  border-bottom: 1px solid var(--border);
  background: var(--surface);
}

.tab-btn {
  flex: 1;
  padding: 1rem;
  background: transparent;
  border: none;
  border-bottom: 2px solid transparent;
  color: var(--text-secondary);
  font-weight: 600;
  font-size: 0.95rem;
  cursor: pointer;
  transition: all 0.2s;
}

.tab-btn:hover {
  color: var(--text-primary);
  background: var(--surface-hover);
}

.tab-btn.active {
  color: var(--primary);
  border-bottom-color: var(--primary);
  background: rgba(59, 130, 246, 0.1);
}

/* Content Area */
.tab-content {
  padding: 1.5rem;
  overflow-y: auto;
  flex: 1;
}

.software-tab {
  padding: 0; /* Let inner container handle padding so pre block scrolls well */
}

/* Loading State */
.loading-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 3rem;
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

/* License Info Styles */
.active-license-card, .no-license-card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 1.25rem;
  margin-bottom: 1.5rem;
}

.status-badge {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 0.75rem;
  border-radius: 6px;
  font-weight: 600;
  font-size: 0.95rem;
  margin-bottom: 1rem;
}

.status-badge svg {
  width: 18px;
  height: 18px;
}

.status-badge.success {
  background: rgba(16, 185, 129, 0.15);
  color: #10b981;
  border: 1px solid rgba(16, 185, 129, 0.3);
}

.status-badge.warning {
  background: rgba(245, 158, 11, 0.15);
  color: #f59e0b;
  border: 1px solid rgba(245, 158, 11, 0.3);
}

.license-details {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.detail-row {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  align-items: baseline;
}

.detail-label {
  color: var(--text-secondary);
  font-size: 0.9rem;
  width: 90px;
}

.detail-value {
  color: var(--text-primary);
  font-weight: 500;
}

.days-left {
  color: var(--text-muted);
  font-weight: normal;
  font-size: 0.9em;
  margin-left: 0.5rem;
}

.community-text {
  color: var(--text-secondary);
  line-height: 1.5;
  margin: 0;
  font-size: 0.95rem;
}

/* Update License Section */
.update-license-section {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.section-subtitle {
  margin: 0;
  font-size: 1.05rem;
  color: var(--text-primary);
  font-weight: 600;
}

.license-input {
  width: 100%;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 0.75rem;
  color: var(--text-primary);
  font-family: monospace;
  font-size: 0.85rem;
  resize: vertical;
  min-height: 100px;
}

.license-input:focus {
  outline: none;
  border-color: var(--primary);
}

.update-btn {
  align-self: flex-end;
  padding: 0.5rem 1.5rem;
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

/* Alerts */
.alert {
  padding: 0.75rem 1rem;
  border-radius: 6px;
  font-size: 0.9rem;
}

.error-alert {
  background: rgba(239, 68, 68, 0.15);
  color: #ef4444;
  border: 1px solid rgba(239, 68, 68, 0.3);
}

.success-alert {
  background: rgba(16, 185, 129, 0.15);
  color: #10b981;
  border: 1px solid rgba(16, 185, 129, 0.3);
}

/* Software Licenses Text */
.licenses-container {
  display: flex;
  flex-direction: column;
}

.license-block {
  padding: 1.5rem;
  border-bottom: 1px solid var(--border);
}

.license-block:last-child {
  border-bottom: none;
}

.license-heading {
  margin: 0 0 1rem 0;
  color: var(--text-primary);
  font-size: 1.1rem;
}

.license-text {
  margin: 0;
  padding: 1rem;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 6px;
  color: var(--text-secondary);
  font-family: monospace;
  font-size: 0.8rem;
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 400px;
  overflow-y: auto;
}
</style>
