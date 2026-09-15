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
 * Messier label to show beside a result, or null when the designation already says it (M 40)
 * @param {{messier?: string, designation: string}} entry - Catalog entry
 * @returns {string | null}
 */
export function messierLabel(entry) {
    if (!entry?.messier) return null
    const compact = (text) => text.replace(/\s/g, '').toLowerCase()
    return compact(entry.messier) === compact(entry.designation) ? null : entry.messier
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
    // Matched by value, not a one-shot flag: a flag armed by an assignment that changed nothing
    // never reaches the watcher, and then swallows the user's next query (typing "M" then "M4").
    let suppressedQuery = null
    // Responses from a superseded or cleared search are dropped, so a slow reply can't
    // reopen the dropdown after a target was picked or overwrite a newer query's results.
    let latestRequest = 0

    function clearSearch() {
        if (searchTimer) {
            clearTimeout(searchTimer)
            searchTimer = null
        }
        latestRequest++
        searching.value = false
        searchResults.value = []
        showResults.value = false
    }

    /** Show text in the search box (e.g. the picked target) without searching for it */
    function setQueryWithoutSearch(text) {
        clearSearch()
        suppressedQuery = text
        searchQuery.value = text
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
        const suppressed = suppressedQuery
        suppressedQuery = null
        if (searchTimer) {
            clearTimeout(searchTimer)
            searchTimer = null
        }

        if (query === suppressed) {
            return
        }

        if (query.length < MIN_QUERY_LENGTH) {
            clearSearch()
            return
        }

        searchTimer = setTimeout(async () => {
            searchTimer = null
            const request = ++latestRequest
            searching.value = true
            try {
                const results = await searchCatalog(query)
                if (request !== latestRequest) return
                searchResults.value = results
                showResults.value = results.length > 0
            } catch {
                if (request === latestRequest) searchResults.value = []
            } finally {
                if (request === latestRequest) searching.value = false
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
        setQueryWithoutSearch,
        hideResults,
        revealResults,
    }
}
