import {defineComponent, h, nextTick, ref} from 'vue'
import {flushPromises, mount} from '@vue/test-utils'
import {astapStatus, databaseType} from '../test/fixtures.js'
import {DEFAULT_ASTAP_DATABASE, fetchAstapState, INSTALL_SETTLE_MS, useAstapInstall} from './useAstapInstall.js'
import {createInstallEvents, INSTALL_INTERRUPTED} from './events/installEvents.js'

vi.mock('./api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal()))

import {ApiError, getAstapDatabases, getAstapStatus, installAstap} from './api.js'

const NOT_INSTALLED = astapStatus({binary_installed: false, database_installed: false, installed_databases: [], ready: false})

function mountInstall(options = {}, eventStream = null) {
    const progress = ref(null)
    eventStream ??= {astapInstallProgress: progress, clearAstapInstallProgress: vi.fn()}
    let install
    const Host = defineComponent({
        setup() {
            install = useAstapInstall(options)
            return () => h('div')
        },
    })
    const wrapper = mount(Host, {global: {provide: {eventStream}}})
    return {wrapper, install, progress, eventStream}
}

describe('fetchAstapState', () => {
    it('fetches the status and the databases together', async () => {
        getAstapStatus.mockResolvedValueOnce(NOT_INSTALLED)
        getAstapDatabases.mockResolvedValueOnce([databaseType()])

        expect(await fetchAstapState()).toEqual({status: NOT_INSTALLED, databases: [databaseType()]})
    })
})

describe('useAstapInstall', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        getAstapStatus.mockResolvedValue(NOT_INSTALLED)
        getAstapDatabases.mockResolvedValue([databaseType(), databaseType({id: 'G05', installed: false})])
    })
    afterEach(() => vi.useRealTimers())

    it('loads the status on mount and offers the default database', async () => {
        const {install} = mountInstall()
        await flushPromises()

        expect(install.loading.value).toBe(false)
        expect(install.status.value).toEqual(NOT_INSTALLED)
        expect(install.selectedDatabases.value).toEqual([DEFAULT_ASTAP_DATABASE])
        expect(install.hasUninstalledDatabases.value).toBe(true)
    })

    it('reports a status that failed to load', async () => {
        getAstapStatus.mockRejectedValueOnce(new Error('Server unavailable'))
        const {install} = mountInstall()
        await flushPromises()

        expect(install.error.value).toBe('Server unavailable')
    })

    it('needs a database to install, and refuses a second install while one runs', async () => {
        const {install} = mountInstall()
        await flushPromises()

        install.selectedDatabases.value = []
        expect(await install.startInstall()).toBe(false)

        install.selectedDatabases.value = ['D80', 'G05']
        expect(await install.startInstall()).toBe(true)
        expect(await install.startInstall()).toBe(false)
        expect(installAstap).toHaveBeenCalledTimes(1)
        expect(installAstap).toHaveBeenCalledWith(['D80', 'G05'])
    })

    it('says why the install request failed and can be tried again', async () => {
        installAstap.mockRejectedValueOnce(new Error('offline'))
        const {install} = mountInstall()
        await flushPromises()

        expect(await install.startInstall()).toBe(false)
        expect(install.error.value).toBe('offline')
        expect(install.installing.value).toBe(false)
    })

    it.each([
        [{stage: 'starting', component: 'ASTAP CLI'}, 'Installing ASTAP...', null],
        [{stage: 'starting', component: 'D80 Database'}, 'Downloading D80 star database...', null],
        [{stage: 'downloading', component: 'D80 Database', percent: 50, bytesDownloaded: 512 * 1024, totalBytes: 1024 * 1024}, 'Downloading D80 Database: 512 KB / 1.0 MB (50.0%)', 50],
        [{stage: 'downloading', component: 'D80 Database', percent: 25, totalBytes: null}, 'Downloading D80 Database: 25.0%', 25],
        [{stage: 'extracting', component: 'ASTAP CLI', percent: 40}, 'Extracting ASTAP CLI: 40.0%', 40],
        [{stage: 'extracting', component: 'ASTAP CLI', percent: null}, 'Extracting ASTAP CLI...', null],
        [{stage: 'completed', component: 'D80 Database'}, 'D80 Database installed successfully', 100],
        [{stage: 'downloading', component: 'D80 Database', percent: 5, overallPercent: 52}, 'Downloading D80 Database: 5.0%', 52],
    ])('reads %o as "%s"', async (event, text, percent) => {
        const {install, progress} = mountInstall()
        await flushPromises()

        progress.value = {percent: null, totalBytes: null, ...event}
        await nextTick()

        expect(install.progressText.value).toBe(text)
        expect(install.progressPercent.value).toBe(percent)
    })

    it('tracks which stages finished', async () => {
        const {install, progress} = mountInstall()
        await flushPromises()

        progress.value = {stage: 'completed', stageName: 'ASTAP CLI Installed', component: 'ASTAP CLI'}
        await nextTick()
        progress.value = {stage: 'completed', stageName: 'Database Installed', component: 'G05 Database'}
        await nextTick()

        expect(install.stageCompletion.value.cliCompleted).toBe(true)
        expect([...install.stageCompletion.value.completedDatabases]).toEqual(['G05 Database'])
    })

    it('stops on a failed stage and says why', async () => {
        const {install, progress} = mountInstall()
        await flushPromises()
        await install.startInstall()

        progress.value = {stage: 'failed', component: 'D80 Database', error: 'Download timeout'}
        await nextTick()

        expect(install.error.value).toBe('Installation failed: Download timeout')
        expect(install.installing.value).toBe(false)
    })

    it('is complete with the last selected database, then reloads and reports', async () => {
        vi.useFakeTimers()
        const onComplete = vi.fn()
        const {install, progress} = mountInstall({onComplete})
        await flushPromises()
        install.selectedDatabases.value = ['D80', 'G05']
        await install.startInstall()

        progress.value = {stage: 'completed', stageName: 'Database Installed', component: 'D80 Database'}
        await nextTick()
        expect(install.installComplete.value).toBe(false)

        getAstapStatus.mockResolvedValue(astapStatus())
        progress.value = {stage: 'completed', stageName: 'Database Installed', component: 'G05 Database'}
        await nextTick()
        expect(install.installComplete.value).toBe(true)
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS - 1)
        expect(onComplete).not.toHaveBeenCalled()

        await vi.advanceTimersByTimeAsync(1)
        await flushPromises()

        expect(onComplete).toHaveBeenCalledWith(astapStatus())
        expect(install.installing.value).toBe(false)
    })

    it('drops the settle when it goes away first, and clears the progress it showed', async () => {
        vi.useFakeTimers()
        const onComplete = vi.fn()
        const {wrapper, install, progress, eventStream} = mountInstall({onComplete})
        await flushPromises()
        await install.startInstall()
        progress.value = {stage: 'completed', component: 'D80 Database'}
        await nextTick()
        getAstapStatus.mockClear()

        wrapper.unmount()
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)

        expect(getAstapStatus).not.toHaveBeenCalled()
        expect(onComplete).not.toHaveBeenCalled()
        expect(eventStream.clearAstapInstallProgress).toHaveBeenCalled()
    })
})

/** The events as the stream dispatches them, through the real install event table. */
describe('useAstapInstall after a sync', () => {
    function mountWithEvents(options = {}) {
        const {handlers, state} = createInstallEvents()
        return {...mountInstall(options, state), handlers}
    }

    beforeEach(() => {
        vi.clearAllMocks()
        getAstapStatus.mockResolvedValue(NOT_INSTALLED)
        getAstapDatabases.mockResolvedValue([databaseType()])
        vi.useFakeTimers()
    })
    afterEach(() => vi.useRealTimers())

    it('finishes an install whose end it missed on a completed status', async () => {
        const onComplete = vi.fn()
        const {install, handlers} = mountWithEvents({onComplete})
        await flushPromises()
        install.selectedDatabases.value = ['D80', 'G05']
        await install.startInstall()
        handlers.astap_install_completed({component: 'D80 Database', stage: 'Database Installed', overall_percent: 50})
        await nextTick()

        getAstapStatus.mockResolvedValue(astapStatus())
        handlers.astap_install_status({state: 'completed'})
        await nextTick()
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)
        await flushPromises()

        expect(onComplete).toHaveBeenCalledOnce()
        expect(onComplete).toHaveBeenCalledWith(astapStatus())
        expect(install.installing.value).toBe(false)
    })

    it('says an install a server restart cut short did not finish', async () => {
        const onComplete = vi.fn()
        const {install, handlers} = mountWithEvents({onComplete})
        await flushPromises()
        await install.startInstall()
        handlers.astap_install_completed({component: 'ASTAP CLI', stage: 'ASTAP CLI Installed', overall_percent: 20})
        await nextTick()

        handlers.astap_install_status({state: 'idle'})
        await nextTick()
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)
        await flushPromises()

        expect(install.error.value).toBe(`Installation failed: ${INSTALL_INTERRUPTED}`)
        expect(install.installing.value).toBe(false)
        expect(install.installComplete.value).toBe(false)
        expect(onComplete).not.toHaveBeenCalled()
    })

    it('keeps an install whose request is still on the way through a sync saying idle', async () => {
        let accept
        installAstap.mockImplementationOnce(() => new Promise((resolve) => (accept = resolve)))
        const {install, handlers} = mountWithEvents()
        await flushPromises()
        const started = install.startInstall()

        handlers.astap_install_status({state: 'idle'})
        await nextTick()
        accept({message: 'Installation started.'})

        expect(await started).toBe(true)
        expect(install.installing.value).toBe(true)
        expect(install.error.value).toBeNull()
    })

    it('follows the install another page is running after its own click is refused', async () => {
        installAstap.mockRejectedValueOnce(new ApiError('An installation is already running', 409))
        const {install, handlers} = mountWithEvents()
        await flushPromises()

        expect(await install.startInstall()).toBe(false)
        handlers.astap_install_progress({
            component: 'D80 Database', bytes_downloaded: 1024, total_bytes: 4096, percent: 25,
            stage: 'Downloading Database', overall_percent: 40,
        })
        await nextTick()

        expect(install.installing.value).toBe(true)
        expect(install.error.value).toBeNull()
    })

    it('shows the error of a failure it missed', async () => {
        const {install, handlers} = mountWithEvents()
        await flushPromises()
        await install.startInstall()

        handlers.astap_install_status({state: 'failed', error: 'network'})
        await nextTick()

        expect(install.error.value).toBe('Installation failed: network')
        expect(install.installing.value).toBe(false)
    })

    it('joins an install another page started, and finishes with it', async () => {
        const onComplete = vi.fn()
        const {install, handlers} = mountWithEvents({onComplete})
        await flushPromises()

        handlers.astap_install_status({
            state: 'running',
            progress: {
                type: 'astap_install_progress', component: 'G05 Database', bytes_downloaded: 512 * 1024,
                total_bytes: 1024 * 1024, percent: 50, stage: 'Downloading Database', overall_percent: 60,
            },
        })
        await nextTick()

        expect(install.installing.value).toBe(true)
        expect(install.canInstall.value).toBe(false)
        expect(install.progressText.value).toBe('Downloading G05 Database: 512 KB / 1.0 MB (50.0%)')
        expect(install.progressPercent.value).toBe(60)

        getAstapStatus.mockResolvedValue(astapStatus())
        handlers.astap_install_completed({component: 'G05 Database', stage: 'Database Installed', overall_percent: 100})
        handlers.astap_install_status({state: 'completed'})
        await nextTick()
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)
        await flushPromises()

        expect(onComplete).toHaveBeenCalledOnce()
        expect(install.installing.value).toBe(false)
    })

    it.each(['idle', 'completed', 'failed'])('does nothing on a connect that finds it %s', async (state) => {
        const onComplete = vi.fn()
        const {install, handlers} = mountWithEvents({onComplete})
        await flushPromises()
        getAstapStatus.mockClear()

        handlers.astap_install_status({state, error: 'an old failure'})
        await nextTick()
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)
        await flushPromises()

        expect(onComplete).not.toHaveBeenCalled()
        expect(getAstapStatus).not.toHaveBeenCalled()
        expect(install.installing.value).toBe(false)
        expect(install.error.value).toBeNull()
        expect(install.progressText.value).toBe('')
    })
})
