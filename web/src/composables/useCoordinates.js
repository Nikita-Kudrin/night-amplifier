import {ref} from 'vue'

/**
 * Parse coordinate string to degrees
 * @param {string} value - Coordinate string (decimal degrees or HMS/DMS format)
 * @param {'ra' | 'dec'} type - Coordinate type
 * @returns {number | null} Degrees or null if invalid
 */
export function parseCoordinate(value, type) {
    if (!value || !value.trim()) return null

    const trimmed = value.trim()

    // Try decimal degrees first
    const decimal = parseFloat(trimmed)
    if (!isNaN(decimal)) {
        if (type === 'ra' && (decimal < 0 || decimal > 360)) return null
        if (type === 'dec' && (decimal < -90 || decimal > 90)) return null
        return decimal
    }

    // Try HMS/DMS format (e.g., "12:30:45" or "12 30 45")
    const parts = trimmed.split(/[:\s]+/).map((p) => parseFloat(p))
    if (parts.length >= 2 && parts.length <= 3 && parts.every((p) => !isNaN(p))) {
        const [d, m, s = 0] = parts
        const sign = d < 0 ? -1 : 1
        let degrees = Math.abs(d) + m / 60 + s / 3600

        if (type === 'ra') {
            // Convert hours to degrees for RA
            degrees = degrees * 15
            if (degrees > 360) return null
        } else {
            degrees = sign * degrees
            if (degrees < -90 || degrees > 90) return null
        }

        return degrees
    }

    return null
}

/**
 * Format Right Ascension from degrees to HMS string
 * @param {number | null | undefined} degrees - RA in degrees
 * @returns {string} Formatted string
 */
export function formatRA(degrees) {
    if (degrees === undefined || degrees === null) return '--'
    const hours = degrees / 15
    const h = Math.floor(hours)
    const m = Math.floor((hours - h) * 60)
    const s = ((hours - h) * 60 - m) * 60
    return `${h}h ${m}m ${s.toFixed(1)}s`
}

/**
 * Format Declination from degrees to DMS string
 * @param {number | null | undefined} degrees - Dec in degrees
 * @returns {string} Formatted string
 */
export function formatDec(degrees) {
    if (degrees === undefined || degrees === null) return '--'
    const sign = degrees >= 0 ? '+' : '-'
    const abs = Math.abs(degrees)
    const d = Math.floor(abs)
    const m = Math.floor((abs - d) * 60)
    const s = ((abs - d) * 60 - m) * 60
    return `${sign}${d}° ${m}' ${s.toFixed(1)}"`
}

/**
 * Format angular distance
 * @param {number | null | undefined} degrees - Distance in degrees
 * @returns {string} Formatted string (arcmin for <1°, degrees otherwise)
 */
export function formatDistance(degrees) {
    if (degrees === undefined || degrees === null) return '--'
    if (degrees < 1) {
        return `${(degrees * 60).toFixed(1)}'`
    }
    return `${degrees.toFixed(1)}°`
}

/**
 * Composable for coordinate input with validation
 * @returns Reactive coordinate input state and methods
 */
export function useCoordinateInput() {
    const raInput = ref('')
    const decInput = ref('')
    const coordError = ref('')

    function validateCoordinates() {
        coordError.value = ''

        const ra = parseCoordinate(raInput.value, 'ra')
        const dec = parseCoordinate(decInput.value, 'dec')

        if (ra === null) {
            coordError.value = 'Invalid RA format. Use decimal degrees or HH:MM:SS'
            return null
        }
        if (dec === null) {
            coordError.value = 'Invalid Dec format. Use decimal degrees or DD:MM:SS'
            return null
        }

        return {ra, dec}
    }

    function clearInputs() {
        raInput.value = ''
        decInput.value = ''
        coordError.value = ''
    }

    return {
        raInput,
        decInput,
        coordError,
        validateCoordinates,
        clearInputs,
    }
}
