<script setup>
/**
 * Push-To's object search: the query field, its result list, and Cancel while a solve
 * runs. Emits `select(designation)` for a picked object and `cancel` for the solve.
 *
 * A pick puts the object's name in the field without searching for it again; see
 * `useCatalogSearch` for why that is decided by value rather than by a flag.
 */
import {onMounted, onUnmounted} from 'vue'
import {useCatalogSearch, getCatalogClass, messierLabel} from '../composables/useCatalogSearch.js'
import {BaseSpinner} from './ui'

defineProps({
  /** A solve is running: the field is locked and Cancel is offered. */
  solving: {type: Boolean, default: false},
})

const emit = defineEmits(['select', 'cancel'])

const {searchQuery, searchResults, searching, showResults, setQueryWithoutSearch, hideResults, revealResults} =
    useCatalogSearch()

function select(entry) {
  setQueryWithoutSearch(entry.name || entry.designation)
  emit('select', entry.designation)
}

function handleClickOutside(event) {
  if (!event.target.closest('.search-container')) {
    hideResults()
  }
}

onMounted(() => document.addEventListener('click', handleClickOutside))
onUnmounted(() => document.removeEventListener('click', handleClickOutside))
</script>

<template>
  <div class="search-container">
    <input
        v-model="searchQuery"
        type="text"
        placeholder="Search Messier, NGC, Stars..."
        class="search-input"
        :disabled="solving"
        @focus="revealResults"
    />
    <div v-if="searching || solving" class="search-spinner-wrapper">
      <BaseSpinner size="sm" />
    </div>
    <button
        v-if="solving"
        class="btn-cancel-solve"
        title="Cancel solving"
        @click="emit('cancel')"
    >
      Cancel
    </button>

    <div v-if="showResults && searchResults.length > 0" class="search-results">
      <div
          v-for="entry in searchResults"
          :key="entry.designation"
          class="search-result-item"
          @click="select(entry)"
      >
        <div class="result-main">
          <span v-if="messierLabel(entry)" class="catalog-badge badge-messier">
            {{ messierLabel(entry) }}
          </span>
          <span :class="['catalog-badge', getCatalogClass(entry.catalog_type)]">
            {{ entry.designation }}
          </span>
          <span class="result-name">{{ entry.name }}</span>
        </div>
        <div v-if="entry.matched_name" class="result-matched">Matched: {{ entry.matched_name }}</div>
        <div class="result-details">
          <span class="result-type">{{ entry.object_type }}</span>
          <span class="result-constellation">{{ entry.constellation }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.search-container {
  position: relative;
}

.search-input {
  width: 100%;
  background: var(--surface-elevated);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 0.5rem;
  font-size: 0.8rem;
  color: var(--text-primary);
}

.search-input:focus {
  outline: none;
  border-color: var(--primary);
}

.search-input::placeholder {
  color: var(--text-muted);
}

.search-spinner-wrapper {
  position: absolute;
  right: 0.5rem;
  top: 50%;
  transform: translateY(-50%);
  display: flex;
  align-items: center;
}

.search-results {
  position: absolute;
  top: 100%;
  left: 0;
  right: 0;
  background: var(--surface-elevated);
  border: 1px solid var(--border);
  border-radius: 6px;
  margin-top: 0.25rem;
  max-height: 200px;
  overflow-y: auto;
  z-index: 100;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
}

.search-result-item {
  padding: 0.5rem;
  cursor: pointer;
  border-bottom: 1px solid var(--border);
}

.search-result-item:last-child {
  border-bottom: none;
}

.search-result-item:hover {
  background: var(--surface-hover);
}

.result-main {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.125rem;
}

.catalog-badge {
  font-size: 0.65rem;
  font-weight: 600;
  padding: 0.125rem 0.375rem;
  border-radius: 4px;
}

.badge-messier {
  background: #4a9eff30;
  color: #4a9eff;
}

.badge-ngc {
  background: #ff9f4a30;
  color: #ff9f4a;
}

.badge-ic {
  background: #9f4aff30;
  color: #9f4aff;
}

.badge-star {
  background: #facc1530;
  color: #facc15;
}

.badge-other {
  background: var(--surface);
  color: var(--text-secondary);
}

.result-name {
  font-size: 0.75rem;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.result-matched {
  font-size: 0.65rem;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  margin-bottom: 0.125rem;
}

.result-details {
  display: flex;
  gap: 0.5rem;
  font-size: 0.65rem;
  color: var(--text-muted);
}

.btn-cancel-solve {
  position: absolute;
  right: 2rem;
  top: 50%;
  transform: translateY(-50%);
  background: var(--surface);
  border: 1px solid var(--border);
  color: var(--text-muted);
  font-size: 0.65rem;
  padding: 0.2rem 0.5rem;
  border-radius: 4px;
  cursor: pointer;
  z-index: 5;
}

.btn-cancel-solve:hover {
  background: var(--surface-hover);
  color: var(--danger);
  border-color: var(--danger);
}
</style>
