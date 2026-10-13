import source from '../composables/api.types.js?raw'
import eventSource from '../composables/api.events.js?raw'

/**
 * The `@property` names of each `@typedef {object}` in the generated `api.types.js`, so a
 * test can hold a hand-written shape to what the server actually sends.
 * @returns {Record<string, {all: string[], required: string[]}>}
 */
export function wireTypes() {
    return typedefs(source)
}

/** As `wireTypes`, of the events in `api.events.js` (`StateChangedEvent`, …). */
export function wireEventTypes() {
    return typedefs(eventSource)
}

function typedefs(text) {
    const types = {}
    for (const [, block] of text.matchAll(/\/\*\*([\s\S]*?)\*\//g)) {
        const name = /@typedef \{object\} (\w+)/.exec(block)?.[1]
        if (!name) continue
        const properties = [...block.matchAll(/@property \{[^}]*\} (\[?)(\w+)\]?/g)]
        types[name] = {
            all: properties.map(([, , key]) => key),
            required: properties.filter(([, optional]) => !optional).map(([, , key]) => key),
        }
    }
    return types
}

/** Every `@typedef` name in `api.types.js`, objects and unions alike. */
export function wireTypeNames() {
    return [...source.matchAll(/@typedef \{[^}]*\} (\w+)/g)].map(([, name]) => name)
}

/**
 * The members of a string-union `@typedef` in `api.types.js` (`CameraPhase`,
 * `StackingType`, …), so a hand-written table of them can be held to the server's enum.
 * @returns {string[]}
 */
export function wireEnum(name) {
    const match = new RegExp(`@typedef \\{([^}]*)\\} ${name}\\b`).exec(source)
    if (!match) throw new Error(`no @typedef ${name} in api.types.js`)
    return [...match[1].matchAll(/'([^']*)'/g)].map(([, member]) => member)
}
