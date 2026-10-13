/**
 * Every `var(--x)` names a property something defines. An undefined one fails silently:
 * `--danger` (never defined) left the not-installed icons with no background in nine
 * components, and its fallbacks were Tailwind colours no theme of this app ever had.
 */
import {readFileSync} from 'node:fs'

// Vitest hands a `.css` import back empty, so the global sheet is read off disk (from
// `web/`, where Vitest runs).
const sources = {
    ...import.meta.glob('./**/*.vue', {query: '?raw', import: 'default', eager: true}),
    './assets/main.css': readFileSync('src/assets/main.css', 'utf8'),
}

function defined() {
    const names = new Set()
    for (const text of Object.values(sources)) {
        for (const [, name] of text.matchAll(/(--[a-z0-9-]+)\s*:/g)) names.add(name)
        // Set from a template binding: `:style="{'--chip-fg': …}"`.
        for (const [, name] of text.matchAll(/['"](--[a-z0-9-]+)['"]/g)) names.add(name)
    }
    return names
}

describe('stylesheets', () => {
    const names = defined()

    it.each(Object.keys(sources))('%s uses only defined custom properties', (path) => {
        const used = [...sources[path].matchAll(/var\((--[a-z0-9-]+)/g)].map(([, name]) => name)
        expect(used.filter((name) => !names.has(name))).toEqual([])
    })

    // A property defined as itself is invalid, and every user of it silently renders nothing.
    it('defines no custom property in terms of itself', () => {
        const css = Object.values(sources).join('\n')
        const circular = [...css.matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]*);/g)]
            .filter(([, name, value]) => value.includes(`var(${name})`))
            .map(([, name]) => name)
        expect(circular).toEqual([])
    })

    it.each(Object.keys(sources))('%s gives no custom property a fallback', (path) => {
        // A fallback hides a missing token instead of failing here.
        expect(sources[path].match(/var\(--[a-z0-9-]+\s*,[^)]*\)/g)).toBeNull()
    })

    // The UI is red for dark adaptation; a hue belongs in `:root` with a name. Shadows and
    // scrims stay neutral black or white and may be written in place.
    it.each(Object.keys(sources))('%s takes every colour from the theme', (path) => {
        const css = path.endsWith('.css')
            ? sources[path].replace(/:root\s*\{[^}]*\}/, '')
            : [...sources[path].matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map(([, body]) => body).join('\n')
        const literals = css.match(/#[0-9a-fA-F]{3,8}\b|rgba?\([^)]*\)/g) ?? []
        const coloured = literals.filter((value) => !/^rgba\((0, 0, 0|255, 255, 255), [0-9.]+\)$/.test(value))
        expect(coloured).toEqual([])
    })
})
