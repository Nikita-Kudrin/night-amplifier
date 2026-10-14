import {nextTick, reactive, ref, watch} from 'vue'
import type {Ref} from 'vue'
import {SIDEBAR_TABS} from '../constants'
import type {AppContext} from './appContext'
import type {AstapStatus, CatalogStatus} from './api.types'

/** One of the sidebar's tabs. */
export type SidebarTab = (typeof SIDEBAR_TABS)[keyof typeof SIDEBAR_TABS]

export interface SidebarTabsOptions {
    /** `/api/capabilities` */
    capabilities: AppContext['capabilities']
    getAstapStatus: () => Promise<Pick<AstapStatus, 'ready'>>
    getCatalogStatus: () => Promise<Pick<CatalogStatus, 'installed'>>
    /** The sidebar, whose scroll position is kept per tab. */
    scroller?: Readonly<Ref<Pick<HTMLElement, 'scrollTop'> | null>> | null
}

/**
 * Which sidebar tab shows: Main (camera + capture), Push-To or Settings, one at a time.
 *
 * A tab is mounted on its first visit and only hidden afterwards (`visited`): remounting
 * dropped typed coordinates and refetched on every switch. Pro's Push-To first checks
 * ASTAP and the catalog, and opens the setup overlay instead when either is missing.
 */
export function useSidebarTabs({capabilities, getAstapStatus, getCatalogStatus, scroller = null}: SidebarTabsOptions) {
    const activeTab = ref<SidebarTab>(SIDEBAR_TABS.MAIN)
    const visited = reactive(new Set<SidebarTab>([SIDEBAR_TABS.MAIN]))
    const checkingSetup = ref(false)
    const showPushToSetup = ref(false)

    // Bumped by every choice, so a setup check that resolves after the observer picked
    // another tab cannot pull them back to Push-To.
    let choice = 0

    function show(tab: SidebarTab) {
        choice++
        activeTab.value = tab
        visited.add(tab)
    }

    async function openPushTo() {
        // Community gets the panel's lock screen without touching ASTAP or the catalog.
        if (!capabilities.value.push_to.astap_solver) return show(SIDEBAR_TABS.PUSH_TO)

        const token = ++choice
        checkingSetup.value = true
        let ready = false
        try {
            const [astap, catalog] = await Promise.all([getAstapStatus(), getCatalogStatus()])
            ready = Boolean(astap.ready && catalog.installed)
        } catch {
            // A failed check opens the setup overlay, which reports what is missing.
        } finally {
            checkingSetup.value = false
        }
        if (token !== choice) return undefined

        if (ready) return show(SIDEBAR_TABS.PUSH_TO)
        showPushToSetup.value = true
        return undefined
    }

    /** The active tab's button returns to Main; any other opens its tab. */
    function toggle(tab: SidebarTab) {
        if (activeTab.value === tab) return show(SIDEBAR_TABS.MAIN)
        if (tab === SIDEBAR_TABS.PUSH_TO) return openPushTo()
        return show(tab)
    }

    function setupComplete() {
        showPushToSetup.value = false
        show(SIDEBAR_TABS.PUSH_TO)
    }

    function closeSetup() {
        showPushToSetup.value = false
    }

    // The sidebar is one scroll container; without this a tab opens at the last one's
    // offset. Read before the switch renders: hiding the old pane clamps the offset.
    if (scroller) {
        const scrollTops: Partial<Record<SidebarTab, number>> = {}
        watch(activeTab, async (tab, previous) => {
            if (!scroller.value) return
            scrollTops[previous] = scroller.value.scrollTop
            await nextTick()
            if (scroller.value) scroller.value.scrollTop = scrollTops[tab] ?? 0
        })
    }

    return {activeTab, visited, checkingSetup, showPushToSetup, toggle, setupComplete, closeSetup}
}
