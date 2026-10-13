import {defineComponent, h, nextTick, ref} from 'vue'
import {flushPromises, mount} from '@vue/test-utils'
import {catalogStatus} from '../test/fixtures.js'
import {INSTALL_SETTLE_MS} from './useAstapInstall.js'
import {useCatalogInstall} from './useCatalogInstall.js'

vi.mock('./api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal()))

import {getCatalogStatus, installCatalog} from './api.js'

const MISSING = catalogStatus({installed: false})

function mountInstall(options = {}) {
    const progress = ref(null)
    const eventStream = {catalogInstallProgress: progress, clearCatalogInstallProgress: vi.fn()}
    let install
    const Host = defineComponent({
        setup() {
            install = useCatalogInstall(options)
            return () => h('div')
        },
    })
    const wrapper = mount(Host, {global: {provide: {eventStream}}})
    return {wrapper, install, progress, eventStream}
}

describe('useCatalogInstall', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        getCatalogStatus.mockResolvedValue(MISSING)
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
        installCatalog.mockRejectedValueOnce(new Error('offline'))
        const {install} = mountInstall()
        await flushPromises()

        expect(await install.startInstall(true)).toBe(false)
        expect(install.error.value).toBe('offline')
        expect(install.canInstall.value).toBe(true)
    })

    it.each([
        [{stage: 'downloading', fileName: 'NGC.csv', percent: 50, bytesDownloaded: 512 * 1024, totalBytes: 1024 * 1024}, 'Downloading NGC.csv: 512 KB / 1.0 MB'],
        [{stage: 'downloading', fileName: 'hyg.csv', percent: null, bytesDownloaded: 2048, totalBytes: null}, 'Downloading hyg.csv: 2 KB'],
        [{stage: 'file_completed', fileName: 'NGC.csv'}, ''],
        [{stage: 'completed', objectCount: 13957}, 'Catalog installed'],
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

        getCatalogStatus.mockResolvedValue(catalogStatus())
        progress.value = {stage: 'completed', objectCount: 13957}
        await nextTick()
        expect(install.installing.value).toBe(true)
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)
        await flushPromises()

        expect(onComplete).toHaveBeenCalledWith(catalogStatus())
        expect(install.installing.value).toBe(false)
        expect(install.status.value.installed).toBe(true)
    })

    it('drops the settle when it goes away first, and clears the progress it showed', async () => {
        vi.useFakeTimers()
        const {wrapper, install, progress, eventStream} = mountInstall()
        await flushPromises()
        await install.startInstall(true)
        progress.value = {stage: 'completed', objectCount: 13957}
        await nextTick()
        getCatalogStatus.mockClear()

        wrapper.unmount()
        await vi.advanceTimersByTimeAsync(INSTALL_SETTLE_MS)

        expect(getCatalogStatus).not.toHaveBeenCalled()
        expect(eventStream.clearCatalogInstallProgress).toHaveBeenCalled()
    })
})
