import {ref, watch, onUnmounted} from 'vue'
import {searchCatalog} from './api.js'

const DEBOUNCE_DELAY_MS = 300
const MIN_QUERY_LENGTH = 2

/**
 * Get CSS class for catalog type badge
 * @param {string | undefined} type - Catalog type
 * @returns {string} CSS class name
 */
export function getCatalogClass(type) {
    switch (type?.toLowerCase()) {
        case 'messier':
            return 'badge-messier'
        case 'ngc':
            return 'badge-ngc'
        case 'ic':
            return 'badge-ic'
        case 'star':
            return 'badge-star'
        default:
            return 'badge-other'
    }
}

/**
 * Composable for catalog search with debouncing
 * @returns Reactive search state and methods
 */
export function useCatalogSearch() {
    const searchQuery = ref('')
    const searchResults = ref([])
    const searching = ref(false)
    const showResults = ref(false)

    let searchTimer = null
    let skipNextSearch = false

    function clearSearch() {
        if (searchTimer) {
            clearTimeout(searchTimer)
            searchTimer = null
        }
        skipNextSearch = true
        searchResults.value = []
        showResults.value = false
    }

    function hideResults() {
        showResults.value = false
    }

    function revealResults() {
        if (searchResults.value.length > 0) {
            showResults.value = true
        }
    }

    watch(searchQuery, (query) => {
        if (searchTimer) {
            clearTimeout(searchTimer)
        }

        if (skipNextSearch) {
            skipNextSearch = false
            return
        }

        if (query.length < MIN_QUERY_LENGTH) {
            clearSearch()
            return
        }

        searchTimer = setTimeout(async () => {
            searching.value = true
            try {
                searchResults.value = await searchCatalog(query)
                showResults.value = searchResults.value.length > 0
            } catch {
                searchResults.value = []
            } finally {
                searching.value = false
            }
        }, DEBOUNCE_DELAY_MS)
    })

    onUnmounted(() => {
        if (searchTimer) {
            clearTimeout(searchTimer)
        }
    })

    return {
        searchQuery,
        searchResults,
        searching,
        showResults,
        clearSearch,
        hideResults,
        revealResults,
    }
}
