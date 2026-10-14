import {defineComponent, h, nextTick} from 'vue'
import {flushPromises, mount} from '@vue/test-utils'
import {astapStatus, databaseType} from '../test/fixtures'
import {createEventStreamStub, sendTo} from '../test/eventStream'
import {DEFAULT_ASTAP_DATABASE, fetchAstapState, INSTALL_SETTLE_MS, useAstapInstall} from './useAstapInstall'
import type {AstapInstall, InstallOptions} from './useAstapInstall'
import {createInstallEvents, INSTALL_INTERRUPTED} from './events/installEvents'
import type {AstapInstallProgress} from './events/installEvents'
import type {AstapStatus, Message} from './api.types'
import type {EventOf, ServerEventType} from './api.events'

vi.mock('./api', async (importOriginal) =>
    (await import('../test/mockApi')).mockApi(await importOriginal()))

import {ApiError, getAstapDatabases, getAstapStatus, installAstap} from './api'
import {contextProvides} from './appContext'

const NOT_INSTALLED = astapStatus({binary_installed: false, database_installed: false, installed_databases: [], ready: false})

/** The fields of a stage with no download yet. */
const NO_TRANSFER = {percent: null, bytesDownloaded: 0, totalBytes: null, error: null} as const

function mountInstall(options: InstallOptions<AstapStatus | null> = {}, eventStream = createEventStreamStub()) {
    const progress = eventStream.astapInstallProgress
    let install!: AstapInstall
    const Host = defineComponent({
        setup() {
            install = useAstapInstall(options)
            return () => h('div')
        },
    })
    const wrapper = mount(Host, {global: {provide: contextProvides({eventStream})}})
    return {wrapper, install, progress, eventStream}
}

describe('fetchAstapState', () => {
    it('fetches the status and the databases together', async () => {
        vi.mocked(getAstapStatus).mockResolvedValueOnce(NOT_INSTALLED)
        vi.mocked(getAstapDatabases).mockResolvedValueOnce([databaseType()])

        expect(await fetchAstapState()).toEqual({status: NOT_INSTALLED, databases: [databaseType()]})
    })
})

describe('useAstapInstall', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        vi.mocked(getAstapStatus).mockResolvedValue(NOT_INSTALLED)
        vi.mocked(getAstapDatabases).mockResolvedValue([databaseType(), databaseType({id: 'G05', installed: false})])
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
        vi.mocked(getAstapStatus).mockRejectedValueOnce(new Error('Server unavailable'))
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
        vi.mocked(installAstap).mockRejectedValueOnce(new Error('offline'))
        const {install} = mountInstall()
        await flushPromises()

        expect(await install.startInstall()).toBe(false)
        expect(install.error.value).toBe('offline')
        expect(install.installing.value).toBe(false)
    })

    it.each<[AstapInstallProgress, string, number | null]>([
        [{stage: 'starting', component: 'ASTAP CLI', ...NO_TRANSFER}, 'Installing ASTAP...', null],
        [{stage: 'starting', component: 'D80 Database', ...NO_TRANSFER}, 'Downloading D80 star database...', null],
        [{stage: 'downloading', component: 'D80 Database', ...NO_TRANSFER, percent: 50, bytesDownloaded: 512 * 1024, totalBytes: 1024 * 1024}, 'Downloading D80 Database: 512 KB / 1.0 MB (50.0%)', 50],
        [{stage: 'downloading', component: 'D80 Database', ...NO_TRANSFER, percent: 25}, 'Downloading D80 Database: 25.0%', 25],
        [{stage: 'extracting', component: 'ASTAP CLI', percent: 40, error: null}, 'Extracting ASTAP CLI: 40.0%', 40],
        [{stage: 'completed', component: 'D80 Database', error: null}, 'D80 Database installed successfully', 100],
        [{stage: 'downloading', component: 'D80 Database', ...NO_TRANSFER, percent: 5, overallPercent: 52}, 'Downloading D80 Database: 5.0%', 52],
    ])('reads %o as "%s"', async (event, text, percent) => {
        const {install, progress} = mountInstall()
        await flushPromises()

        progress.value = event
        await nextTick()

        expect(install.progressText.value).toBe(text)
        expect(install.progressPercent.value).toBe(percent)
    })

    // The event table only reads an extraction with its percent (`progress` is required).
    it('takes no extraction without a percent', () => {
        expectTypeOf<Extract<AstapInstallProgress, {stage: 'extracting'}>['percent']>().toEqualTypeOf<number>()
        // @ts-expect-error an extraction always says how far it got
        const extracting: AstapInstallProgress = {stage: 'extracting', component: 'ASTAP CLI', percent: null, error: null}
        expect(extracting.stage).toBe('extracting')
    })

    it('tracks which stages finished', async () => {
        const {install, progress} = mountInstall()
        await flushPromises()

        progress.value = {stage: 'completed', stageId: 'cli_completed', component: 'ASTAP CLI', error: null}
        await nextTick()
        progress.value = {stage: 'completed', stageId: 'database_completed', component: 'G05 Database', error: null}
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

        progress.value = {stage: 'completed', stageId: 'database_completed', component: 'D80 Database', error: null}
        await nextTick()
        expect(install.installComplete.value).toBe(false)

        vi.mocked(getAstapStatus).mockResolvedValue(astapStatus())
        progress.value = {stage: 'completed', stageId: 'database_completed', component: 'G05 Database', error: null}
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
        progress.value = {stage: 'completed', component: 'D80 Database', error: null}
        await nextTick()
        vi.mocked(getAstapStatus).mockClear()

        wrapper.unmount()
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)

        expect(getAstapStatus).not.toHaveBeenCalled()
        expect(onComplete).not.toHaveBeenCalled()
        expect(eventStream.clearAstapInstallProgress).toHaveBeenCalled()
    })
})

/** The events as the stream dispatches them, through the real install event table. */
describe('useAstapInstall after a sync', () => {
    function mountWithEvents(options: InstallOptions<AstapStatus | null> = {}) {
        const {handlers, state} = createInstallEvents()
        const send = <K extends ServerEventType>(type: K, fields: Partial<Omit<EventOf<K>, 'type'>>) =>
            sendTo(handlers, type, fields)
        return {...mountInstall(options, createEventStreamStub(state)), send}
    }

    beforeEach(() => {
        vi.clearAllMocks()
        vi.mocked(getAstapStatus).mockResolvedValue(NOT_INSTALLED)
        vi.mocked(getAstapDatabases).mockResolvedValue([databaseType()])
        vi.useFakeTimers()
    })
    afterEach(() => vi.useRealTimers())

    it('finishes an install whose end it missed on a completed status', async () => {
        const onComplete = vi.fn()
        const {install, send} = mountWithEvents({onComplete})
        await flushPromises()
        install.selectedDatabases.value = ['D80', 'G05']
        await install.startInstall()
        send('astap_install_completed', {component: 'D80 Database', stage: 'Database Installed', stage_id: 'database_completed', overall_percent: 50})
        await nextTick()

        vi.mocked(getAstapStatus).mockResolvedValue(astapStatus())
        send('astap_install_status', {state: 'completed'})
        await nextTick()
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)
        await flushPromises()

        expect(onComplete).toHaveBeenCalledOnce()
        expect(onComplete).toHaveBeenCalledWith(astapStatus())
        expect(install.installing.value).toBe(false)
    })

    // The label is copy the server may reword; the id is what the UI branches on.
    it('follows the stage ids, whatever the labels say', async () => {
        const {install, send} = mountWithEvents()
        await flushPromises()
        await install.startInstall()

        send('astap_install_completed', {component: 'ASTAP CLI', stage: 'CLI ready', stage_id: 'cli_completed'})
        await nextTick()
        expect(install.stageCompletion.value).toMatchObject({cliExtracted: true, cliCompleted: true})

        send('astap_install_completed', {component: 'D80 Database', stage: 'Database ready', stage_id: 'database_completed'})
        await nextTick()
        expect([...install.stageCompletion.value.completedDatabases]).toEqual(['D80 Database'])

        send('astap_install_completed', {component: 'G05 Database', stage: 'Database Installed', stage_id: null})
        await nextTick()
        expect([...install.stageCompletion.value.completedDatabases]).toEqual(['D80 Database'])
    })

    it('says an install a server restart cut short did not finish', async () => {
        const onComplete = vi.fn()
        const {install, send} = mountWithEvents({onComplete})
        await flushPromises()
        await install.startInstall()
        send('astap_install_completed', {component: 'ASTAP CLI', stage: 'ASTAP CLI Installed', stage_id: 'cli_completed', overall_percent: 20})
        await nextTick()

        send('astap_install_status', {state: 'idle'})
        await nextTick()
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)
        await flushPromises()

        expect(install.error.value).toBe(`Installation failed: ${INSTALL_INTERRUPTED}`)
        expect(install.installing.value).toBe(false)
        expect(install.installComplete.value).toBe(false)
        expect(onComplete).not.toHaveBeenCalled()
    })

    it('keeps an install whose request is still on the way through a sync saying idle', async () => {
        let accept!: (answer: Message) => void
        vi.mocked(installAstap).mockImplementationOnce(() => new Promise((resolve) => (accept = resolve)))
        const {install, send} = mountWithEvents()
        await flushPromises()
        const started = install.startInstall()

        send('astap_install_status', {state: 'idle'})
        await nextTick()
        accept({message: 'Installation started.'})

        expect(await started).toBe(true)
        expect(install.installing.value).toBe(true)
        expect(install.error.value).toBeNull()
    })

    it('follows the install another page is running after its own click is refused', async () => {
        vi.mocked(installAstap).mockRejectedValueOnce(new ApiError('An installation is already running', 409))
        const {install, send} = mountWithEvents()
        await flushPromises()

        expect(await install.startInstall()).toBe(false)
        send('astap_install_progress', {
            component: 'D80 Database', bytes_downloaded: 1024, total_bytes: 4096, percent: 25,
            stage: 'Downloading Database', stage_id: 'downloading_database', overall_percent: 40,
        })
        await nextTick()

        expect(install.installing.value).toBe(true)
        expect(install.error.value).toBeNull()
    })

    it('shows the error of a failure it missed', async () => {
        const {install, send} = mountWithEvents()
        await flushPromises()
        await install.startInstall()

        send('astap_install_status', {state: 'failed', error: 'network'})
        await nextTick()

        expect(install.error.value).toBe('Installation failed: network')
        expect(install.installing.value).toBe(false)
    })

    it('joins an install another page started, and finishes with it', async () => {
        const onComplete = vi.fn()
        const {install, send} = mountWithEvents({onComplete})
        await flushPromises()

        send('astap_install_status', {
            state: 'running',
            progress: {
                type: 'astap_install_progress', component: 'G05 Database', bytes_downloaded: 512 * 1024,
                total_bytes: 1024 * 1024, percent: 50, stage: 'Downloading Database', stage_id: 'downloading_database', overall_percent: 60,
            },
        })
        await nextTick()

        expect(install.installing.value).toBe(true)
        expect(install.canInstall.value).toBe(false)
        expect(install.progressText.value).toBe('Downloading G05 Database: 512 KB / 1.0 MB (50.0%)')
        expect(install.progressPercent.value).toBe(60)

        vi.mocked(getAstapStatus).mockResolvedValue(astapStatus())
        send('astap_install_completed', {component: 'G05 Database', stage: 'Database Installed', stage_id: 'database_completed', overall_percent: 100})
        send('astap_install_status', {state: 'completed'})
        await nextTick()
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)
        await flushPromises()

        expect(onComplete).toHaveBeenCalledOnce()
        expect(install.installing.value).toBe(false)
    })

    it.each(['idle', 'completed', 'failed'] as const)('does nothing on a connect that finds it %s', async (state) => {
        const onComplete = vi.fn()
        const {install, send} = mountWithEvents({onComplete})
        await flushPromises()
        vi.mocked(getAstapStatus).mockClear()

        send('astap_install_status', {state, error: 'an old failure'})
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
