import {ref} from 'vue'
import {tryOnMounted} from './lifecycle'
import {getSoftwareLicenses} from './api'
import {useError} from './useError'

/**
 * The build's version and licence texts, loaded on mount — About and the EULA both show
 * them. A server that answers without the core licence has not answered: agreeing to a
 * text never shown is no agreement, so that is an error too.
 */
export function useSoftwareLicenses() {
    const version = ref('')
    const coreLicense = ref('')
    const thirdPartyLicenses = ref('')
    const {error, loading, setError, withErrorHandling} = useError()
    // Loading from the start: the texts are asked for on mount, and nothing is shown before.
    loading.value = true

    async function load() {
        await withErrorHandling(async () => {
            const res = await getSoftwareLicenses()
            version.value = res.version
            coreLicense.value = res.core_license
            thirdPartyLicenses.value = res.third_party_licenses ?? ''
        })
        if (!error.value && !coreLicense.value) setError('The server sent no license text.')
    }

    tryOnMounted(() => void load())

    return {version, coreLicense, thirdPartyLicenses, loading, error, load}
}
