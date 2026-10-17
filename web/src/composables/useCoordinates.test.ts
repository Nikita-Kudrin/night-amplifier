import {describe, it, expect} from 'vitest'
import {useCoordinateInput} from './useCoordinates'

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

    it('reads the sexagesimal form its placeholder offers', () => {
        const {raInput, decInput, validateCoordinates} = useCoordinateInput()
        raInput.value = '05:35:17'
        decInput.value = '-05:23:28'

        const result = validateCoordinates()
        expect(result?.ra).toBeCloseTo((5 + 35 / 60 + 17 / 3600) * 15, 9)
        expect(result?.dec).toBeCloseTo(-(5 + 23 / 60 + 28 / 3600), 9)
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
