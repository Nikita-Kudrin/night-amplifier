import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {getCatalogClass, useCatalogSearch} from '../useCatalogSearch.js'

vi.mock('../api.js', () => ({
    searchCatalog: vi.fn(),
}))

import {searchCatalog} from '../api.js'

describe('getCatalogClass', () => {
    it('returns badge-messier for Messier objects', () => {
        expect(getCatalogClass('messier')).toBe('badge-messier')
        expect(getCatalogClass('Messier')).toBe('badge-messier')
        expect(getCatalogClass('MESSIER')).toBe('badge-messier')
    })

    it('returns badge-ngc for NGC objects', () => {
        expect(getCatalogClass('ngc')).toBe('badge-ngc')
        expect(getCatalogClass('NGC')).toBe('badge-ngc')
    })

    it('returns badge-ic for IC objects', () => {
        expect(getCatalogClass('ic')).toBe('badge-ic')
        expect(getCatalogClass('IC')).toBe('badge-ic')
    })

    it('returns badge-other for unknown types', () => {
        expect(getCatalogClass('unknown')).toBe('badge-other')
        expect(getCatalogClass(undefined)).toBe('badge-other')
        expect(getCatalogClass(null)).toBe('badge-other')
    })
})

describe('useCatalogSearch', () => {
    beforeEach(() => {
        vi.useFakeTimers()
        vi.clearAllMocks()
    })

    afterEach(() => {
        vi.useRealTimers()
    })

    it('provides reactive search state', () => {
        const {searchQuery, searchResults, searching, showResults} = useCatalogSearch()
        expect(searchQuery.value).toBe('')
        expect(searchResults.value).toEqual([])
        expect(searching.value).toBe(false)
        expect(showResults.value).toBe(false)
    })

    it('does not search for queries shorter than 2 characters', async () => {
        const {searchQuery} = useCatalogSearch()
        searchQuery.value = 'M'
        await vi.runAllTimersAsync()
        expect(searchCatalog).not.toHaveBeenCalled()
    })

    it('searches after debounce delay', async () => {
        const mockResults = [{designation: 'M31', name: 'Andromeda Galaxy'}]
        searchCatalog.mockResolvedValue(mockResults)

        const {searchQuery, searchResults, showResults} = useCatalogSearch()
        searchQuery.value = 'M31'

        // Should not have searched yet
        expect(searchCatalog).not.toHaveBeenCalled()

        // Fast forward past debounce delay
        await vi.runAllTimersAsync()

        expect(searchCatalog).toHaveBeenCalledWith('M31')
        expect(searchResults.value).toEqual(mockResults)
        expect(showResults.value).toBe(true)
    })

    it('sets searching to true during search', async () => {
        let resolveSearch
        searchCatalog.mockImplementation(
            () => new Promise((resolve) => {
                resolveSearch = resolve
            })
        )

        const {searchQuery, searching} = useCatalogSearch()
        searchQuery.value = 'M42'

        await vi.runAllTimersAsync()

        expect(searching.value).toBe(true)

        resolveSearch([])
        await vi.runAllTimersAsync()

        expect(searching.value).toBe(false)
    })

    it('clears results when clearSearch is called', async () => {
        const mockResults = [{designation: 'M31'}]
        searchCatalog.mockResolvedValue(mockResults)

        const {searchQuery, searchResults, showResults, clearSearch} = useCatalogSearch()
        searchQuery.value = 'M31'
        await vi.runAllTimersAsync()

        expect(searchResults.value).toEqual(mockResults)
        expect(showResults.value).toBe(true)

        clearSearch()

        expect(searchResults.value).toEqual([])
        expect(showResults.value).toBe(false)
    })

    it('hideResults sets showResults to false', () => {
        const {showResults, hideResults} = useCatalogSearch()
        showResults.value = true
        hideResults()
        expect(showResults.value).toBe(false)
    })

    it('revealResults shows results if there are any', async () => {
        const mockResults = [{designation: 'M31'}]
        searchCatalog.mockResolvedValue(mockResults)

        const {searchQuery, showResults, hideResults, revealResults} = useCatalogSearch()
        searchQuery.value = 'M31'
        await vi.runAllTimersAsync()

        hideResults()
        expect(showResults.value).toBe(false)

        revealResults()
        expect(showResults.value).toBe(true)
    })

    it('revealResults does nothing if no results', () => {
        const {showResults, revealResults} = useCatalogSearch()
        revealResults()
        expect(showResults.value).toBe(false)
    })

    it('handles search errors gracefully', async () => {
        searchCatalog.mockRejectedValue(new Error('Network error'))

        const {searchQuery, searchResults, searching} = useCatalogSearch()
        searchQuery.value = 'M31'
        await vi.runAllTimersAsync()

        expect(searchResults.value).toEqual([])
        expect(searching.value).toBe(false)
    })

    it('cancels pending search when query changes', async () => {
        searchCatalog.mockResolvedValue([])

        const {searchQuery} = useCatalogSearch()
        searchQuery.value = 'M31'

        // Change query before debounce completes
        vi.advanceTimersByTime(100)
        searchQuery.value = 'M42'
        await vi.runAllTimersAsync()

        // Should only have searched once for M42
        expect(searchCatalog).toHaveBeenCalledTimes(1)
        expect(searchCatalog).toHaveBeenCalledWith('M42')
    })
})
