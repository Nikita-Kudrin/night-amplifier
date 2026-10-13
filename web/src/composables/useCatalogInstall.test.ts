import {defineComponent, h, nextTick} from 'vue'
import {flushPromises, mount} from '@vue/test-utils'
import {catalogStatus} from '../test/fixtures.js'
import {createEventStreamStub, sendTo} from '../test/eventStream.js'
import {INSTALL_SETTLE_MS} from './useAstapInstall.js'
import type {InstallOptions} from './useAstapInstall.js'
import {useCatalogInstall} from './useCatalogInstall.js'
import type {CatalogInstall} from './useCatalogInstall.js'
import {createInstallEvents, INSTALL_INTERRUPTED} from './events/installEvents.js'
import type {CatalogInstallProgress} from './events/installEvents.js'
import type {CatalogStatus, Message} from './api.types.js'
import type {EventOf, ServerEventType} from './api.events.js'

vi.mock('./api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal()))

import {ApiError, getCatalogStatus, installCatalog} from './api.js'
import {contextProvides} from './appContext.js'

const MISSING = catalogStatus({installed: false})

function mountInstall(options: InstallOptions<CatalogStatus | null> = {}, eventStream = createEventStreamStub()) {
    const progress = eventStream.catalogInstallProgress
    let install!: CatalogInstall
    const Host = defineComponent({
        setup() {
            install = useCatalogInstall(options)
            return () => h('div')
        },
    })
    const wrapper = mount(Host, {global: {provide: contextProvides({eventStream})}})
    return {wrapper, install, progress, eventStream}
}

describe('useCatalogInstall', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        vi.mocked(getCatalogStatus).mockResolvedValue(MISSING)
    })
    afterEach(() => vi.useRealTimers())

    it('loads the status on mount', async () => {
        const {install} = mountInstall()
        expect(install.loading.value).toBe(true)
        await flushPromises()

        expect(install.loading.value).toBe(false)
        expect(install.status.value).toEqual(MISSING)
    })

    it('installs once at a time, with or without the stars', async () => {
        const {install} = mountInstall()
        await flushPromises()

        expect(await install.startInstall(false)).toBe(true)
        expect(install.progressText.value).toBe('Starting download...')
        expect(await install.startInstall(true)).toBe(false)
        expect(installCatalog).toHaveBeenCalledTimes(1)
        expect(installCatalog).toHaveBeenCalledWith(false)
    })

    it('says why the request failed and lets it be tried again', async () => {
        vi.mocked(installCatalog).mockRejectedValueOnce(new Error('offline'))
        const {install} = mountInstall()
        await flushPromises()

        expect(await install.startInstall(true)).toBe(false)
        expect(install.error.value).toBe('offline')
        expect(install.canInstall.value).toBe(true)
    })

    it.each<[CatalogInstallProgress, string]>([
        [{stage: 'downloading', fileName: 'NGC.csv', percent: 50, bytesDownloaded: 512 * 1024, totalBytes: 1024 * 1024, error: null}, 'Downloading NGC.csv: 512 KB / 1.0 MB'],
        [{stage: 'downloading', fileName: 'hyg.csv', percent: null, bytesDownloaded: 2048, totalBytes: null, error: null}, 'Downloading hyg.csv: 2 KB'],
        [{stage: 'file_completed', fileName: 'NGC.csv'}, ''],
        [{stage: 'completed', objectCount: 13957, error: null}, 'Catalog installed'],
        [{stage: 'failed', error: 'disk full'}, 'Failed: disk full'],
    ])('reads %o as "%s"', async (event, text) => {
        const {install, progress} = mountInstall()
        await flushPromises()

        progress.value = event
        await nextTick()

        expect(install.progressText.value).toBe(text)
    })

    it('stops on a failure and says why', async () => {
        const {install, progress} = mountInstall()
        await flushPromises()
        await install.startInstall(true)

        progress.value = {stage: 'failed', error: 'disk full'}
        await nextTick()

        expect(install.error.value).toBe('Installation failed: disk full')
        expect(install.installing.value).toBe(false)
    })

    it('reloads the status once the install settled, then reports it', async () => {
        vi.useFakeTimers()
        const onComplete = vi.fn()
        const {install, progress} = mountInstall({onComplete})
        await flushPromises()
        await install.startInstall(true)

        vi.mocked(getCatalogStatus).mockResolvedValue(catalogStatus())
        progress.value = {stage: 'completed', objectCount: 13957, error: null}
        await nextTick()
        expect(install.installing.value).toBe(true)
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)
        await flushPromises()

        expect(onComplete).toHaveBeenCalledWith(catalogStatus())
        expect(install.installing.value).toBe(false)
        expect(install.status.value?.installed).toBe(true)
    })

    it('drops the settle when it goes away first, and clears the progress it showed', async () => {
        vi.useFakeTimers()
        const {wrapper, install, progress, eventStream} = mountInstall()
        await flushPromises()
        await install.startInstall(true)
        progress.value = {stage: 'completed', objectCount: 13957, error: null}
        await nextTick()
        vi.mocked(getCatalogStatus).mockClear()

        wrapper.unmount()
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)

        expect(getCatalogStatus).not.toHaveBeenCalled()
        expect(eventStream.clearCatalogInstallProgress).toHaveBeenCalled()
    })
})

/** The events as the stream dispatches them, through the real install event table. */
describe('useCatalogInstall after a sync', () => {
    function mountWithEvents(options: InstallOptions<CatalogStatus | null> = {}) {
        const {handlers, state} = createInstallEvents()
        const send = <K extends ServerEventType>(type: K, fields: Partial<Omit<EventOf<K>, 'type'>>) =>
            sendTo(handlers, type, fields)
        return {...mountInstall(options, createEventStreamStub(state)), send}
    }

    beforeEach(() => {
        vi.clearAllMocks()
        vi.mocked(getCatalogStatus).mockResolvedValue(MISSING)
        vi.useFakeTimers()
    })
    afterEach(() => vi.useRealTimers())

    it('finishes an install whose end it missed on a completed status', async () => {
        const onComplete = vi.fn()
        const {install, send} = mountWithEvents({onComplete})
        await flushPromises()
        await install.startInstall(false)
        send('catalog_install_progress', {file_name: 'NGC.csv', bytes_downloaded: 300, total_bytes: 1200, percent: 25})
        await nextTick()

        vi.mocked(getCatalogStatus).mockResolvedValue(catalogStatus())
        send('catalog_install_status', {state: 'completed'})
        await nextTick()
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)
        await flushPromises()

        expect(onComplete).toHaveBeenCalledOnce()
        expect(onComplete).toHaveBeenCalledWith(catalogStatus())
        expect(install.installing.value).toBe(false)
    })

    // `idle` is what a restarted server says: read as completed, a restart showed
    // "Catalog installed", then the form came back with no word of why.
    it('says an install a server restart cut short did not finish', async () => {
        const onComplete = vi.fn()
        const {install, send} = mountWithEvents({onComplete})
        await flushPromises()
        await install.startInstall(false)
        send('catalog_install_progress', {file_name: 'NGC.csv', bytes_downloaded: 300, total_bytes: 1200, percent: 25})
        await nextTick()

        send('catalog_install_status', {state: 'idle'})
        await nextTick()
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)
        await flushPromises()

        expect(install.error.value).toBe(`Installation failed: ${INSTALL_INTERRUPTED}`)
        expect(install.installing.value).toBe(false)
        expect(install.progressText.value).not.toBe('Catalog installed')
        expect(onComplete).not.toHaveBeenCalled()
    })

    // The server says `idle` until the request reaches it.
    it('keeps an install whose request is still on the way through a sync saying idle', async () => {
        let accept!: (answer: Message) => void
        vi.mocked(installCatalog).mockImplementationOnce(() => new Promise((resolve) => (accept = resolve)))
        const {install, send} = mountWithEvents()
        await flushPromises()
        const started = install.startInstall(false)

        send('catalog_install_status', {state: 'idle'})
        await nextTick()
        accept({message: 'Catalog installation started.'})

        expect(await started).toBe(true)
        expect(install.installing.value).toBe(true)
        expect(install.error.value).toBeNull()
    })

    // A click on a second page meets the server's 409, then follows the install it names.
    it('follows the install another page is running after its own click is refused', async () => {
        vi.mocked(installCatalog).mockRejectedValueOnce(new ApiError('An installation is already running', 409))
        const {install, send} = mountWithEvents()
        await flushPromises()

        expect(await install.startInstall(false)).toBe(false)
        expect(install.error.value).toBe('An installation is already running')
        send('catalog_install_progress', {file_name: 'NGC.csv', bytes_downloaded: 600, total_bytes: 1200, percent: 50})
        await nextTick()

        expect(install.installing.value).toBe(true)
        expect(install.error.value).toBeNull()
    })

    it('shows the error of a failure it missed', async () => {
        const {install, send} = mountWithEvents()
        await flushPromises()
        await install.startInstall(true)

        send('catalog_install_status', {state: 'failed', error: 'disk full'})
        await nextTick()

        expect(install.error.value).toBe('Installation failed: disk full')
        expect(install.installing.value).toBe(false)
    })

    it('joins an install another page started', async () => {
        const {install, send} = mountWithEvents()
        await flushPromises()

        send('catalog_install_status', {
            state: 'running',
            progress: {type: 'catalog_install_progress', file_name: 'NGC.csv', bytes_downloaded: 2048, total_bytes: null, percent: null},
        })
        await nextTick()

        expect(install.installing.value).toBe(true)
        expect(install.canInstall.value).toBe(false)
        expect(install.progressText.value).toBe('Downloading NGC.csv: 2 KB')
    })

    it.each(['idle', 'completed', 'failed'] as const)('does nothing on a connect that finds it %s', async (state) => {
        const onComplete = vi.fn()
        const {install, send} = mountWithEvents({onComplete})
        await flushPromises()
        vi.mocked(getCatalogStatus).mockClear()

        send('catalog_install_status', {state, error: 'an old failure'})
        await nextTick()
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)
        await flushPromises()

        expect(onComplete).not.toHaveBeenCalled()
        expect(getCatalogStatus).not.toHaveBeenCalled()
        expect(install.installing.value).toBe(false)
        expect(install.error.value).toBeNull()
        expect(install.progressText.value).toBe('')
    })
})
