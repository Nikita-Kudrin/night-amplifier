import {
    formatAngle,
    formatBytes,
    formatFov,
    formatMs,
    formatPercent,
    formatResolution,
    formatSigma,
    formatTemperature,
    MISSING,
} from './format.js'

describe('format', () => {
    it('formats a fraction as a percentage', () => {
        expect(formatPercent(0.42)).toBe('42%')
        expect(formatPercent(0.0415, 1)).toBe('4.2%')
        expect(formatPercent(1)).toBe('100%')
    })

    it('formats sigma and temperature', () => {
        expect(formatSigma(2.5)).toBe('2.5σ')
        expect(formatTemperature(-9.96)).toBe('-10.0°C')
    })

    it('formats angles in arcminutes below a degree', () => {
        expect(formatAngle(0.5)).toBe("30.0'")
        expect(formatAngle(0.1)).toBe("6.0'")
        expect(formatAngle(1.5)).toBe('1.5°')
        expect(formatAngle(5)).toBe('5.0°')
    })

    it('formats a FOV with two decimals of degree', () => {
        expect(formatFov(1.234)).toBe('1.23°')
        expect(formatFov(0.75)).toBe("45.0'")
    })

    it('picks a binary unit for sizes', () => {
        expect(formatBytes(512)).toBe('512 B')
        expect(formatBytes(512 * 1024)).toBe('512 KB')
        expect(formatBytes(52428800)).toBe('50.0 MB')
        expect(formatBytes(1261887242)).toBe('1.18 GB')
    })

    it('formats short durations', () => {
        expect(formatMs(4.26)).toBe('4.3 ms')
        expect(formatMs(219.3)).toBe('219 ms')
        expect(formatMs(null)).toBe('')
        expect(formatMs(Number.NaN)).toBe('')
    })

    it('formats an image size, and none for a camera that reports 0x0', () => {
        expect(formatResolution(3008, 3008)).toBe('3008x3008')
        expect(formatResolution(0, 0)).toBe(MISSING)
        expect(formatResolution(undefined, undefined)).toBe(MISSING)
    })

    it.each([formatPercent, formatSigma, formatTemperature, formatAngle, formatFov, formatBytes])(
        '%o answers a dash for a missing value',
        (format) => {
            for (const missing of [null, undefined, Number.NaN, Infinity]) expect(format(missing)).toBe(MISSING)
        }
    )
})
