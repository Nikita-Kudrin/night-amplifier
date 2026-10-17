<script setup lang="ts">
import type {DatabaseType} from '../../composables/api.types'

const props = withDefaults(defineProps<{
  databases: readonly DatabaseType[]
  /** The ids of the databases chosen to install. */
  modelValue?: readonly string[]
  hint?: string
}>(), {
  modelValue: () => [],
  hint: '',
})

const emit = defineEmits<{'update:modelValue': [ids: string[]]}>()

function toggleDatabase(dbId: string) {
  const current = [...props.modelValue]
  const index = current.indexOf(dbId)
  if (index >= 0) {
    current.splice(index, 1)
  } else {
    current.push(dbId)
  }
  emit('update:modelValue', current)
}

function isSelected(dbId: string) {
  return props.modelValue.includes(dbId)
}

function formatFovRange(db: DatabaseType) {
  return `${db.min_fov_deg}°–${db.max_fov_deg}°`
}
</script>

<template>
  <div class="database-selector">
    <p v-if="hint" class="section-hint">{{ hint }}</p>

    <div class="database-options">
      <label
          v-for="db in databases"
          :key="db.id"
          class="database-option"
          :class="{ selected: isSelected(db.id), installed: db.installed }"
      >
        <input
            type="checkbox"
            :value="db.id"
            :checked="isSelected(db.id) || db.installed"
            :disabled="db.installed"
            @change="toggleDatabase(db.id)"
        />
        <div class="database-info">
          <span class="database-name">{{ db.description }}</span>
          <span class="database-details">
            <span class="detail-item">{{ formatFovRange(db) }}</span>
            <span class="detail-separator">|</span>
            <span class="detail-item">{{ db.size }}</span>
            <span class="detail-separator">|</span>
            <span class="detail-item database-id">{{ db.id }}</span>
            <span v-if="db.installed" class="installed-badge">Installed</span>
          </span>
        </div>
      </label>
    </div>
  </div>
</template>

<style scoped>
.section-hint {
  font-size: var(--fs-sm);
  color: var(--text-muted);
  margin: 0 0 0.75rem;
}

.database-options {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.database-option {
  display: flex;
  align-items: flex-start;
  gap: 0.75rem;
  padding: 0.75rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition: border-color var(--transition-normal), background var(--transition-normal);
}

.database-option:hover:not(.installed) {
  background: var(--surface-elevated);
}

.database-option.selected {
  border-color: var(--primary);
  background: var(--primary-bg);
}

/* Settled, not chosen: untinted, so it does not read as one more selected row (both tints are red). */
.database-option.installed {
  border-color: var(--success-border);
  background: var(--surface);
  cursor: default;
}

.database-option input {
  margin-top: 0.25rem;
}

.database-info {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  flex: 1;
}

.database-name {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text-primary);
}

.installed .database-name {
  color: var(--text-secondary);
}

.database-details {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: var(--fs-sm);
  color: var(--text-muted);
}

.detail-item {
  white-space: nowrap;
}

.detail-separator {
  color: var(--border);
}

.database-id {
  font-family: monospace;
  font-weight: 500;
  color: var(--text-secondary);
}

.installed-badge {
  color: var(--success);
  font-weight: 600;
  font-size: var(--fs-xs);
  text-transform: uppercase;
}
</style>
