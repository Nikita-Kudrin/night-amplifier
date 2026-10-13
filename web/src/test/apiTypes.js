import source from '../composables/api.types.js?raw'

/**
 * The `@property` names of each `@typedef {object}` in the generated `api.types.js`, so a
 * test can hold a hand-written shape to what the server actually sends.
 * @returns {Record<string, {all: string[], required: string[]}>}
 */
export function wireTypes() {
    const types = {}
    for (const [, block] of source.matchAll(/\/\*\*([\s\S]*?)\*\//g)) {
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
