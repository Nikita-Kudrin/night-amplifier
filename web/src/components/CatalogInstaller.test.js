import {flushPromises, mount} from '@vue/test-utils'
import {ref} from 'vue'
import CatalogInstaller from './CatalogInstaller.vue'
import {catalogStatus} from '../test/fixtures.js'

vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal()))

import {getCatalogStatus, installCatalog} from '../composables/api.js'
import {contextProvides} from '../composables/appContext.js'

const MISSING = catalogStatus({installed: false, ngc_file_exists: true, addendum_file_exists: false, hyg_file_exists: false})

function mountInstaller(status = MISSING) {
    getCatalogStatus.mockResolvedValue(status)
    const progress = ref(null)
    const eventStream = {catalogInstallProgress: progress, clearCatalogInstallProgress: vi.fn()}
    const wrapper = mount(CatalogInstaller, {global: {provide: contextProvides({eventStream})}})
    return {wrapper, progress, eventStream}
}

describe('CatalogInstaller', () => {
    afterEach(() => {
        vi.useRealTimers()
    })

    it('reports an installed catalog and offers nothing to install', async () => {
        const {wrapper} = mountInstaller(catalogStatus())
        await flushPromises()

        expect(wrapper.text()).toContain('Installed (Target Catalogs)')
        expect(wrapper.find('.section-number').text()).toBe('✓')
        expect(wrapper.emitted('installed')).toHaveLength(1)
        expect(wrapper.find('.btn-primary').exists()).toBe(false)
    })

    it('lists which files are there before an install', async () => {
        const {wrapper} = mountInstaller()
        await flushPromises()

        const rows = wrapper.findAll('.status-item').map((row) => row.text())
        expect(rows).toEqual(['✓NGC.csv', '✗addendum.csv', '✗hyg_stars.csv'])
        expect(wrapper.find('.section-number').text()).toBe('2')
        expect(wrapper.emitted('installed')).toBeUndefined()
    })

    it('installs with the star catalog unless it is unticked', async () => {
        const {wrapper} = mountInstaller()
        await flushPromises()

        await wrapper.find('input[type="checkbox"]').setValue(false)
        await wrapper.find('.btn-primary').trigger('click')

        expect(installCatalog).toHaveBeenCalledWith(false)
        expect(wrapper.emitted('state-change')).toEqual([[true]])
        expect(wrapper.text()).toContain('Starting download...')
    })

    it('shows download progress from the event stream', async () => {
        const {wrapper, progress} = mountInstaller()
        await flushPromises()
        await wrapper.find('.btn-primary').trigger('click')

        progress.value = {stage: 'downloading', fileName: 'NGC.csv', percent: 50, bytesDownloaded: 512 * 1024, totalBytes: 1024 * 1024}
        await flushPromises()

        expect(wrapper.text()).toContain('Downloading NGC.csv: 512 KB / 1.0 MB')
        expect(wrapper.find('.progress-fill').attributes('style')).toContain('width: 50%')

        progress.value = {stage: 'downloading', fileName: 'hyg.csv', percent: null, bytesDownloaded: 2048}
        await flushPromises()
        expect(wrapper.text()).toContain('Downloading hyg.csv: 2 KB')
        expect(wrapper.find('.progress-bar').exists()).toBe(false)
    })

    it('reloads the status once the install completes', async () => {
        vi.useFakeTimers()
        const {wrapper, progress} = mountInstaller()
        await flushPromises()
        await wrapper.find('.btn-primary').trigger('click')

        getCatalogStatus.mockResolvedValue(catalogStatus())
        progress.value = {stage: 'completed', objectCount: 13957}
        await flushPromises()
        expect(wrapper.text()).toContain('Catalog installed')

        await vi.advanceTimersByTimeAsync(500)
        await flushPromises()

        expect(wrapper.emitted('state-change')).toEqual([[true], [false]])
        expect(wrapper.emitted('installed')).toHaveLength(1)
        expect(wrapper.text()).toContain('Installed (Target Catalogs)')
    })

    it('drops the status reload when it goes away before the install settled', async () => {
        vi.useFakeTimers()
        const {wrapper, progress} = mountInstaller()
        await flushPromises()
        await wrapper.find('.btn-primary').trigger('click')
        progress.value = {stage: 'completed', objectCount: 13957}
        await flushPromises()
        getCatalogStatus.mockClear()

        wrapper.unmount()
        await vi.advanceTimersByTimeAsync(500)

        expect(getCatalogStatus).not.toHaveBeenCalled()
    })

    it('says why an install failed, from the event or the request', async () => {
        const {wrapper, progress} = mountInstaller()
        await flushPromises()
        await wrapper.find('.btn-primary').trigger('click')

        progress.value = {stage: 'failed', error: 'disk full'}
        await flushPromises()
        expect(wrapper.find('.section-content.error').text()).toContain('Installation failed: disk full')
        expect(wrapper.emitted('state-change')).toEqual([[true], [false]])

        installCatalog.mockRejectedValueOnce(new Error('offline'))
        await wrapper.find('.btn').trigger('click') // Retry reloads the status
        await flushPromises()
        await wrapper.find('.btn-primary').trigger('click')
        await flushPromises()
        expect(wrapper.find('.section-content.error').text()).toContain('offline')
    })

    it('retries a status that failed to load', async () => {
        getCatalogStatus.mockRejectedValueOnce(new Error('Server unavailable'))
        const {wrapper} = mountInstaller()
        await flushPromises()
        expect(wrapper.text()).toContain('Server unavailable')

        getCatalogStatus.mockResolvedValue(catalogStatus())
        await wrapper.find('.btn-sm').trigger('click')
        await flushPromises()
        expect(wrapper.text()).toContain('Installed (Target Catalogs)')
    })

    it('clears the progress it showed when it goes away', async () => {
        const {wrapper, eventStream} = mountInstaller()
        await flushPromises()
        wrapper.unmount()
        expect(eventStream.clearCatalogInstallProgress).toHaveBeenCalled()
    })
})
