<script setup>
import {computed} from 'vue'
import InstallStatusList from './ui/InstallStatusList.vue'

const props = defineProps({
  status: {
    type: Object,
    required: true,
    // { binary_installed: boolean, database_installed: boolean, installed_databases: Array<{id, database_path}> }
  },
})

const items = computed(() => {
  const databases = props.status?.installed_databases ?? []
  return [
    {label: 'ASTAP CLI', ok: Boolean(props.status?.binary_installed)},
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
