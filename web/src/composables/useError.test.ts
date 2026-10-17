import {useError} from './useError'

describe('useError', () => {
    it('shows a failure\'s message and resolves to nothing', async () => {
        const {error, loading, withErrorHandling} = useError()

        const result = await withErrorHandling(async () => {
            throw new Error('Cooler busy')
        })

        expect(result).toBeUndefined()
        expect(error.value).toBe('Cooler busy')
        expect(loading.value).toBe(false)
    })

    // A rejection need not be an Error; reading `.message` off `null` threw out of the handler.
    it.each([null, 'offline', {message: ''}])('shows a generic message for a rejection of %j', async (reason) => {
        const {error, withErrorHandling} = useError()

        await expect(withErrorHandling(() => Promise.reject(reason))).resolves.toBeUndefined()

        expect(error.value).toBe('An error occurred')
    })

    it('resolves to the result and clears the last error', async () => {
        const {error, setError, withErrorHandling} = useError()
        setError('stale')

        expect(await withErrorHandling(async () => 42)).toBe(42)
        expect(error.value).toBeNull()
    })

    it('hands the error on when asked', async () => {
        const failure = new Error('refused')
        const onError = vi.fn()
        const {withErrorHandling} = useError()

        await expect(withErrorHandling(() => Promise.reject(failure), {onError, rethrow: true})).rejects.toBe(failure)
        expect(onError).toHaveBeenCalledWith(failure)
    })
})
