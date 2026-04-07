import {describe, it, expect} from 'vitest'
import {
    parseCoordinate,
    formatRA,
    formatDec,
    formatDistance,
    useCoordinateInput,
} from '../useCoordinates.js'

describe('parseCoordinate', () => {
    describe('RA parsing - decimal degrees', () => {
        it('parses decimal degrees', () => {
            expect(parseCoordinate('180', 'ra')).toBe(180)
            expect(parseCoordinate('0', 'ra')).toBe(0)
            expect(parseCoordinate('359.5', 'ra')).toBeCloseTo(359.5)
        })

        it('rejects out-of-range RA values', () => {
            expect(parseCoordinate('-1', 'ra')).toBeNull()
            expect(parseCoordinate('361', 'ra')).toBeNull()
        })
    })

    describe('Dec parsing - decimal degrees', () => {
        it('parses decimal degrees', () => {
            expect(parseCoordinate('45', 'dec')).toBe(45)
            expect(parseCoordinate('-45', 'dec')).toBe(-45)
            expect(parseCoordinate('0', 'dec')).toBe(0)
        })

        it('rejects out-of-range Dec values', () => {
            expect(parseCoordinate('91', 'dec')).toBeNull()
            expect(parseCoordinate('-91', 'dec')).toBeNull()
        })
    })

    describe('edge cases', () => {
        it('returns null for empty input', () => {
            expect(parseCoordinate('', 'ra')).toBeNull()
            expect(parseCoordinate('   ', 'ra')).toBeNull()
            expect(parseCoordinate(null, 'ra')).toBeNull()
            expect(parseCoordinate(undefined, 'ra')).toBeNull()
        })

        it('returns null for non-numeric input', () => {
            expect(parseCoordinate('abc', 'ra')).toBeNull()
        })

        it('trims whitespace', () => {
            expect(parseCoordinate('  180  ', 'ra')).toBe(180)
        })

        it('parses first numeric part of colon-separated input as decimal', () => {
            // Note: parseFloat('12:30') returns 12, so colon-separated inputs
            // are interpreted as decimal degrees (first number before colon)
            expect(parseCoordinate('12:30', 'ra')).toBe(12)
            expect(parseCoordinate('45:30', 'dec')).toBe(45)
        })
    })
})

describe('formatRA', () => {
    it('formats RA in HMS', () => {
        expect(formatRA(0)).toBe('0h 0m 0.0s')
        expect(formatRA(180)).toBe('12h 0m 0.0s')
        expect(formatRA(90)).toBe('6h 0m 0.0s')
    })

    it('handles null and undefined', () => {
        expect(formatRA(null)).toBe('--')
        expect(formatRA(undefined)).toBe('--')
    })
})

describe('formatDec', () => {
    it('formats positive Dec in DMS', () => {
        expect(formatDec(0)).toBe('+0° 0\' 0.0"')
        expect(formatDec(45.5)).toBe('+45° 30\' 0.0"')
    })

    it('formats negative Dec in DMS', () => {
        expect(formatDec(-30.25)).toBe('-30° 15\' 0.0"')
    })

    it('handles null and undefined', () => {
        expect(formatDec(null)).toBe('--')
        expect(formatDec(undefined)).toBe('--')
    })
})

describe('formatDistance', () => {
    it('formats distance in degrees', () => {
        expect(formatDistance(5)).toBe('5.0°')
        expect(formatDistance(1.5)).toBe('1.5°')
    })

    it('formats small distances in arcminutes', () => {
        expect(formatDistance(0.5)).toBe("30.0'")
        expect(formatDistance(0.1)).toBe("6.0'")
    })

    it('handles null and undefined', () => {
        expect(formatDistance(null)).toBe('--')
        expect(formatDistance(undefined)).toBe('--')
    })
})

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
