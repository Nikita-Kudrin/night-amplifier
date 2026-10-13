import {describe, it, expect} from 'vitest'
import {useCoordinateInput} from './useCoordinates.js'

describe('useCoordinateInput', () => {
    it('provides reactive input refs', () => {
        const {raInput, decInput, coordError} = useCoordinateInput()
        expect(raInput.value).toBe('')
        expect(decInput.value).toBe('')
        expect(coordError.value).toBe('')
    })

    it('validates coordinates successfully', () => {
        const {raInput, decInput, validateCoordinates} = useCoordinateInput()
        raInput.value = '180'
        decInput.value = '45'

        const result = validateCoordinates()
        expect(result).toEqual({ra: 180, dec: 45})
    })

    it('sets error for invalid RA', () => {
        const {raInput, decInput, coordError, validateCoordinates} = useCoordinateInput()
        raInput.value = 'invalid'
        decInput.value = '45'

        const result = validateCoordinates()
        expect(result).toBeNull()
        expect(coordError.value).toContain('RA')
    })

    it('sets error for invalid Dec', () => {
        const {raInput, decInput, coordError, validateCoordinates} = useCoordinateInput()
        raInput.value = '180'
        decInput.value = 'invalid'

        const result = validateCoordinates()
        expect(result).toBeNull()
        expect(coordError.value).toContain('Dec')
    })

    it('clears inputs', () => {
        const {raInput, decInput, coordError, clearInputs} = useCoordinateInput()
        raInput.value = '180'
        decInput.value = '45'
        coordError.value = 'some error'

        clearInputs()

        expect(raInput.value).toBe('')
        expect(decInput.value).toBe('')
        expect(coordError.value).toBe('')
    })
})
