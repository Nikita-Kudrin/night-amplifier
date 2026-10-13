import {splitExposure, toMicros} from './exposure.js'

describe('exposure', () => {
    it.each([
        [2000000, {value: 2, unit: 's'}],
        [1000000, {value: 1, unit: 's'}],
        [500000, {value: 500, unit: 'ms'}],
        [1000, {value: 1, unit: 'ms'}],
        [999, {value: 999, unit: 'us'}],
    ])('shows %i µs as %o', (us, shown) => {
        expect(splitExposure(us)).toEqual(shown)
    })

    it('reads a typed value in its unit', () => {
        expect(toMicros(2, 's')).toBe(2000000)
        expect(toMicros(40, 'ms')).toBe(40000)
        expect(toMicros(100, 'us')).toBe(100)
    })

    it('sends whole microseconds, which the server field requires', () => {
        expect(toMicros(4.1, 's')).toBe(4100000)
        expect(Number.isInteger(toMicros(0.3333, 'ms'))).toBe(true)
    })

    it('reads back what it split', () => {
        for (const us of [100, 5000, 40000, 300000, 2500000, 30000000]) {
            const {value, unit} = splitExposure(us)
            expect(toMicros(value, unit)).toBe(us)
        }
    })
})
