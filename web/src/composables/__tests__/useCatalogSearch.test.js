import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {getCatalogClass, messierLabel, useCatalogSearch as originalUseCatalogSearch} from '../useCatalogSearch.js'
import { mount } from '@vue/test-utils'

let currentApp = null;
function useCatalogSearch() {
    let result;
    currentApp = mount({
        setup() {
            result = originalUseCatalogSearch()
            return () => {}
        }
    })
    return result
}

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

describe('messierLabel', () => {
    it('labels an object whose designation is from another catalog', () => {
        expect(messierLabel({designation: 'NGC 6121', messier: 'M4'})).toBe('M4')
    })

    it('adds nothing when the designation is already the Messier number', () => {
        expect(messierLabel({designation: 'M 40', messier: 'M40'})).toBeNull()
    })

    it('adds nothing for objects outside the Messier catalog', () => {
        expect(messierLabel({designation: 'NGC 6302'})).toBeNull()
    })
})

describe('useCatalogSearch', () => {
    beforeEach(() => {
        vi.useFakeTimers()
        vi.clearAllMocks()
    })

    afterEach(() => {
        vi.useRealTimers()
        if (currentApp) {
            currentApp.unmount()
            currentApp = null
        }
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

    it('searches a two-character query typed right after a one-character one', async () => {
        searchCatalog.mockResolvedValue([{designation: 'NGC 6121', messier: 'M4'}])

        const {searchQuery, searchResults} = useCatalogSearch()
        searchQuery.value = 'M'
        await vi.runAllTimersAsync()
        searchQuery.value = 'M4'
        await vi.runAllTimersAsync()

        expect(searchCatalog).toHaveBeenCalledTimes(1)
        expect(searchCatalog).toHaveBeenCalledWith('M4')
        expect(searchResults.value).toEqual([{designation: 'NGC 6121', messier: 'M4'}])
    })

    it('searches a two-character query typed into a cleared field', async () => {
        searchCatalog.mockResolvedValue([])

        const {searchQuery} = useCatalogSearch()
        searchQuery.value = 'M31'
        await vi.runAllTimersAsync()
        searchQuery.value = ''
        await vi.runAllTimersAsync()
        searchQuery.value = 'M4'
        await vi.runAllTimersAsync()

        expect(searchCatalog).toHaveBeenLastCalledWith('M4')
        expect(searchCatalog).toHaveBeenCalledTimes(2)
    })

    it('setQueryWithoutSearch shows the text without searching, and the next edit searches', async () => {
        searchCatalog.mockResolvedValue([])

        const {searchQuery, setQueryWithoutSearch} = useCatalogSearch()
        setQueryWithoutSearch('NGC 6121')
        await vi.runAllTimersAsync()

        expect(searchQuery.value).toBe('NGC 6121')
        expect(searchCatalog).not.toHaveBeenCalled()

        searchQuery.value = 'NGC 612'
        await vi.runAllTimersAsync()
        expect(searchCatalog).toHaveBeenCalledWith('NGC 612')
    })

    it('setQueryWithoutSearch with the text already in the box does not swallow the next edit', async () => {
        searchCatalog.mockResolvedValue([{designation: 'Mel 22', name: 'Pleiades'}])

        const {searchQuery, setQueryWithoutSearch} = useCatalogSearch()
        searchQuery.value = 'Pleiades'
        await vi.runAllTimersAsync()
        expect(searchCatalog).toHaveBeenCalledTimes(1)

        // Same value: Vue never runs the watcher for this assignment
        setQueryWithoutSearch('Pleiades')
        await vi.runAllTimersAsync()

        searchQuery.value = 'Pleiade'
        await vi.runAllTimersAsync()
        expect(searchCatalog).toHaveBeenCalledTimes(2)
        expect(searchCatalog).toHaveBeenLastCalledWith('Pleiade')
    })

    it('drops a response that arrives after the search was cleared', async () => {
        let resolveSearch
        searchCatalog.mockImplementation(() => new Promise((resolve) => {
            resolveSearch = resolve
        }))

        const {searchQuery, searchResults, showResults, searching, setQueryWithoutSearch} = useCatalogSearch()
        searchQuery.value = 'M31'
        await vi.runAllTimersAsync()
        expect(searching.value).toBe(true)

        setQueryWithoutSearch('Andromeda Galaxy')
        resolveSearch([{designation: 'NGC 224'}])
        await vi.runAllTimersAsync()

        expect(searchResults.value).toEqual([])
        expect(showResults.value).toBe(false)
        expect(searching.value).toBe(false)
    })

    it('drops an older response that arrives after a newer one', async () => {
        const pending = []
        searchCatalog.mockImplementation(() => new Promise((resolve) => pending.push(resolve)))

        const {searchQuery, searchResults} = useCatalogSearch()
        searchQuery.value = 'M3'
        await vi.runAllTimersAsync()
        searchQuery.value = 'M31'
        await vi.runAllTimersAsync()

        pending[1]([{designation: 'NGC 224'}])
        await vi.runAllTimersAsync()
        pending[0]([{designation: 'NGC 5272'}])
        await vi.runAllTimersAsync()

        expect(searchResults.value).toEqual([{designation: 'NGC 224'}])
    })

    it('picking a target while a newer query is still debouncing sends nothing', async () => {
        searchCatalog.mockResolvedValue([])

        const {searchQuery, setQueryWithoutSearch} = useCatalogSearch()
        searchQuery.value = 'M31'
        vi.advanceTimersByTime(100)
        setQueryWithoutSearch('Andromeda Galaxy')
        await vi.runAllTimersAsync()

        expect(searchCatalog).not.toHaveBeenCalled()
    })

    it('a suppression armed by a same-value pick does not skip that value later', async () => {
        searchCatalog.mockResolvedValue([])

        const {searchQuery, setQueryWithoutSearch} = useCatalogSearch()
        searchQuery.value = 'Pleiades'
        await vi.runAllTimersAsync()
        // Same value: the watcher never runs, so the suppression stays armed
        setQueryWithoutSearch('Pleiades')
        searchQuery.value = ''
        await vi.runAllTimersAsync()
        searchQuery.value = 'Pleiades'
        await vi.runAllTimersAsync()

        expect(searchCatalog.mock.calls.map(([query]) => query)).toEqual(['Pleiades', 'Pleiades'])
    })

    it('an error from a superseded request does not clear newer results', async () => {
        const pending = []
        searchCatalog.mockImplementation(() => new Promise((resolve, reject) => pending.push({resolve, reject})))

        const {searchQuery, searchResults, searching} = useCatalogSearch()
        searchQuery.value = 'M3'
        await vi.runAllTimersAsync()
        searchQuery.value = 'M31'
        await vi.runAllTimersAsync()

        pending[1].resolve([{designation: 'NGC 224'}])
        await vi.runAllTimersAsync()
        pending[0].reject(new Error('late failure'))
        await vi.runAllTimersAsync()

        expect(searchResults.value).toEqual([{designation: 'NGC 224'}])
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
