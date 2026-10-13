import {computed, ref, watch} from 'vue'
import {useEventStreamContext} from './appContext.js'
import {tryOnMounted, tryOnScopeDispose} from './lifecycle.js'
import {getCatalogStatus, installCatalog} from './api.js'
import {ACTIVE_INSTALL_STAGES, ignoresProgress, useInstallSettle} from './useAstapInstall.js'
import type {InstallOptions, InstallStage} from './useAstapInstall.js'
import {formatBytes} from '../utils/format.js'
import type {CatalogStatus} from './api.types.js'
import type {CatalogInstallProgress} from './events/installEvents.js'

/**
 * The catalog install's progress as the installer shows it: the last event over an empty
 * one, so a field its stage has none of is empty. `stage` is '' before any event.
 */
export type CatalogProgressView = {
    stage: InstallStage | ''
    fileName: string
    percent: number | null
    bytesDownloaded: number
    totalBytes: number | null
    error: string | null
} & Partial<Pick<CatalogInstallProgress, 'objectCount'>>

function emptyProgress(stage: InstallStage | '' = ''): CatalogProgressView {
    return {stage, fileName: '', percent: null, bytesDownloaded: 0, totalBytes: null, error: null}
}

/**
 * The target catalog install: its status, the request, and its progress from the event
 * stream. Built like `useAstapInstall`, but a composable of its own — the catalog reports
 * files and an object count, ASTAP stages and databases.
 */
export function useCatalogInstall({onComplete}: InstallOptions<CatalogStatus | null> = {}) {
    const loading = ref(true)
    const installing = ref(false)
    const status = ref<CatalogStatus | null>(null)
    const error = ref<string | null>(null)
    const progress = ref(emptyProgress())

    const eventStream = useEventStreamContext()

    const canInstall = computed(() => !installing.value)

    const progressText = computed(() => {
        const p = progress.value
        if (p.stage === 'starting') return 'Starting download...'
        if (p.stage === 'downloading') {
            if (p.percent !== null && p.totalBytes !== null) {
                return `Downloading ${p.fileName}: ${formatBytes(p.bytesDownloaded)} / ${formatBytes(p.totalBytes)}`
            }
            return `Downloading ${p.fileName}: ${formatBytes(p.bytesDownloaded)}`
        }
        if (p.stage === 'completed') return 'Catalog installed'
        if (p.stage === 'failed') return `Failed: ${p.error}`
        return ''
    })

    async function loadStatus() {
        loading.value = true
        error.value = null
        try {
            status.value = await getCatalogStatus()
            return status.value
        } catch (e) {
            error.value = e instanceof Error ? e.message : String(e)
            return null
        } finally {
            loading.value = false
        }
    }

    let requesting = false

    async function startInstall(includeStars: boolean) {
        if (!canInstall.value) return false
        installing.value = true
        error.value = null
        progress.value = emptyProgress('starting')
        requesting = true
        try {
            await installCatalog(includeStars)
            return true
        } catch (e) {
            // A 409 too: another page's install runs, which this one joins on its next event.
            error.value = e instanceof Error ? e.message : String(e)
            installing.value = false
            return false
        } finally {
            requesting = false
        }
    }

    function handleProgress(p: CatalogInstallProgress) {
        if (ignoresProgress(p, {installing: installing.value, requesting})) return
        progress.value = {...emptyProgress(), ...p}
        if (ACTIVE_INSTALL_STAGES.has(p.stage) && !installing.value) {
            installing.value = true
            error.value = null
        }
        if (p.stage === 'failed') {
            error.value = `Installation failed: ${p.error}`
            installing.value = false
        }
    }

    watch(() => eventStream?.catalogInstallProgress?.value, (p) => p && handleProgress(p), {deep: true})

    const installComplete = computed(() => progress.value.stage === 'completed')
    useInstallSettle(installComplete, {loadStatus, installing, status, onComplete})

    tryOnMounted(loadStatus)
    tryOnScopeDispose(() => eventStream?.clearCatalogInstallProgress?.())

    return {loading, installing, status, error, progress, canInstall, progressText, installComplete, loadStatus, startInstall}
}

export type CatalogInstall = ReturnType<typeof useCatalogInstall>
