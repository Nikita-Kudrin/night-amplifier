import {ref} from 'vue'
import {parseCoordinate} from '../utils/coordinates'

/** Coordinates in degrees, as typed into the RA and Dec fields. */
export interface TypedCoordinates {
    ra: number
    dec: number
}

/** Composable for coordinate input with validation */
export function useCoordinateInput() {
    const raInput = ref('')
    const decInput = ref('')
    const coordError = ref('')

    function validateCoordinates(): TypedCoordinates | null {
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
