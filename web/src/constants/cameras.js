/**
 * Database of popular astronomy cameras for FOV calculation.
 *
 * Each entry contains brand, model, sensor chip name, pixel size (um),
 * and native resolution (without binning).
 *
 * Users can search by brand, model, or sensor name, or fall back to
 * manual pixel-size entry / auto-fill from the connected camera.
 */
export const CAMERA_DATABASE = [
    // ── ZWO ──────────────────────────────────────────────────────────
    {
        brand: 'ZWO',
        model: 'ASI2600MC Pro',
        sensor: 'IMX571',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 6248,
        height: 4176
    },
    {
        brand: 'ZWO',
        model: 'ASI2600MM Pro',
        sensor: 'IMX571',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 6248,
        height: 4176
    },
    {
        brand: 'ZWO',
        model: 'ASI533MC Pro',
        sensor: 'IMX533',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 3008,
        height: 3008
    },
    {
        brand: 'ZWO',
        model: 'ASI533MM Pro',
        sensor: 'IMX533',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 3008,
        height: 3008
    },
    {
        brand: 'ZWO',
        model: 'ASI294MC Pro',
        sensor: 'IMX294',
        pixel_size_x: 4.63,
        pixel_size_y: 4.63,
        width: 4144,
        height: 2822
    },
    {
        brand: 'ZWO',
        model: 'ASI294MM Pro',
        sensor: 'IMX294',
        pixel_size_x: 4.63,
        pixel_size_y: 4.63,
        width: 4144,
        height: 2822
    },
    {
        brand: 'ZWO',
        model: 'ASI6200MC Pro',
        sensor: 'IMX455',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 9576,
        height: 6388
    },
    {
        brand: 'ZWO',
        model: 'ASI6200MM Pro',
        sensor: 'IMX455',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 9576,
        height: 6388
    },
    {
        brand: 'ZWO',
        model: 'ASI2400MC Pro',
        sensor: 'IMX410',
        pixel_size_x: 5.94,
        pixel_size_y: 5.94,
        width: 6072,
        height: 4044
    },
    {
        brand: 'ZWO',
        model: 'ASI183MC Pro',
        sensor: 'IMX183',
        pixel_size_x: 2.4,
        pixel_size_y: 2.4,
        width: 5496,
        height: 3672
    },
    {
        brand: 'ZWO',
        model: 'ASI183MM Pro',
        sensor: 'IMX183',
        pixel_size_x: 2.4,
        pixel_size_y: 2.4,
        width: 5496,
        height: 3672
    },
    {
        brand: 'ZWO',
        model: 'ASI071MC Pro',
        sensor: 'IMX071',
        pixel_size_x: 4.78,
        pixel_size_y: 4.78,
        width: 4944,
        height: 3284
    },
    {
        brand: 'ZWO',
        model: 'ASI1600MC Pro',
        sensor: 'Panasonic MN34230',
        pixel_size_x: 3.8,
        pixel_size_y: 3.8,
        width: 4656,
        height: 3520
    },
    {
        brand: 'ZWO',
        model: 'ASI1600MM Pro',
        sensor: 'Panasonic MN34230',
        pixel_size_x: 3.8,
        pixel_size_y: 3.8,
        width: 4656,
        height: 3520
    },
    {
        brand: 'ZWO',
        model: 'ASI585MC',
        sensor: 'IMX585',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 3840,
        height: 2160
    },
    {
        brand: 'ZWO',
        model: 'ASI678MC',
        sensor: 'IMX678',
        pixel_size_x: 2.0,
        pixel_size_y: 2.0,
        width: 3840,
        height: 2160
    },
    {
        brand: 'ZWO',
        model: 'ASI662MC',
        sensor: 'IMX662',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 1920,
        height: 1080
    },
    {
        brand: 'ZWO',
        model: 'ASI462MC',
        sensor: 'IMX462',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 1920,
        height: 1080
    },
    {
        brand: 'ZWO',
        model: 'ASI385MC',
        sensor: 'IMX385',
        pixel_size_x: 3.75,
        pixel_size_y: 3.75,
        width: 1936,
        height: 1096
    },
    {
        brand: 'ZWO',
        model: 'ASI224MC',
        sensor: 'IMX224',
        pixel_size_x: 3.75,
        pixel_size_y: 3.75,
        width: 1304,
        height: 976
    },
    {
        brand: 'ZWO',
        model: 'ASI120MC-S',
        sensor: 'AR0130CS',
        pixel_size_x: 3.75,
        pixel_size_y: 3.75,
        width: 1280,
        height: 960
    },
    {
        brand: 'ZWO',
        model: 'ASI120MM-S',
        sensor: 'AR0130CS',
        pixel_size_x: 3.75,
        pixel_size_y: 3.75,
        width: 1280,
        height: 960
    },
    {
        brand: 'ZWO',
        model: 'ASI174MC',
        sensor: 'IMX174',
        pixel_size_x: 5.86,
        pixel_size_y: 5.86,
        width: 1936,
        height: 1216
    },
    {
        brand: 'ZWO',
        model: 'ASI174MM',
        sensor: 'IMX174',
        pixel_size_x: 5.86,
        pixel_size_y: 5.86,
        width: 1936,
        height: 1216
    },
    {
        brand: 'ZWO',
        model: 'ASI290MC',
        sensor: 'IMX290',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 1936,
        height: 1096
    },
    {
        brand: 'ZWO',
        model: 'ASI290MM',
        sensor: 'IMX290',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 1936,
        height: 1096
    },
    {
        brand: 'ZWO',
        model: 'ASI482MC',
        sensor: 'IMX482',
        pixel_size_x: 5.8,
        pixel_size_y: 5.8,
        width: 1920,
        height: 1080
    },
    {
        brand: 'ZWO',
        model: 'ASI715MC',
        sensor: 'IMX715',
        pixel_size_x: 1.45,
        pixel_size_y: 1.45,
        width: 3840,
        height: 2160
    },

    // ── Player One ───────────────────────────────────────────────────
    {
        brand: 'Player One',
        model: 'Poseidon-C Pro',
        sensor: 'IMX571',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 6248,
        height: 4176
    },
    {
        brand: 'Player One',
        model: 'Poseidon-M Pro',
        sensor: 'IMX571',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 6248,
        height: 4176
    },
    {
        brand: 'Player One',
        model: 'Saturn-C',
        sensor: 'IMX533',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 3008,
        height: 3008
    },
    {
        brand: 'Player One',
        model: 'Saturn-M',
        sensor: 'IMX533',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 3008,
        height: 3008
    },
    {
        brand: 'Player One',
        model: 'Neptune-C II',
        sensor: 'IMX464',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 2712,
        height: 1538
    },
    {
        brand: 'Player One',
        model: 'Mars-C II',
        sensor: 'IMX662',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 1920,
        height: 1080
    },
    {
        brand: 'Player One',
        model: 'Mars-M II',
        sensor: 'IMX662',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 1920,
        height: 1080
    },
    {
        brand: 'Player One',
        model: 'Uranus-C Pro',
        sensor: 'IMX585',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 3840,
        height: 2160
    },
    {
        brand: 'Player One',
        model: 'Uranus-M Pro',
        sensor: 'IMX585',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 3840,
        height: 2160
    },
    {
        brand: 'Player One',
        model: 'Ares-C Pro',
        sensor: 'IMX678',
        pixel_size_x: 2.0,
        pixel_size_y: 2.0,
        width: 3840,
        height: 2160
    },
    {
        brand: 'Player One',
        model: 'Ares-M Pro',
        sensor: 'IMX678',
        pixel_size_x: 2.0,
        pixel_size_y: 2.0,
        width: 3840,
        height: 2160
    },
    {
        brand: 'Player One',
        model: 'Apollo-C',
        sensor: 'IMX294',
        pixel_size_x: 4.63,
        pixel_size_y: 4.63,
        width: 4144,
        height: 2822
    },
    {
        brand: 'Player One',
        model: 'Apollo-M Max',
        sensor: 'IMX294',
        pixel_size_x: 4.63,
        pixel_size_y: 4.63,
        width: 4144,
        height: 2822
    },
    {
        brand: 'Player One',
        model: 'Artemis-C Pro',
        sensor: 'IMX294',
        pixel_size_x: 4.63,
        pixel_size_y: 4.63,
        width: 4144,
        height: 2822
    },

    // ── QHY ──────────────────────────────────────────────────────────
    {
        brand: 'QHY',
        model: 'QHY268C',
        sensor: 'IMX571',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 6252,
        height: 4176
    },
    {
        brand: 'QHY',
        model: 'QHY268M',
        sensor: 'IMX571',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 6252,
        height: 4176
    },
    {
        brand: 'QHY',
        model: 'QHY600C',
        sensor: 'IMX455',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 9576,
        height: 6388
    },
    {
        brand: 'QHY',
        model: 'QHY600M',
        sensor: 'IMX455',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 9576,
        height: 6388
    },
    {
        brand: 'QHY',
        model: 'QHY533C',
        sensor: 'IMX533',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 3008,
        height: 3008
    },
    {
        brand: 'QHY',
        model: 'QHY533M',
        sensor: 'IMX533',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 3008,
        height: 3008
    },
    {
        brand: 'QHY',
        model: 'QHY294C Pro',
        sensor: 'IMX294',
        pixel_size_x: 4.63,
        pixel_size_y: 4.63,
        width: 4144,
        height: 2822
    },
    {
        brand: 'QHY',
        model: 'QHY294M Pro',
        sensor: 'IMX294',
        pixel_size_x: 4.63,
        pixel_size_y: 4.63,
        width: 4144,
        height: 2822
    },
    {brand: 'QHY', model: 'QHY183C', sensor: 'IMX183', pixel_size_x: 2.4, pixel_size_y: 2.4, width: 5496, height: 3672},
    {
        brand: 'QHY',
        model: 'QHY5III585C',
        sensor: 'IMX585',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 3840,
        height: 2160
    },
    {
        brand: 'QHY',
        model: 'QHY5III462C',
        sensor: 'IMX462',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 1920,
        height: 1080
    },
    {
        brand: 'QHY',
        model: 'QHY5III678C',
        sensor: 'IMX678',
        pixel_size_x: 2.0,
        pixel_size_y: 2.0,
        width: 3840,
        height: 2160
    },
    {
        brand: 'QHY',
        model: 'QHY5III715C',
        sensor: 'IMX715',
        pixel_size_x: 1.45,
        pixel_size_y: 1.45,
        width: 3840,
        height: 2160
    },

    // ── Touptek / RISING CAM ─────────────────────────────────────────
    {
        brand: 'Touptek',
        model: 'ATR585C',
        sensor: 'IMX585',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 3840,
        height: 2160
    },
    {
        brand: 'Touptek',
        model: 'ATR571C',
        sensor: 'IMX571',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 6248,
        height: 4176
    },
    {
        brand: 'Touptek',
        model: 'ATR533C',
        sensor: 'IMX533',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 3008,
        height: 3008
    },
    {
        brand: 'Touptek',
        model: 'ATR294C',
        sensor: 'IMX294',
        pixel_size_x: 4.63,
        pixel_size_y: 4.63,
        width: 4144,
        height: 2822
    },
    {
        brand: 'Touptek',
        model: 'ATR678C',
        sensor: 'IMX678',
        pixel_size_x: 2.0,
        pixel_size_y: 2.0,
        width: 3840,
        height: 2160
    },
    {
        brand: 'Touptek',
        model: 'ATR462C',
        sensor: 'IMX462',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 1920,
        height: 1080
    },

    // ── SVBONY ───────────────────────────────────────────────────────
    {
        brand: 'SVBONY',
        model: 'SV705C',
        sensor: 'IMX585',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 3840,
        height: 2160
    },
    {
        brand: 'SVBONY',
        model: 'SV505C',
        sensor: 'IMX464',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 2712,
        height: 1538
    },
    {
        brand: 'SVBONY',
        model: 'SV405CC',
        sensor: 'IMX294',
        pixel_size_x: 4.63,
        pixel_size_y: 4.63,
        width: 4144,
        height: 2822
    },
    {
        brand: 'SVBONY',
        model: 'SV305 Pro',
        sensor: 'IMX290',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 1936,
        height: 1096
    },
    {
        brand: 'SVBONY',
        model: 'SV905C',
        sensor: 'IMX678',
        pixel_size_x: 2.0,
        pixel_size_y: 2.0,
        width: 3840,
        height: 2160
    },

    // ── Altair Astro ─────────────────────────────────────────────────
    {
        brand: 'Altair',
        model: 'Hypercam 585C',
        sensor: 'IMX585',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 3840,
        height: 2160
    },
    {
        brand: 'Altair',
        model: 'Hypercam 294C Pro',
        sensor: 'IMX294',
        pixel_size_x: 4.63,
        pixel_size_y: 4.63,
        width: 4144,
        height: 2822
    },
    {
        brand: 'Altair',
        model: 'Hypercam 533C',
        sensor: 'IMX533',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 3008,
        height: 3008
    },
    {
        brand: 'Altair',
        model: 'Hypercam 571C',
        sensor: 'IMX571',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 6248,
        height: 4176
    },
    {
        brand: 'Altair',
        model: 'Hypercam 678C',
        sensor: 'IMX678',
        pixel_size_x: 2.0,
        pixel_size_y: 2.0,
        width: 3840,
        height: 2160
    },

    // ── Mallincam ────────────────────────────────────────────────────
    {
        brand: 'Mallincam',
        model: 'DS26C',
        sensor: 'IMX571',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 6248,
        height: 4176
    },
    {
        brand: 'Mallincam',
        model: 'SkyRaider DS16C',
        sensor: 'IMX178',
        pixel_size_x: 2.4,
        pixel_size_y: 2.4,
        width: 3096,
        height: 2080
    },

    // ── Vaonis ───────────────────────────────────────────────────────
    {
        brand: 'Vaonis',
        model: 'Stellina',
        sensor: 'IMX178',
        pixel_size_x: 2.4,
        pixel_size_y: 2.4,
        width: 3096,
        height: 2080
    },

    // ── CMOS generic (by sensor) ─────────────────────────────────────
    {
        brand: 'Generic',
        model: 'IMX585 Camera',
        sensor: 'IMX585',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 3840,
        height: 2160
    },
    {
        brand: 'Generic',
        model: 'IMX662 Camera',
        sensor: 'IMX662',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 1920,
        height: 1080
    },
    {
        brand: 'Generic',
        model: 'IMX678 Camera',
        sensor: 'IMX678',
        pixel_size_x: 2.0,
        pixel_size_y: 2.0,
        width: 3840,
        height: 2160
    },
    {
        brand: 'Generic',
        model: 'IMX715 Camera',
        sensor: 'IMX715',
        pixel_size_x: 1.45,
        pixel_size_y: 1.45,
        width: 3840,
        height: 2160
    },
    {
        brand: 'Generic',
        model: 'IMX533 Camera',
        sensor: 'IMX533',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 3008,
        height: 3008
    },
    {
        brand: 'Generic',
        model: 'IMX571 Camera',
        sensor: 'IMX571',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 6248,
        height: 4176
    },
    {
        brand: 'Generic',
        model: 'IMX294 Camera',
        sensor: 'IMX294',
        pixel_size_x: 4.63,
        pixel_size_y: 4.63,
        width: 4144,
        height: 2822
    },
    {
        brand: 'Generic',
        model: 'IMX455 Camera',
        sensor: 'IMX455',
        pixel_size_x: 3.76,
        pixel_size_y: 3.76,
        width: 9576,
        height: 6388
    },
    {
        brand: 'Generic',
        model: 'IMX462 Camera',
        sensor: 'IMX462',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 1920,
        height: 1080
    },
    {
        brand: 'Generic',
        model: 'IMX290 Camera',
        sensor: 'IMX290',
        pixel_size_x: 2.9,
        pixel_size_y: 2.9,
        width: 1936,
        height: 1096
    },
    {
        brand: 'Generic',
        model: 'IMX183 Camera',
        sensor: 'IMX183',
        pixel_size_x: 2.4,
        pixel_size_y: 2.4,
        width: 5496,
        height: 3672
    },
    {
        brand: 'Generic',
        model: 'IMX224 Camera',
        sensor: 'IMX224',
        pixel_size_x: 3.75,
        pixel_size_y: 3.75,
        width: 1304,
        height: 976
    },
    {
        brand: 'Generic',
        model: 'IMX174 Camera',
        sensor: 'IMX174',
        pixel_size_x: 5.86,
        pixel_size_y: 5.86,
        width: 1936,
        height: 1216
    },
]
