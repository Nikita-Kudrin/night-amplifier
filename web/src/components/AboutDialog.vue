<script setup>
import {ref, onMounted} from 'vue'
import {getLicenseStatus, updateLicense} from '../composables/api.js'
import {useError} from '../composables/useError.js'
import {useSoftwareLicenses} from '../composables/useSoftwareLicenses.js'
import {BaseAlert, BaseModal, BaseSpinner, ButtonGroup} from './ui'
import LicenseTexts from './LicenseTexts.vue'

defineEmits(['close'])

const TABS = [
  {value: 'license', label: 'Pro License'},
  {value: 'software', label: 'Legal & Software'},
]
const activeTab = ref('license')

const licenseStatus = ref(null)
const licenseToken = ref('')
const isLoadingLicense = ref(true)
const licenseSuccess = ref(false)
const {error: licenseError, loading: isUpdatingLicense, withErrorHandling} = useError()

const {
  version,
  coreLicense,
  thirdPartyLicenses,
  loading: isLoadingSoftware,
  error: softwareError,
  load: loadSoftware,
} = useSoftwareLicenses()

async function fetchLicenseStatus() {
  await withErrorHandling(async () => {
    licenseStatus.value = await getLicenseStatus()
  })
  isLoadingLicense.value = false
}

async function handleUpdateLicense() {
  const token = licenseToken.value.trim()
  if (!token) return
  licenseSuccess.value = false
  await withErrorHandling(async () => {
    licenseStatus.value = await updateLicense(token)
    licenseSuccess.value = true
    licenseToken.value = ''
  })
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

onMounted(fetchLicenseStatus)
</script>

<template>
  <BaseModal 
    title="About NightAmplifier" 
    max-width="600px" 
    no-padding
    @close="$emit('close')"
  >
    <template #title-extra>
      <span v-if="version" class="version-badge">v{{ version }}</span>
    </template>

    <template #default>
      <!-- Tabs -->
      <ButtonGroup v-model="activeTab" class="tabs" :options="TABS"/>

      <!-- Content: License -->
      <div v-if="activeTab === 'license'" class="tab-content">
        <div v-if="isLoadingLicense" class="loading-state">
          <BaseSpinner size="md" />
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
              advanced stacking, better background extraction, Push-To plate solving and more.
            </p>
          </div>

          <!-- Update License Form -->
          <div class="update-license-section">
            <h3 class="section-subtitle">Update License Key</h3>
            
            <BaseAlert v-if="licenseError" bordered :dismissible="false">
              {{ licenseError }}
            </BaseAlert>

            <BaseAlert v-if="licenseSuccess" type="success" bordered :dismissible="false">
              License updated successfully! Enjoy NightAmplifier Pro.
            </BaseAlert>

            <textarea 
              v-model="licenseToken"
              class="input license-input"
              placeholder="Paste your license here..."
              rows="4"
            ></textarea>
            
            <button 
              class="btn btn-primary update-btn" 
              :disabled="!licenseToken.trim() || isUpdatingLicense"
              @click="handleUpdateLicense"
            >
              <BaseSpinner v-if="isUpdatingLicense" size="sm" light />
              <span v-else>Update License</span>
            </button>
          </div>
        </div>
      </div>

      <!-- Content: Software Licenses -->
      <div v-if="activeTab === 'software'" class="tab-content software-tab">
        <div v-if="isLoadingSoftware" class="loading-state">
          <BaseSpinner size="md" />
          <p>Loading license information...</p>
        </div>

        <template v-else-if="softwareError">
          <BaseAlert bordered :dismissible="false">
            Could not load the license information: {{ softwareError }}
          </BaseAlert>
          <button class="btn btn-primary retry-btn" @click="loadSoftware">Retry</button>
        </template>

        <LicenseTexts
            v-else
            :core="coreLicense"
            :third-party="thirdPartyLicenses"
            id-prefix="about"
        />
      </div>
    </template>
  </BaseModal>
</template>

<style scoped>
.version-badge {
  font-size: var(--fs-base);
  font-weight: normal;
  color: var(--text-muted);
  background: var(--surface-hover);
  padding: 0.25rem 0.5rem;
  border-radius: var(--radius-md);
  margin-left: 0.75rem;
}

/* The two tabs: a segmented control at tab size. */
.tabs {
  padding: 0.75rem 1.5rem 0;
}

.tabs :deep(.btn-option) {
  padding: 0.5rem;
  font-size: var(--fs-lg);
  font-weight: 600;
}

/* Content Area */
.tab-content {
  padding: 1.5rem;
  overflow-y: auto;
  flex: 1;
}

.retry-btn {
  margin-top: 1rem;
}

/* License Info Styles */
.active-license-card, .no-license-card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: 1.25rem;
  margin-bottom: 1.5rem;
}

.status-badge {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 0.75rem;
  border-radius: var(--radius-md);
  font-weight: 600;
  font-size: var(--fs-lg);
  margin-bottom: 1rem;
}

.status-badge svg {
  width: 18px;
  height: 18px;
}

.status-badge.success {
  background: var(--success-bg);
  color: var(--success);
  border: 1px solid color-mix(in srgb, currentColor 30%, transparent);
}

.status-badge.warning {
  background: var(--warning-bg);
  color: var(--warning);
  border: 1px solid color-mix(in srgb, currentColor 30%, transparent);
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
  font-size: var(--fs-base);
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
  font-size: var(--fs-lg);
}

/* Update License Section */
.update-license-section {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.section-subtitle {
  margin: 0;
  font-size: var(--fs-xl);
  color: var(--text-primary);
  font-weight: 600;
}

.license-input {
  padding: 0.75rem;
  font-family: monospace;
  font-size: var(--fs-base);
  resize: vertical;
  min-height: 100px;
  background: var(--surface);
}

.update-btn {
  align-self: flex-end;
  padding: 0.5rem 1.5rem;
}
</style>
