import {flushPromises, mount} from '@vue/test-utils'
import EulaModal from './EulaModal.vue'

vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal(), {
            getSoftwareLicenses: vi.fn(async () => ({version: '1.0.0', core_license: 'CORE TEXT', third_party_licenses: 'THIRD PARTY'})),
    }))

import {getSoftwareLicenses, updateSettings} from '../composables/api.js'

async function mountModal() {
    const wrapper = mount(EulaModal)
    await flushPromises()
    return wrapper
}

describe('EulaModal', () => {
    beforeEach(() => vi.clearAllMocks())

    it('shows both license texts once loaded', async () => {
        const wrapper = await mountModal()

        expect(wrapper.find('#eula-core-license').text()).toBe('CORE TEXT')
        expect(wrapper.find('#eula-third-party').text()).toBe('THIRD PARTY')
    })

    it('leaves out the third-party section when there is none', async () => {
        getSoftwareLicenses.mockResolvedValueOnce({core_license: 'CORE TEXT', third_party_licenses: null})
        const wrapper = await mountModal()

        expect(wrapper.find('#eula-third-party').exists()).toBe(false)
    })

    it('accepts only once the box is ticked, and says so to the server', async () => {
        const wrapper = await mountModal()
        const accept = wrapper.find('#eula-accept-button')
        expect(accept.element.disabled).toBe(true)

        await wrapper.find('#eula-agree-checkbox').setValue(true)
        await accept.trigger('click')
        await flushPromises()

        expect(updateSettings).toHaveBeenCalledWith({eula_accepted: true})
        expect(wrapper.emitted('accepted')).toHaveLength(1)
    })

    it('stays open and says why when the server refuses', async () => {
        updateSettings.mockRejectedValueOnce(new Error('Read-only settings file'))
        const wrapper = await mountModal()

        await wrapper.find('#eula-agree-checkbox').setValue(true)
        await wrapper.find('#eula-accept-button').trigger('click')
        await flushPromises()

        expect(wrapper.find('.alert-error').text()).toContain('Read-only settings file')
        expect(wrapper.emitted('accepted')).toBeUndefined()
    })

    it('offers no Accept while the text failed to load, and retries on request', async () => {
        vi.spyOn(console, 'error').mockImplementation(() => {})
        getSoftwareLicenses.mockRejectedValueOnce(new Error('Server unavailable'))
        const wrapper = await mountModal()

        expect(wrapper.find('.alert-error').text()).toContain('Server unavailable')
        expect(wrapper.find('#eula-accept-button').exists()).toBe(false)
        expect(wrapper.find('#eula-agree-checkbox').exists()).toBe(false)

        await wrapper.find('#eula-retry-button').trigger('click')
        await flushPromises()

        expect(getSoftwareLicenses).toHaveBeenCalledTimes(2)
        expect(wrapper.find('.alert-error').exists()).toBe(false)
        expect(wrapper.find('#eula-core-license').text()).toBe('CORE TEXT')
    })

    it('keeps Accept disabled when the server sent an empty license', async () => {
        getSoftwareLicenses.mockResolvedValueOnce({core_license: '', third_party_licenses: null})
        const wrapper = await mountModal()

        expect(wrapper.find('#eula-accept-button').exists()).toBe(false)
        expect(wrapper.find('.alert-error').text()).toContain('no license text')
    })

    it('cannot be dismissed', async () => {
        const wrapper = await mountModal()
        expect(wrapper.findComponent({name: 'BaseModal'}).props('persistent')).toBe(true)
    })
})
