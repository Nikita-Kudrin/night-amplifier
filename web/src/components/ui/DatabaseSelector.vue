<script setup>
const props = defineProps({
  databases: {
    type: Array,
    required: true,
    // Each database: { id: string, description: string, fov_range: string, size: string, installed: boolean }
  },
  modelValue: {
    type: Array,
    default: () => [],
  },
  hint: {
    type: String,
    default: '',
  },
})

const emit = defineEmits(['update:modelValue'])

function toggleDatabase(dbId) {
  const current = [...props.modelValue]
  const index = current.indexOf(dbId)
  if (index >= 0) {
    current.splice(index, 1)
  } else {
    current.push(dbId)
  }
  emit('update:modelValue', current)
}

function isSelected(dbId) {
  return props.modelValue.includes(dbId)
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
            <span class="detail-item">{{ db.fov_range }}</span>
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
  font-size: 0.75rem;
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
  border-radius: 8px;
  cursor: pointer;
  transition: border-color 0.2s, background 0.2s;
}

.database-option:hover:not(.installed) {
  background: var(--surface-elevated);
}

.database-option.selected {
  border-color: var(--primary);
  background: rgba(74, 158, 255, 0.1);
}

.database-option.installed {
  border-color: var(--success, #22c55e);
  background: rgba(34, 197, 94, 0.05);
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
  font-size: 0.875rem;
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
  font-size: 0.75rem;
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
  color: var(--success, #22c55e);
  font-weight: 600;
  font-size: 0.7rem;
  text-transform: uppercase;
}
</style>
