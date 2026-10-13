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

        it('rejects a number with trailing text', () => {
            expect(parseCoordinate('180abc', 'ra')).toBeNull()
            expect(parseCoordinate('12.5.3', 'dec')).toBeNull()
        })
    })

    // `parseFloat` once ran first and read the leading number: `05:35:17` (M42) set RA 5°.
    describe('sexagesimal', () => {
        const hms = (h: number, m: number, s = 0) => (h + m / 60 + s / 3600) * 15
        const dms = (d: number, m: number, s = 0) => d + m / 60 + s / 3600

        it('reads RA as hours, colon- or space-separated', () => {
            expect(parseCoordinate('05:35:17', 'ra')).toBeCloseTo(hms(5, 35, 17), 9)
            expect(parseCoordinate('12 30 00', 'ra')).toBeCloseTo(187.5, 9)
            expect(parseCoordinate('12:30', 'ra')).toBeCloseTo(187.5, 9)
            expect(parseCoordinate('23:59:59.99', 'ra')).toBeCloseTo(hms(23, 59, 59.99), 9)
        })

        it('reads Dec as degrees, with the sign from the text', () => {
            expect(parseCoordinate('-05:23:28', 'dec')).toBeCloseTo(-dms(5, 23, 28), 9)
            expect(parseCoordinate('+05:23:28', 'dec')).toBeCloseTo(dms(5, 23, 28), 9)
            expect(parseCoordinate('45:30', 'dec')).toBeCloseTo(45.5, 9)
            expect(parseCoordinate('-00:30', 'dec')).toBeCloseTo(-0.5, 9)
            expect(parseCoordinate('-00 00 36', 'dec')).toBeCloseTo(-0.01, 9)
        })

        it('reads unit symbols, ASCII or Unicode, in any case', () => {
            expect(parseCoordinate('5h35m', 'ra')).toBeCloseTo(hms(5, 35), 9)
            expect(parseCoordinate('5H 35M 17S', 'ra')).toBeCloseTo(hms(5, 35, 17), 9)
            expect(parseCoordinate('5°23′', 'dec')).toBeCloseTo(dms(5, 23), 9)
            expect(parseCoordinate('−5° 23′ 28″', 'dec')).toBeCloseTo(-dms(5, 23, 28), 9)
            expect(parseCoordinate(`-5d23'28"`, 'dec')).toBeCloseTo(-dms(5, 23, 28), 9)
        })

        it('allows 24h and ±90°, as the decimal path allows 360 and ±90', () => {
            expect(parseCoordinate('24:00:00', 'ra')).toBe(360)
            expect(parseCoordinate('-90:00:00', 'dec')).toBe(-90)
            expect(parseCoordinate('24:00:01', 'ra')).toBeNull()
            expect(parseCoordinate('90:00:01', 'dec')).toBeNull()
            expect(parseCoordinate('-00:30', 'ra')).toBeNull()
        })

        it('rejects minutes or seconds of 60 or more', () => {
            expect(parseCoordinate('05:60', 'ra')).toBeNull()
            expect(parseCoordinate('05:35:60', 'ra')).toBeNull()
            expect(parseCoordinate('45:00:60.5', 'dec')).toBeNull()
        })

        it('rejects malformed input', () => {
            for (const text of ['12:ab', '1:2:3:4', '12:', ':30', '-', '+-5:00', '- 5:00', '5:-3', '12::30x']) {
                expect(parseCoordinate(text, 'ra'), text).toBeNull()
                expect(parseCoordinate(text, 'dec'), text).toBeNull()
            }
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

