<script setup lang="ts">
/** What an installer has on disk: one row per part, ticked when it is there. */
export interface InstallStatusItem {
  label: string
  ok: boolean
}

withDefaults(defineProps<{
  items: readonly InstallStatusItem[]
  size?: 'sm' | 'md'
}>(), {
  size: 'sm',
})
</script>

<template>
  <div class="status-items" :class="`size-${size}`">
    <div v-for="item in items" :key="item.label" class="status-item">
      <span class="status-icon" :class="{ installed: item.ok }">{{ item.ok ? '✓' : '✗' }}</span>
      <span>{{ item.label }}</span>
    </div>
  </div>
</template>

<style scoped>
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
  font-size: var(--fs-md);
  color: var(--text-secondary);
}

.status-icon {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: var(--fs-xs);
  background: var(--error);
  color: var(--on-primary);
}

.status-icon.installed {
  background: var(--success);
}

.size-md {
  margin-bottom: 0;
}

.size-md .status-item {
  font-size: var(--fs-base);
}

.size-md .status-icon {
  width: 20px;
  height: 20px;
  font-size: var(--fs-sm);
}
</style>
