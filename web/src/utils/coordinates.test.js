import {describe, it, expect} from 'vitest'
import {formatDec, formatRA, parseCoordinate} from './coordinates.js'

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

// Rounded before splitting: flooring first showed a value just under a minute as `60.0s`.
describe('carrying a rounded second', () => {
    it('carries into the next minute of RA', () => {
        expect(formatRA((5 + 34 / 60 + 59.96 / 3600) * 15)).toBe('5h 35m 0.0s')
    })

    it('carries through the hour, and wraps 24h to 0h', () => {
        expect(formatRA((5 + 59 / 60 + 59.97 / 3600) * 15)).toBe('6h 0m 0.0s')
        expect(formatRA(359.99999)).toBe('0h 0m 0.0s')
    })

    it('carries into the next arcminute and degree of Dec, either sign', () => {
        expect(formatDec(41 + 15 / 60 + 59.96 / 3600)).toBe('+41° 16\' 0.0"')
        expect(formatDec(-(5 + 59 / 60 + 59.98 / 3600))).toBe('-6° 0\' 0.0"')
    })

    it('keeps a value that rounds down where it is', () => {
        expect(formatRA((5 + 34 / 60 + 59.94 / 3600) * 15)).toBe('5h 34m 59.9s')
    })
})

