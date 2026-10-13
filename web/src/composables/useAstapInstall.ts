import {ref, computed, watch} from 'vue'
import type {Ref, WatchSource} from 'vue'
import {useEventStreamContext} from './appContext.js'
import {tryOnMounted, tryOnScopeDispose} from './lifecycle.js'
import {getAstapStatus, getAstapDatabases, installAstap} from './api.js'
import {useTimeouts} from './useTimeouts.js'
import {formatBytes, formatPercent} from '../utils/format.js'
import type {AstapStatus, DatabaseType} from './api.types.js'
import type {AstapInstallProgress, InstallProgress} from './events/installEvents.js'

/** Where an install stands, the same stages for ASTAP and the catalog. */
export type InstallStage = InstallProgress<object>['stage']

/**
 * The ASTAP install's progress as the installer shows it: the last event's fields, each
 * present. `stage` is '' before any event, and a field the stage has none of is empty.
 */
export interface AstapProgressView {
    component: string
    stage: InstallStage | ''
    percent: number | null
    bytesDownloaded: number
    totalBytes: number | null
    stageName: string
    overallPercent: number | null
    error: string | null
    ended: boolean
}

/** Which parts of an ASTAP install finished, by the stage names the server reports. */
export interface AstapStageCompletion {
    cliDownloaded: boolean
    cliExtracted: boolean
    cliCompleted: boolean
    completedDatabases: Set<string>
}

/** What `ignoresProgress` reads of a progress: its stage, and how a status ended it. */
interface ProgressEnd {
    stage: InstallStage
    ended?: boolean
    interrupted?: boolean
}

export interface InstallOptions<S> {
    /** The install finished and the status was reloaded; `installing` is false again. */
    onComplete?: (status: S) => void
}

/** The database a fresh install offers: the one sized for most fields of view. */
export const DEFAULT_ASTAP_DATABASE = 'D80'

/** How long after an install's last event the status is read again. */
export const INSTALL_SETTLE_MS = 500

/** Stages of an install still under way, which a page that did not start it joins. */
export const ACTIVE_INSTALL_STAGES: ReadonlySet<InstallStage> =
    new Set(['starting', 'downloading', 'extracting', 'file_completed'])

/**
 * Whether an installer ignores `progress`: an `ended` status while it follows no install,
 * or a sync's `idle` (`interrupted`) while its own request is still on the way — the
 * server says that until the request reaches it.
 */
export function ignoresProgress(
    progress: ProgressEnd,
    {installing, requesting}: {installing: boolean; requesting: boolean},
): boolean {
    return Boolean(progress.ended && (!installing || (progress.interrupted && requesting)))
}

/** ASTAP's status and the databases it offers, fetched together as every caller wants them. */
export async function fetchAstapState() {
    const [status, databases] = await Promise.all([getAstapStatus(), getAstapDatabases()])
    return {status, databases}
}

/**
 * Once `complete()` turns true, waits `INSTALL_SETTLE_MS`, reloads the status, and hands
 * it to `onComplete`. The wait is the component's, cleared if it goes away first.
 */
export function useInstallSettle<S>(
    complete: WatchSource<boolean>,
    {loadStatus, installing, status, onComplete}: {
        loadStatus: () => Promise<unknown>
        installing: Ref<boolean>
        status: Readonly<Ref<S>>
    } & InstallOptions<S>,
) {
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
 */
export function useAstapInstall({onComplete}: InstallOptions<AstapStatus | null> = {}) {
    // Core state
    const loading = ref(true)
    const installing = ref(false)
    const status = ref<AstapStatus | null>(null)
    const databases = ref<DatabaseType[]>([])
    const selectedDatabases = ref<string[]>([DEFAULT_ASTAP_DATABASE])
    const error = ref<string | null>(null)

    // Installation progress
    const installProgress = ref(createEmptyProgress())

    // Stage completion tracking
    const stageCompletion = ref(createEmptyStageCompletion())

    // Event stream for receiving WebSocket events
    const eventStream = useEventStreamContext()

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
        if (p.overallPercent !== null) {
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
            error.value = e instanceof Error ? e.message : String(e)
            return null
        } finally {
            loading.value = false
        }
    }

    let requesting = false

    async function startInstall() {
        if (!canInstall.value) return false

        installing.value = true
        error.value = null
        installProgress.value = createEmptyProgress()
        stageCompletion.value = createEmptyStageCompletion()

        requesting = true
        try {
            await installAstap(selectedDatabases.value)
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

    function handleProgressUpdate(progress: AstapInstallProgress | null) {
        if (!progress) return
        if (ignoresProgress(progress, {installing: installing.value, requesting})) return

        const stage = progress.stage
        if (ACTIVE_INSTALL_STAGES.has(stage) && !installing.value) {
            // Joining an install: the refusal a click on this page met is moot now.
            installing.value = true
            error.value = null
        }

        // A stage without a download or a percent leaves those fields empty.
        installProgress.value = {
            ...createEmptyProgress(),
            ...progress,
            stageName: progress.stageName || '',
            ended: 'ended' in progress && Boolean(progress.ended),
        }

        updateStageCompletion(stageCompletion.value, stage, progress.stageName, progress.component)

        if (stage === 'failed') {
            error.value = `Installation failed: ${progress.error}`
            installing.value = false
        }
    }

    // Each database reports its own completion; the install is done with the last one's,
    // or once the server says it ended (a page that joined it may not know its databases).
    const installComplete = computed(() => {
        const p = installProgress.value
        if (p.stage !== 'completed') return false
        if (p.ended) return true
        const lastDb = selectedDatabases.value[selectedDatabases.value.length - 1]
        return p.component.includes(lastDb)
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
    tryOnMounted(loadStatus)

    tryOnScopeDispose(() => {
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

export type AstapInstall = ReturnType<typeof useAstapInstall>

// Helper functions

function createEmptyProgress(): AstapProgressView {
    return {
        component: '',
        percent: null,
        bytesDownloaded: 0,
        totalBytes: null,
        stage: '',
        stageName: '',
        overallPercent: null,
        error: null,
        ended: false,
    }
}

function createEmptyStageCompletion(): AstapStageCompletion {
    return {
        cliDownloaded: false,
        cliExtracted: false,
        cliCompleted: false,
        completedDatabases: new Set(),
    }
}

function formatProgressText(progress: AstapProgressView): string {
    const {stage, component, percent, bytesDownloaded, totalBytes, error: progressError} = progress

    if (stage === 'starting') {
        if (component === 'ASTAP CLI') {
            return 'Installing ASTAP...'
        }
        if (component.includes('Database')) {
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

function calculateProgressPercent(progress: AstapProgressView): number | null {
    const {stage, percent, overallPercent} = progress

    if (overallPercent !== null) {
        return overallPercent
    }
    if (stage === 'downloading' && percent !== null) {
        return percent
    }
    if (stage === 'extracting' && percent !== null) {
        return percent
    }
    if (stage === 'completed') {
        return 100
    }
    return null
}

function updateStageCompletion(
    completion: AstapStageCompletion,
    stage: InstallStage,
    stageName: string | null | undefined,
    component: string,
) {
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
