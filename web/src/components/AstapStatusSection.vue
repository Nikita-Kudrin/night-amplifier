<script setup lang="ts">
import {computed} from 'vue'
import InstallStatusList from './ui/InstallStatusList.vue'
import type {AstapStatus} from '../composables/api.types.js'

const props = defineProps<{
  status: AstapStatus
}>()

const items = computed(() => {
  const databases = props.status.installed_databases
  return [
    {label: 'ASTAP CLI', ok: props.status.binary_installed},
    ...(databases.length
        ? databases.map((db) => ({label: `${db.id} Database`, ok: true}))
        : [{label: 'No Star Database', ok: false}]),
  ]
})
</script>

<template>
  <div class="status-section">
    <h3 class="eyebrow">Current Status</h3>
    <InstallStatusList :items="items" size="md"/>
  </div>
</template>

<style scoped>
.status-section {
  margin-bottom: 1.25rem;
}

.eyebrow {
  margin-bottom: 0.75rem;
}
</style>
