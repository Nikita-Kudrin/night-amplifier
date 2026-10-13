import {ref} from 'vue'

/**
 * Why an install a page was following ended with the server saying `idle` — nothing
 * installed since it started. Read as a success, it showed "installed" for a restart.
 */
export const INSTALL_INTERRUPTED = 'The server restarted before the install finished'

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
    const astapInstallProgress = ref(null)
    const catalogInstallProgress = ref(null)

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
    }

    function replay(event) {
        handlers[event.type]?.(event)
    }

    /** A `failed` status, or an `idle` one, which can only mean the install was cut short. */
    function endedFailure(data) {
        return data.state === 'idle'
            ? {stage: 'failed', error: INSTALL_INTERRUPTED, ended: true, interrupted: true}
            : {stage: 'failed', error: data.error, ended: true}
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
