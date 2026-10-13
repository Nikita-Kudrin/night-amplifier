import {ref, computed, watch, onMounted, onUnmounted, inject} from 'vue'
import {getAstapStatus, getAstapDatabases, installAstap} from './api.js'
import {useTimeouts} from './useTimeouts.js'
import {formatBytes, formatPercent} from '../utils/format.js'

/** The database a fresh install offers: the one sized for most fields of view. */
export const DEFAULT_ASTAP_DATABASE = 'D80'

/** How long after an install's last event the status is read again. */
export const INSTALL_SETTLE_MS = 500

/** ASTAP's status and the databases it offers, fetched together as every caller wants them. */
export async function fetchAstapState() {
    const [status, databases] = await Promise.all([getAstapStatus(), getAstapDatabases()])
    return {status, databases}
}

/**
 * Once `complete()` turns true, waits `INSTALL_SETTLE_MS`, reloads the status, and hands
 * it to `onComplete`. The wait is the component's, cleared if it goes away first.
 */
export function useInstallSettle(complete, {loadStatus, installing, status, onComplete}) {
    const {later} = useTimeouts()
    watch(complete, (done) => {
        if (!done) return
        later(async () => {
            await loadStatus()
            installing.value = false
            onComplete?.(status.value)
        }, INSTALL_SETTLE_MS)
    })
}

/**
 * Composable for managing ASTAP installation state and progress.
 * Handles status checking, database selection, installation progress tracking,
 * and WebSocket event integration. Supports multi-database selection.
 * @param {Object} [options]
 * @param {(status: Object) => void} [options.onComplete] - The install finished and the
 *   status was reloaded; `installing` is false again.
 */
export function useAstapInstall({onComplete} = {}) {
    // Core state
    const loading = ref(true)
    const installing = ref(false)
    const status = ref(null)
    const databases = ref([])
    const selectedDatabases = ref([DEFAULT_ASTAP_DATABASE])
    const error = ref(null)

    // Installation progress
    const installProgress = ref(createEmptyProgress())

    // Stage completion tracking
    const stageCompletion = ref(createEmptyStageCompletion())

    // Event stream for receiving WebSocket events
    const eventStream = inject('eventStream', null)

    // Computed properties
    const canInstall = computed(() => !installing.value && selectedDatabases.value.length > 0)

    const hasUninstalledDatabases = computed(() =>
        databases.value.some(db => !db.installed)
    )

    const progressText = computed(() => {
        return formatProgressText(installProgress.value)
    })

    const overallProgressText = computed(() => {
        const p = installProgress.value
        if (p.overallPercent !== null && p.overallPercent !== undefined) {
            return `Overall progress: ${formatPercent(p.overallPercent / 100)}`
        }
        return ''
    })

    const progressPercent = computed(() => {
        return calculateProgressPercent(installProgress.value)
    })

    // Methods
    async function loadStatus() {
        loading.value = true
        error.value = null
        try {
            const state = await fetchAstapState()
            status.value = state.status
            databases.value = state.databases
            return state.status
        } catch (e) {
            error.value = e.message
            return null
        } finally {
            loading.value = false
        }
    }

    async function startInstall() {
        if (!canInstall.value) return false

        installing.value = true
        error.value = null
        installProgress.value = createEmptyProgress()
        stageCompletion.value = createEmptyStageCompletion()

        try {
            await installAstap(selectedDatabases.value)
            return true
        } catch (e) {
            error.value = e.message
            installing.value = false
            return false
        }
    }

    function handleProgressUpdate(progress) {
        if (!progress) return

        const stage = progress.stage

        installProgress.value = {
            component: progress.component || '',
            percent: progress.percent,
            bytesDownloaded: progress.bytesDownloaded || 0,
            totalBytes: progress.totalBytes,
            stage: stage,
            stageName: progress.stageName || '',
            overallPercent: progress.overallPercent,
            error: progress.error,
        }

        updateStageCompletion(stageCompletion.value, stage, progress.stageName, progress.component)

        if (stage === 'failed') {
            error.value = `Installation failed: ${progress.error}`
            installing.value = false
        }
    }

    // Each database reports its own completion; the install is done with the last one's.
    const installComplete = computed(() => {
        const p = installProgress.value
        if (p.stage !== 'completed') return false
        const lastDb = selectedDatabases.value[selectedDatabases.value.length - 1]
        return Boolean(p.component?.includes(lastDb))
    })

    useInstallSettle(installComplete, {loadStatus, installing, status, onComplete})

    function resetState() {
        installProgress.value = createEmptyProgress()
        stageCompletion.value = createEmptyStageCompletion()
        installing.value = false
        error.value = null
    }

    // Watch for ASTAP install progress updates from eventStream
    const stopWatch = watch(
        () => eventStream?.astapInstallProgress?.value,
        (progress) => {
            if (progress) {
                handleProgressUpdate(progress)
            }
        },
        {deep: true}
    )

    // Lifecycle
    onMounted(() => {
        loadStatus()
    })

    onUnmounted(() => {
        eventStream?.clearAstapInstallProgress?.()
        stopWatch()
    })

    return {
        // State
        loading,
        installing,
        status,
        databases,
        selectedDatabases,
        error,
        installProgress,
        stageCompletion,
        // Computed
        canInstall,
        hasUninstalledDatabases,
        progressText,
        overallProgressText,
        progressPercent,
        installComplete,
        // Methods
        loadStatus,
        startInstall,
        handleProgressUpdate,
        resetState,
    }
}

// Helper functions

function createEmptyProgress() {
    return {
        component: '',
        percent: null,
        bytesDownloaded: 0,
        totalBytes: null,
        stage: '',
        stageName: '',
        overallPercent: null,
        error: null,
    }
}

function createEmptyStageCompletion() {
    return {
        cliDownloaded: false,
        cliExtracted: false,
        cliCompleted: false,
        completedDatabases: new Set(),
    }
}

function formatProgressText(progress) {
    const {stage, component, percent, bytesDownloaded, totalBytes, error: progressError} = progress

    if (stage === 'starting') {
        if (component === 'ASTAP CLI') {
            return 'Installing ASTAP...'
        }
        if (component && component.includes('Database')) {
            return `Downloading ${component.replace('Database', 'star database')}...`
        }
        return `Starting ${component}...`
    }

    if (stage === 'downloading') {
        if (percent !== null && totalBytes !== null) {
            const sizes = `${formatBytes(bytesDownloaded)} / ${formatBytes(totalBytes)}`
            return `Downloading ${component}: ${sizes} (${formatPercent(percent / 100, 1)})`
        }
        if (percent !== null) {
            return `Downloading ${component}: ${formatPercent(percent / 100, 1)}`
        }
        return `Downloading ${component}: ${formatBytes(bytesDownloaded)}`
    }

    if (stage === 'extracting') {
        if (percent !== null) {
            return `Extracting ${component}: ${formatPercent(percent / 100, 1)}`
        }
        return `Extracting ${component}...`
    }

    if (stage === 'completed') {
        return `${component} installed successfully`
    }

    if (stage === 'failed') {
        return `Failed: ${progressError}`
    }

    return ''
}

function calculateProgressPercent(progress) {
    const {stage, percent, overallPercent} = progress

    if (overallPercent !== null && overallPercent !== undefined) {
        return overallPercent
    }
    if (stage === 'downloading' && percent !== null && percent !== undefined) {
        return percent
    }
    if (stage === 'extracting' && percent !== null && percent !== undefined) {
        return percent
    }
    if (stage === 'completed') {
        return 100
    }
    return null
}

function updateStageCompletion(completion, stage, stageName, component) {
    if (stage === 'downloading') {
        if (stageName === 'Downloading ASTAP CLI') {
            completion.cliDownloaded = false
        } else if (stageName === 'Downloading Database') {
            completion.cliCompleted = true
        }
    } else if (stage === 'extracting') {
        if (stageName === 'Extracting ASTAP CLI') {
            completion.cliDownloaded = true
            completion.cliExtracted = false
        }
    } else if (stage === 'completed') {
        if (stageName === 'ASTAP CLI Installed') {
            completion.cliExtracted = true
            completion.cliCompleted = true
        } else if (stageName === 'Database Installed' && component) {
            completion.completedDatabases.add(component)
        }
    }
}
