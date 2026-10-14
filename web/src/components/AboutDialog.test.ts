import {flushPromises, mount} from '@vue/test-utils'
import AboutDialog from './AboutDialog.vue'

vi.mock('../composables/api', async (importOriginal) =>
    (await import('../test/mockApi')).mockApi(await importOriginal(), {
            getLicenseStatus: vi.fn(async () => ({active: false, details: null})),
            getSoftwareLicenses: vi.fn(async () => ({version: '1.2.3', core_license: 'CORE TEXT', third_party_licenses: 'THIRD PARTY'})),
    }))

import {getLicenseStatus, getSoftwareLicenses, updateLicense} from '../composables/api'
import type {LicenseStatus} from '../composables/api.types'

const ACTIVE: LicenseStatus = {
    active: true,
    details: {
        name: 'Ada Observer',
        email: 'ada@example.org',
        issued_at: '2026-01-01T00:00:00Z',
        expires_at: '2099-01-01T00:00:00Z',
    },
}

async function mountDialog() {
    const wrapper = mount(AboutDialog)
    await flushPromises()
    return wrapper
}

describe('AboutDialog', () => {
    afterEach(() => {
        vi.restoreAllMocks()
    })

    it('names the version and the Community edition without a licence', async () => {
        const wrapper = await mountDialog()

        expect(wrapper.find('.version-badge').text()).toBe('v1.2.3')
        expect(wrapper.find('.status-badge.warning').text()).toContain('Pro License Not Active')
    })

    it('shows who an active licence belongs to and how long it has left', async () => {
        vi.mocked(getLicenseStatus).mockResolvedValueOnce(ACTIVE)
        const wrapper = await mountDialog()

        const card = wrapper.find('.active-license-card')
        expect(card.text()).toContain('Ada Observer')
        expect(card.text()).toContain('ada@example.org')
        expect(card.find('.days-left').text()).toMatch(/\(\d+ days left\)/)
    })

    it('updates the licence from a pasted token and clears the field', async () => {
        vi.mocked(updateLicense).mockResolvedValueOnce(ACTIVE)
        const wrapper = await mountDialog()

        const button = wrapper.find<HTMLButtonElement>('.update-btn')
        expect(button.element.disabled).toBe(true)
        await wrapper.find('.license-input').setValue('  token-123  ')
        await button.trigger('click')
        await flushPromises()

        expect(updateLicense).toHaveBeenCalledWith('token-123')
        expect(wrapper.find('.alert-success').exists()).toBe(true)
        expect(wrapper.find<HTMLTextAreaElement>('.license-input').element.value).toBe('')
        expect(wrapper.find('.active-license-card').exists()).toBe(true)
    })

    it('says why a token was refused', async () => {
        vi.mocked(updateLicense).mockRejectedValueOnce(new Error('Invalid signature'))
        const wrapper = await mountDialog()

        await wrapper.find('.license-input').setValue('bad')
        await wrapper.find('.update-btn').trigger('click')
        await flushPromises()

        expect(wrapper.find('.alert-error').text()).toContain('Invalid signature')
    })

    it('shows the licence status error when it cannot be read', async () => {
        vi.mocked(getLicenseStatus).mockRejectedValueOnce(new Error('Server unavailable'))
        const wrapper = await mountDialog()

        expect(wrapper.find('.alert-error').text()).toContain('Server unavailable')
    })

    it('shows the software licences on their tab', async () => {
        const wrapper = await mountDialog()
        await wrapper.findAll('.tabs .btn-option')[1].trigger('click')

        const texts = wrapper.findAll('.license-text').map((pre) => pre.text())
        expect(texts).toEqual(['CORE TEXT', 'THIRD PARTY'])
    })

    // The tab used to show two empty boxes and only log the failure.
    it('says why the software licences did not load, and retries', async () => {
        vi.mocked(getSoftwareLicenses).mockRejectedValueOnce(new Error('offline'))
        const wrapper = await mountDialog()
        await wrapper.findAll('.tabs .btn-option')[1].trigger('click')

        expect(wrapper.find('.version-badge').exists()).toBe(false)
        expect(wrapper.find('.alert-error').text()).toContain('offline')

        await wrapper.find('.retry-btn').trigger('click')
        await flushPromises()

        expect(wrapper.findAll('.license-text').map((pre) => pre.text())).toEqual(['CORE TEXT', 'THIRD PARTY'])
        expect(wrapper.find('.version-badge').text()).toBe('v1.2.3')
    })

    it('marks the active tab', async () => {
        const wrapper = await mountDialog()
        const tabs = () => wrapper.findAll('.tabs .btn-option').map((tab) => tab.classes().includes('active'))

        expect(tabs()).toEqual([true, false])
        await wrapper.findAll('.tabs .btn-option')[1].trigger('click')
        expect(tabs()).toEqual([false, true])
    })

    it('closes from its own close button', async () => {
        const wrapper = await mountDialog()
        await wrapper.find('.btn-close').trigger('click')
        expect(wrapper.emitted('close')).toHaveLength(1)
    })
})
