import {ref, computed, watch, onMounted, onUnmounted, inject} from 'vue'
import {getAstapStatus, getAstapDatabases, installAstap} from './api.js'

/**
 * Composable for managing ASTAP installation state and progress.
 * Handles status checking, database selection, installation progress tracking,
 * and WebSocket event integration.
 */
export function useAstapInstall() {
    // Core state
    const loading = ref(true)
    const installing = ref(false)
    const status = ref(null)
    const databases = ref([])
    const selectedDatabase = ref('D80')
    const error = ref(null)

    // Installation progress
    const installProgress = ref(createEmptyProgress())

    // Stage completion tracking
    const stageCompletion = ref(createEmptyStageCompletion())

    // Event stream for receiving WebSocket events
    const eventStream = inject('eventStream', null)

    // Computed properties
    const canInstall = computed(() => !installing.value && selectedDatabase.value)

    const progressText = computed(() => {
        return formatProgressText(installProgress.value)
    })

    const overallProgressText = computed(() => {
        const p = installProgress.value
        if (p.overallPercent !== null && p.overallPercent !== undefined) {
            return `Overall progress: ${p.overallPercent.toFixed(0)}%`
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
            const [statusData, dbData] = await Promise.all([getAstapStatus(), getAstapDatabases()])
            status.value = statusData
            databases.value = dbData
            return statusData
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
            await installAstap(selectedDatabase.value)
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

        updateStageCompletion(stageCompletion.value, stage, progress.stageName)

        if (stage === 'failed') {
            error.value = `Installation failed: ${progress.error}`
            installing.value = false
        }
    }

    function isInstallationComplete() {
        const p = installProgress.value
        return p.stage === 'completed' && p.component && p.component.includes('Database')
    }

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
        selectedDatabase,
        error,
        installProgress,
        stageCompletion,
        // Computed
        canInstall,
        progressText,
        overallProgressText,
        progressPercent,
        // Methods
        loadStatus,
        startInstall,
        handleProgressUpdate,
        isInstallationComplete,
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
        dbDownloaded: false,
        dbExtracted: false,
        dbCompleted: false,
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
            const downloadedMb = (bytesDownloaded / (1024 * 1024)).toFixed(1)
            const totalMb = (totalBytes / (1024 * 1024)).toFixed(1)
            return `Downloading ${component}: ${downloadedMb} / ${totalMb} MB (${percent.toFixed(1)}%)`
        }
        if (percent !== null) {
            return `Downloading ${component}: ${percent.toFixed(1)}%`
        }
        const mb = (bytesDownloaded / (1024 * 1024)).toFixed(1)
        return `Downloading ${component}: ${mb} MB`
    }

    if (stage === 'extracting') {
        if (percent !== null) {
            return `Extracting ${component}: ${percent.toFixed(1)}%`
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

function updateStageCompletion(completion, stage, stageName) {
    if (stage === 'downloading') {
        if (stageName === 'Downloading ASTAP CLI') {
            completion.cliDownloaded = false
        } else if (stageName === 'Downloading Database') {
            completion.cliCompleted = true
            completion.dbDownloaded = false
        }
    } else if (stage === 'extracting') {
        if (stageName === 'Extracting ASTAP CLI') {
            completion.cliDownloaded = true
            completion.cliExtracted = false
        } else if (stageName === 'Extracting Database') {
            completion.dbDownloaded = true
            completion.dbExtracted = false
        }
    } else if (stage === 'completed') {
        if (stageName === 'ASTAP CLI Installed') {
            completion.cliExtracted = true
            completion.cliCompleted = true
        } else if (stageName === 'Database Installed') {
            completion.dbExtracted = true
            completion.dbCompleted = true
        }
    }
}
