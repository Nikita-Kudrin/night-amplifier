import {SERVER_DEFAULT_SETTINGS, SERVER_SETTING_RANGES} from '../composables/api.settings.js'
import type {ClampedPath} from '../composables/api.settings.js'
import type {Settings} from '../composables/api.types.js'
import type {PathValue, SettingPath} from '../utils/settings.js'

/** The server's default for a setting, by its dotted path in `Settings`. */
export function serverDefault<P extends SettingPath>(path: P): PathValue<Settings, P> {
    const value = path
        .split('.')
        .reduce<unknown>((group, key) => (group as Record<string, unknown> | undefined)?.[key], SERVER_DEFAULT_SETTINGS)
    return value as PathValue<Settings, P>
}

/** What a control may set on top of the server's range: its step, and a narrower range. */
interface ControlUi {
    min?: number
    max?: number
    step?: number
}

type Range<P extends ClampedPath> = (typeof SERVER_SETTING_RANGES)[P]

/** A range end as a number, or `null` for one the server leaves open and the control too. */
type End<U, K extends 'min' | 'max', Server> = U extends Record<K, number> ? number : Server extends number ? number : null

export type Limits<P extends ClampedPath, U extends ControlUi> = Readonly<
    Omit<U, 'min' | 'max'> & {
        min: End<U, 'min', Range<P>['min']>
        max: End<U, 'max', Range<P>['max']>
        default: PathValue<Settings, P>
    }
>

/**
 * `{min, max, default}` of a setting the server clamps, read from the generated
 * `api.settings.ts`: the range it enforces and the default it starts from. `ui` adds the
 * presentation (`step`) and may narrow the range to what the control offers, never widen
 * it (`serverSettings.test.ts` checks).
 */
export function serverLimits<P extends ClampedPath, U extends ControlUi = Record<never, never>>(
    path: P,
    ui?: U
): Limits<P, U> {
    const range = SERVER_SETTING_RANGES[path]
    return Object.freeze({min: range.min, max: range.max, default: serverDefault(path), ...ui}) as Limits<P, U>
}
