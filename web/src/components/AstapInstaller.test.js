import {flushPromises, mount} from '@vue/test-utils'
import {ref} from 'vue'
import AstapInstaller from './AstapInstaller.vue'
import {astapStatus, databaseType} from '../test/fixtures.js'

vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal()))

import {getAstapDatabases, getAstapStatus, installAstap} from '../composables/api.js'
import {contextProvides} from '../composables/appContext.js'

const FRESH = astapStatus({binary_installed: false, database_installed: false, installed_databases: [], ready: false})
const DATABASES = [databaseType(), databaseType({id: 'G05', description: 'Camera Lenses', min_fov_deg: 3, max_fov_deg: 20})]

function mountInstaller(status = FRESH, databases = DATABASES) {
    getAstapStatus.mockResolvedValue(status)
    getAstapDatabases.mockResolvedValue(databases)
    const progress = ref(null)
    const eventStream = {astapInstallProgress: progress, clearAstapInstallProgress: vi.fn()}
    const wrapper = mount(AstapInstaller, {global: {provide: contextProvides({eventStream})}})
    return {wrapper, progress}
}

describe('AstapInstaller', () => {
    afterEach(() => {
        vi.useRealTimers()
    })

    it('reports a ready solver with its databases', async () => {
        const {wrapper} = mountInstaller(astapStatus(), [databaseType({installed: true})])
        await flushPromises()

        expect(wrapper.text()).toContain('Installed (D80)')
        expect(wrapper.find('.section-number').text()).toBe('✓')
        expect(wrapper.emitted('installed')).toHaveLength(1)
        expect(wrapper.find('.btn-link').exists()).toBe(false)
    })

    it('offers more databases when some are not installed, and lets that be cancelled', async () => {
        const {wrapper} = mountInstaller(astapStatus(), [databaseType({installed: true}), DATABASES[1]])
        await flushPromises()

        await wrapper.find('.btn-link').trigger('click')
        expect(wrapper.text()).toContain('Select additional databases to download.')

        await wrapper.findAll('.btn-sm').find((b) => b.text() === 'Cancel').trigger('click')
        expect(wrapper.text()).toContain('Installed (D80)')
    })

    it('lists what is missing on a fresh install and installs the chosen databases', async () => {
        const {wrapper} = mountInstaller()
        await flushPromises()

        expect(wrapper.findAll('.status-item').map((row) => row.text())).toEqual(['✗ASTAP CLI', '✗Star Database'])
        await wrapper.find('.install-btn').trigger('click')

        expect(installAstap).toHaveBeenCalledWith(['D80'])
        expect(wrapper.emitted('state-change')).toEqual([[true]])
    })

    it('shows the progress events, then reloads once the last database is in', async () => {
        vi.useFakeTimers()
        const {wrapper, progress} = mountInstaller()
        await flushPromises()
        await wrapper.find('.install-btn').trigger('click')

        progress.value = {component: 'ASTAP CLI', stage: 'downloading', percent: 40, bytesDownloaded: 1024 * 1024, totalBytes: 2.5 * 1024 * 1024, overallPercent: 20}
        await flushPromises()
        expect(wrapper.text()).toContain('Downloading ASTAP CLI: 1.0 MB / 2.5 MB (40.0%)')
        expect(wrapper.find('.progress-fill').attributes('style')).toContain('width: 20%')

        getAstapStatus.mockResolvedValue(astapStatus())
        progress.value = {component: 'D80 Database', stage: 'completed', stageName: 'Database Installed', overallPercent: 100}
        await flushPromises()
        await vi.advanceTimersByTimeAsync(500)
        await flushPromises()

        expect(wrapper.emitted('state-change')).toEqual([[true], [false]])
        expect(wrapper.text()).toContain('Installed (D80)')
    })

    it('drops the status reload when it goes away before the install settled', async () => {
        vi.useFakeTimers()
        const {wrapper, progress} = mountInstaller()
        await flushPromises()
        await wrapper.find('.install-btn').trigger('click')
        progress.value = {component: 'D80 Database', stage: 'completed', stageName: 'Database Installed', overallPercent: 100}
        await flushPromises()
        getAstapStatus.mockClear()

        wrapper.unmount()
        await vi.advanceTimersByTimeAsync(500)

        expect(getAstapStatus).not.toHaveBeenCalled()
    })

    it('says why the status could not be read, and retries', async () => {
        getAstapStatus.mockRejectedValueOnce(new Error('Server unavailable'))
        getAstapDatabases.mockResolvedValue(DATABASES)
        const wrapper = mount(AstapInstaller, {global: {provide: contextProvides({eventStream: null})}})
        await flushPromises()
        expect(wrapper.find('.section-content.error').text()).toContain('Server unavailable')

        getAstapStatus.mockResolvedValue(astapStatus())
        await wrapper.find('.btn-sm').trigger('click')
        await flushPromises()
        expect(wrapper.text()).toContain('Installed')
    })
})
