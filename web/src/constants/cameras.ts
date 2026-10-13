/**
 * Database of popular astronomy cameras for FOV calculation, kept as data in
 * `cameras.json`: brand, model, sensor chip, pixel size (µm) and native resolution
 * (unbinned). Users search by brand/model/sensor, or fall back to manual pixel-size entry
 * / auto-fill from the connected camera.
 */
import cameras from './cameras.json'

export const CAMERA_DATABASE = cameras
