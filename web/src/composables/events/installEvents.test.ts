import {createInstallEvents, INSTALL_INTERRUPTED} from './installEvents'
import {sendTo} from '../../test/eventStream'
import type {EventOf, ServerEventType} from '../api.events'

/** An install table, and `send(type, fields)` to hand it an event as the stream would. */
function installEvents() {
    const {handlers, state} = createInstallEvents()
    const send = <K extends ServerEventType>(type: K, fields: Partial<Omit<EventOf<K>, 'type'>>) =>
        sendTo(handlers, type, fields)
    return {state, send}
}

describe('createInstallEvents', () => {
    it('reads a running install as its last event', () => {
        const {send, state} = installEvents()

        send('astap_install_status', {
            state: 'running',
            progress: {
                type: 'astap_install_progress', component: 'D80 Database', bytes_downloaded: 500,
                total_bytes: 2000, percent: 25, stage: 'Downloading Database', stage_id: 'downloading_database', overall_percent: 37.5,
            },
        })
        send('catalog_install_status', {
            state: 'running',
            progress: {type: 'catalog_install_progress', file_name: 'NGC.csv', bytes_downloaded: 300, total_bytes: 1200, percent: 25},
        })

        expect(state.astapInstallProgress.value).toEqual({
            component: 'D80 Database', stage: 'downloading', percent: 25, bytesDownloaded: 500,
            totalBytes: 2000, stageId: 'downloading_database', overallPercent: 37.5, error: null,
        })
        expect(state.catalogInstallProgress.value).toEqual({
            stage: 'downloading', fileName: 'NGC.csv', percent: 25, bytesDownloaded: 300, totalBytes: 1200, error: null,
        })
    })

    it('reads an install begun but not yet heard from as starting', () => {
        const {send, state} = installEvents()

        send('astap_install_status', {state: 'running', progress: null})
        send('catalog_install_status', {state: 'running', progress: null})

        expect(state.astapInstallProgress.value).toMatchObject({stage: 'starting', component: 'ASTAP'})
        expect(state.catalogInstallProgress.value).toMatchObject({stage: 'starting'})
    })

    it('reads completed as an ended install', () => {
        const {send, state} = installEvents()

        send('astap_install_status', {state: 'completed'})
        send('catalog_install_status', {state: 'completed'})

        expect(state.astapInstallProgress.value).toMatchObject({stage: 'completed', ended: true, error: null})
        expect(state.catalogInstallProgress.value).toMatchObject({stage: 'completed', ended: true, error: null})
    })

    // `idle` is what a restarted server says: read as completed, a restart showed "installed".
    it('reads idle as an install that was cut short', () => {
        const {send, state} = installEvents()

        send('astap_install_status', {state: 'idle'})
        send('catalog_install_status', {state: 'idle'})

        const interrupted = {stage: 'failed', error: INSTALL_INTERRUPTED, ended: true, interrupted: true}
        expect(state.astapInstallProgress.value).toMatchObject(interrupted)
        expect(state.catalogInstallProgress.value).toEqual(interrupted)
    })

    it('reads a failed install as ended, with its error', () => {
        const {send, state} = installEvents()

        send('astap_install_status', {state: 'failed', error: 'network'})
        send('catalog_install_status', {state: 'failed', error: 'disk full'})

        expect(state.astapInstallProgress.value).toMatchObject({stage: 'failed', error: 'network', ended: true})
        expect(state.catalogInstallProgress.value).toEqual({stage: 'failed', error: 'disk full', ended: true})
    })

    it('marks only a status as ended, never a live event', () => {
        const {send, state} = installEvents()

        send('astap_install_completed', {component: 'D80 Database', stage: 'Database Installed', stage_id: 'database_completed', overall_percent: 100})
        send('catalog_install_completed', {object_count: 13957})

        expect(state.astapInstallProgress.value).not.toHaveProperty('ended')
        expect(state.catalogInstallProgress.value).not.toHaveProperty('ended')
    })
})
