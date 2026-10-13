<script setup>
/**
 * Push-To's object search: the query field, its result list, and Cancel while a solve
 * runs. Emits `select(designation)` for a picked object and `cancel` for the solve.
 *
 * A pick puts the object's name in the field without searching for it again; see
 * `useCatalogSearch` for why that is decided by value rather than by a flag.
 */
import {ref} from 'vue'
import {useCatalogSearch, getCatalogClass, messierLabel} from '../composables/useCatalogSearch.js'
import {useClickOutside} from '../composables/useClickOutside.js'
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

const containerRef = ref(null)
useClickOutside([containerRef], hideResults)
</script>

<template>
  <div ref="containerRef" class="search-container">
    <input
        v-model="searchQuery"
        type="text"
        placeholder="Search Messier, NGC, Stars..."
        class="input search-input"
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

    <div v-if="showResults && searchResults.length > 0" class="dropdown-menu search-results">
      <div
          v-for="entry in searchResults"
          :key="entry.designation"
          class="dropdown-item search-result-item"
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
  padding: 0.5rem;
  font-size: var(--fs-md);
  border-radius: var(--radius-md);
}

.search-spinner-wrapper {
  position: absolute;
  right: 0.5rem;
  top: 50%;
  transform: translateY(-50%);
  display: flex;
  align-items: center;
}

.result-main {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.125rem;
}

.catalog-badge {
  font-size: var(--fs-2xs);
  font-weight: 600;
  padding: 0.125rem 0.375rem;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, currentColor 19%, transparent);
}

.badge-messier {
  color: var(--category-blue);
}

.badge-ngc {
  color: var(--category-orange);
}

.badge-ic {
  color: var(--category-purple);
}

.badge-star {
  color: var(--category-yellow);
}

.badge-other {
  background: var(--surface);
  color: var(--text-secondary);
}

.result-name {
  font-size: var(--fs-sm);
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.result-matched {
  font-size: var(--fs-2xs);
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  margin-bottom: 0.125rem;
}

.result-details {
  display: flex;
  gap: 0.5rem;
  font-size: var(--fs-2xs);
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
  font-size: var(--fs-2xs);
  padding: 0.2rem 0.5rem;
  border-radius: var(--radius-sm);
  cursor: pointer;
  z-index: var(--z-raised);
}

.btn-cancel-solve:hover {
  background: var(--surface-hover);
  color: var(--error);
  border-color: var(--error);
}
</style>
