<script setup>
const props = defineProps({
  databases: {
    type: Array,
    required: true,
    // Each database: { id: string, description: string, fov_range: string, size: string }
  },
  modelValue: {
    type: String,
    default: '',
  },
  hint: {
    type: String,
    default: '',
  },
})

const emit = defineEmits(['update:modelValue'])
</script>

<template>
  <div class="database-selector">
    <p v-if="hint" class="section-hint">{{ hint }}</p>

    <div class="database-options">
      <label
          v-for="db in databases"
          :key="db.id"
          class="database-option"
          :class="{ selected: modelValue === db.id }"
      >
        <input
            type="radio"
            :value="db.id"
            :checked="modelValue === db.id"
            name="database"
            @change="emit('update:modelValue', db.id)"
        />
        <div class="database-info">
          <span class="database-name">{{ db.description }}</span>
          <span class="database-details">
            <span class="detail-item">{{ db.fov_range }}</span>
            <span class="detail-separator">|</span>
            <span class="detail-item">{{ db.size }}</span>
            <span class="detail-separator">|</span>
            <span class="detail-item database-id">{{ db.id }}</span>
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

.database-option:hover {
  background: var(--surface-elevated);
}

.database-option.selected {
  border-color: var(--primary);
  background: rgba(74, 158, 255, 0.1);
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
</style>
