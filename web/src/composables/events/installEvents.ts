import {ref} from 'vue'
import type {AstapInstallStatusEvent, CatalogInstallStatusEvent, ServerEvent} from '../api.events.js'
import {handlerOf} from './handlers.js'
import type {EventHandlers} from './handlers.js'

/**
 * Why an install a page was following ended with the server saying `idle` — nothing
 * installed since it started. Read as a success, it showed "installed" for a restart.
 */
export const INSTALL_INTERRUPTED = 'The server restarted before the install finished'

interface Transfer {
    percent: number | null
    bytesDownloaded: number
    totalBytes: number | null
}

/**
 * Where an install stands, by `stage`, plus `Extra`: what one installer adds to every
 * stage. `ended` marks a `*_install_status` that reported the end, `interrupted` one that
 * said `idle`; a live event carries neither.
 */
export type InstallProgress<Extra> = Extra & (
    | ({stage: 'starting' | 'downloading'; error: null} & Transfer)
    | {stage: 'extracting'; percent: number; error: null}
    // A file done keeps the fields of what it followed, the download's when there was one.
    | ({stage: 'file_completed'; error?: string | null} & Partial<Transfer>)
    | {stage: 'completed'; error: null; ended?: true}
    | {stage: 'failed'; error: string; ended?: true; interrupted?: true}
)

export type AstapInstallProgress = InstallProgress<{
    component: string
    stageName?: string | null
    overallPercent?: number | null
}>

export type CatalogInstallProgress = InstallProgress<{
    fileName?: string
    objectCount?: number | null
}>

/**
 * Download and unpack progress of the ASTAP and catalog installs, in the client's casing:
 * the installers read only these fields, never the event's own.
 *
 * A `*_install_status` (sent as an install begins and ends, and by every sync) replays a
 * running install's last event, or reads as an `ended` 'completed'/'failed' — which an
 * installer acts on only while it believes an install runs: every connect says `idle`, a
 * failure marked `interrupted` (see `INSTALL_INTERRUPTED`).
 */
export function createInstallEvents() {
    const astapInstallProgress = ref<AstapInstallProgress | null>(null)
    const catalogInstallProgress = ref<CatalogInstallProgress | null>(null)

    const handlers = {
        astap_install_starting(data) {
            astapInstallProgress.value = {
                component: data.component, stage: 'starting',
                percent: null, bytesDownloaded: 0, totalBytes: null,
                overallPercent: null, error: null,
            }
        },
        astap_install_progress(data) {
            astapInstallProgress.value = {
                component: data.component, stage: 'downloading',
                percent: data.percent, bytesDownloaded: data.bytes_downloaded,
                totalBytes: data.total_bytes, stageName: data.stage,
                overallPercent: data.overall_percent, error: null,
            }
        },
        astap_install_extracting(data) {
            astapInstallProgress.value = {
                component: data.component, stage: 'extracting',
                percent: data.progress, stageName: data.stage,
                overallPercent: data.overall_percent, error: null,
            }
        },
        astap_install_completed(data) {
            astapInstallProgress.value = {
                component: data.component, stage: 'completed',
                stageName: data.stage, overallPercent: data.overall_percent, error: null,
            }
        },
        astap_install_failed(data) {
            astapInstallProgress.value = {component: data.component, stage: 'failed', error: data.error}
        },
        catalog_install_starting() {
            catalogInstallProgress.value = {
                stage: 'starting', fileName: '',
                percent: null, bytesDownloaded: 0, totalBytes: null, error: null,
            }
        },
        catalog_install_progress(data) {
            catalogInstallProgress.value = {
                stage: 'downloading', fileName: data.file_name,
                percent: data.percent, bytesDownloaded: data.bytes_downloaded,
                totalBytes: data.total_bytes, error: null,
            }
        },
        catalog_file_completed(data) {
            catalogInstallProgress.value = {
                ...catalogInstallProgress.value,
                fileName: data.file_name, stage: 'file_completed',
            }
        },
        catalog_install_completed(data) {
            catalogInstallProgress.value = {stage: 'completed', objectCount: data.object_count, error: null}
        },
        catalog_install_failed(data) {
            catalogInstallProgress.value = {stage: 'failed', error: data.error}
        },
        astap_install_status(data) {
            if (data.state === 'running') {
                replay(data.progress ?? {type: 'astap_install_starting', component: 'ASTAP'})
                return
            }
            astapInstallProgress.value = data.state === 'completed'
                ? {component: '', stage: 'completed', stageName: '', overallPercent: 100, error: null, ended: true}
                : {component: '', ...endedFailure(data)}
        },
        catalog_install_status(data) {
            if (data.state === 'running') {
                replay(data.progress ?? {type: 'catalog_install_starting'})
                return
            }
            catalogInstallProgress.value = data.state === 'completed'
                ? {stage: 'completed', objectCount: null, error: null, ended: true}
                : endedFailure(data)
        },
    } satisfies EventHandlers

    function replay(event: ServerEvent) {
        handlerOf(handlers, event.type)?.(event)
    }

    /** A `failed` status, or an `idle` one, which can only mean the install was cut short. */
    function endedFailure(data: AstapInstallStatusEvent | CatalogInstallStatusEvent) {
        return data.state === 'idle'
            ? {stage: 'failed', error: INSTALL_INTERRUPTED, ended: true, interrupted: true} as const
            // The server sends `error` with every `failed`; the generated type flattens the states.
            : {stage: 'failed', error: data.error ?? '', ended: true} as const
    }

    return {
        handlers,
        state: {
            astapInstallProgress,
            catalogInstallProgress,
            clearAstapInstallProgress: () => (astapInstallProgress.value = null),
            clearCatalogInstallProgress: () => (catalogInstallProgress.value = null),
        },
    }
}
