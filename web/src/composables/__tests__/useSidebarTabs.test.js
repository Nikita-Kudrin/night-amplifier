import {describe, it, expect, vi} from 'vitest'
import {nextTick, ref} from 'vue'
import {flushPromises} from '@vue/test-utils'
import {useSidebarTabs} from '../useSidebarTabs.js'
import {SIDEBAR_TABS} from '../../constants'

const {MAIN, PUSH_TO, SETTINGS} = SIDEBAR_TABS

const PRO = {push_to: {astap_solver: true}}
const COMMUNITY = {push_to: {astap_solver: false}}

function setup({capabilities = COMMUNITY, astap = {ready: true}, catalog = {installed: true}, scroller} = {}) {
    const getAstapStatus = vi.fn(() => (astap instanceof Error ? Promise.reject(astap) : Promise.resolve(astap)))
    const getCatalogStatus = vi.fn(() => Promise.resolve(catalog))
    const tabs = useSidebarTabs({capabilities: ref(capabilities), getAstapStatus, getCatalogStatus, scroller})
    return {...tabs, getAstapStatus, getCatalogStatus}
}

/** A status getter that resolves only when told to. */
function deferred(value) {
    let resolve
    const promise = new Promise((r) => (resolve = () => r(value)))
    return {get: vi.fn(() => promise), resolve}
}

describe('useSidebarTabs', () => {
    it('starts on Main with only Main mounted', () => {
        const {activeTab, visited} = setup()
        expect(activeTab.value).toBe(MAIN)
        expect([...visited]).toEqual([MAIN])
    })

    it('opens Settings, and its button again returns to Main', () => {
        const {activeTab, toggle} = setup()

        toggle(SETTINGS)
        expect(activeTab.value).toBe(SETTINGS)

        toggle(SETTINGS)
        expect(activeTab.value).toBe(MAIN)
    })

    it('switches between Push-To and Settings directly, never showing both', async () => {
        const {activeTab, toggle} = setup()

        await toggle(PUSH_TO)
        expect(activeTab.value).toBe(PUSH_TO)
        await toggle(SETTINGS)
        expect(activeTab.value).toBe(SETTINGS)
        await toggle(PUSH_TO)
        expect(activeTab.value).toBe(PUSH_TO)
        await toggle(PUSH_TO)
        expect(activeTab.value).toBe(MAIN)
    })

    it('keeps every visited tab mounted', async () => {
        const {visited, toggle} = setup()

        toggle(SETTINGS)
        toggle(SETTINGS)
        expect([...visited].sort()).toEqual([MAIN, SETTINGS].sort())

        await toggle(PUSH_TO)
        toggle(SETTINGS)
        toggle(SETTINGS)
        expect([...visited].sort()).toEqual([MAIN, PUSH_TO, SETTINGS].sort())
    })

    // Checking would start ASTAP/catalog work a Community install can never use.
    it('opens Push-To on Community without checking the solver setup', async () => {
        const {activeTab, toggle, getAstapStatus, getCatalogStatus} = setup({capabilities: COMMUNITY})

        await toggle(PUSH_TO)

        expect(activeTab.value).toBe(PUSH_TO)
        expect(getAstapStatus).not.toHaveBeenCalled()
        expect(getCatalogStatus).not.toHaveBeenCalled()
    })

    it('opens Push-To on Pro once ASTAP and the catalog are installed', async () => {
        const {activeTab, showPushToSetup, checkingSetup, toggle} = setup({capabilities: PRO})

        const opening = toggle(PUSH_TO)
        expect(checkingSetup.value).toBe(true)
        await opening

        expect(checkingSetup.value).toBe(false)
        expect(activeTab.value).toBe(PUSH_TO)
        expect(showPushToSetup.value).toBe(false)
    })

    it.each([
        ['ASTAP is missing', {astap: {ready: false}}],
        ['the catalog is missing', {catalog: {installed: false}}],
        ['the check fails', {astap: new Error('offline')}],
    ])('shows the setup overlay and stays put when %s', async (_, missing) => {
        const {activeTab, visited, showPushToSetup, checkingSetup, toggle} = setup({capabilities: PRO, ...missing})

        toggle(SETTINGS)
        await toggle(PUSH_TO)

        expect(showPushToSetup.value).toBe(true)
        expect(checkingSetup.value).toBe(false)
        expect(activeTab.value).toBe(SETTINGS)
        expect(visited.has(PUSH_TO)).toBe(false)
    })

    it('opens Push-To once setup completes, and closing the overlay leaves the tab', async () => {
        const {activeTab, showPushToSetup, toggle, setupComplete, closeSetup} = setup({
            capabilities: PRO,
            astap: {ready: false},
        })

        await toggle(PUSH_TO)
        closeSetup()
        expect(showPushToSetup.value).toBe(false)
        expect(activeTab.value).toBe(MAIN)

        await toggle(PUSH_TO)
        setupComplete()
        expect(showPushToSetup.value).toBe(false)
        expect(activeTab.value).toBe(PUSH_TO)
    })

    it('does not pull the observer back to Push-To when they moved on during the check', async () => {
        const astap = deferred({ready: true})
        const tabs = useSidebarTabs({
            capabilities: ref(PRO),
            getAstapStatus: astap.get,
            getCatalogStatus: () => Promise.resolve({installed: true}),
        })

        const opening = tabs.toggle(PUSH_TO)
        tabs.toggle(SETTINGS)
        astap.resolve()
        await opening

        expect(tabs.activeTab.value).toBe(SETTINGS)
        expect(tabs.showPushToSetup.value).toBe(false)
        expect(tabs.checkingSetup.value).toBe(false)
    })

    it('restores each tab its own scroll position', async () => {
        const scroller = ref({scrollTop: 0})
        const {toggle} = setup({scroller})

        scroller.value.scrollTop = 240
        toggle(SETTINGS)
        await flushPromises()
        expect(scroller.value.scrollTop).toBe(0)

        scroller.value.scrollTop = 90
        toggle(SETTINGS)
        await flushPromises()
        expect(scroller.value.scrollTop).toBe(240)

        toggle(SETTINGS)
        await nextTick()
        await flushPromises()
        expect(scroller.value.scrollTop).toBe(90)
    })
})
