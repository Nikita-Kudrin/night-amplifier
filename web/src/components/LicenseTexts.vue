<script setup lang="ts">
/** The core licence and, when there is one, the third-party notices, each in its own scroll box. */
withDefaults(defineProps<{
  core: string
  thirdParty?: string
  coreTitle?: string
  /** Prefixes the two texts' ids (`<prefix>-core-license`, `<prefix>-third-party`). */
  idPrefix?: string
}>(), {
  thirdParty: '',
  coreTitle: 'NightAmplifier License',
  idPrefix: 'license',
})
</script>

<template>
  <div class="licenses-container">
    <div class="license-block">
      <h3 class="license-heading">{{ coreTitle }}</h3>
      <pre :id="`${idPrefix}-core-license`" class="license-text">{{ core }}</pre>
    </div>

    <div v-if="thirdParty" class="license-block">
      <h3 class="license-heading">Third-Party Software</h3>
      <pre :id="`${idPrefix}-third-party`" class="license-text">{{ thirdParty }}</pre>
    </div>
  </div>
</template>

<style scoped>
.licenses-container {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.license-heading {
  margin: 0 0 0.5rem;
  font-size: var(--fs-lg);
  font-weight: 600;
  color: var(--text-primary);
}

.license-text {
  margin: 0;
  padding: 1rem;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  color: var(--text-secondary);
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: var(--license-text-height);
  overflow-y: auto;
  line-height: 1.6;
}

.licenses-container {
  --license-text-height: 250px;
}

@media (max-width: 768px) {
  .licenses-container {
    --license-text-height: 180px;
  }
}
</style>
