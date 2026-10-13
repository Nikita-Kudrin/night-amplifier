import {computed, inject, onMounted, onUnmounted, ref, watch} from 'vue'
import {getCatalogStatus, installCatalog} from './api.js'
import {useInstallSettle} from './useAstapInstall.js'
import {formatBytes} from '../utils/format.js'

function emptyProgress(stage = '') {
    return {stage, fileName: '', percent: null, bytesDownloaded: 0, totalBytes: null, error: null}
}

/**
 * The target catalog install: its status, the request, and its progress from the event
 * stream. Built like `useAstapInstall`, but a composable of its own — the catalog reports
 * files and an object count, ASTAP stages and databases.
 * @param {Object} [options]
 * @param {(status: Object) => void} [options.onComplete] - Installed and reloaded.
 */
export function useCatalogInstall({onComplete} = {}) {
    const loading = ref(true)
    const installing = ref(false)
    const status = ref(null)
    const error = ref(null)
    const progress = ref(emptyProgress())

    const eventStream = inject('eventStream', null)

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
            error.value = e.message
            return null
        } finally {
            loading.value = false
        }
    }

    async function startInstall(includeStars) {
        if (!canInstall.value) return false
        installing.value = true
        error.value = null
        progress.value = emptyProgress('starting')
        try {
            await installCatalog(includeStars)
            return true
        } catch (e) {
            error.value = e.message
            installing.value = false
            return false
        }
    }

    function handleProgress(p) {
        progress.value = {...emptyProgress(), ...p}
        if (p.stage === 'failed') {
            error.value = `Installation failed: ${p.error}`
            installing.value = false
        }
    }

    watch(() => eventStream?.catalogInstallProgress?.value, (p) => p && handleProgress(p), {deep: true})

    const installComplete = computed(() => progress.value.stage === 'completed')
    useInstallSettle(installComplete, {loadStatus, installing, status, onComplete})

    onMounted(loadStatus)
    onUnmounted(() => eventStream?.clearCatalogInstallProgress?.())

    return {loading, installing, status, error, progress, canInstall, progressText, installComplete, loadStatus, startInstall}
}
