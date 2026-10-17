/** How the constants hold the wire's enums: as rows of a choice, or as a table by name. */

/** One choice of a select or button group; `pro` locks it without Pro. */
export interface Option<T> {
    value: T
    label: string
    pro?: boolean
}

/** A wire enum by name: `byName(['warming_up'])` is `{WARMING_UP: 'warming_up'}`. */
export type ByName<V extends string> = {readonly [K in V as Uppercase<K>]: K}

/** Every value of a generated `*_VALUES` list under its upper-cased name. */
export function byName<const V extends string>(values: readonly V[]): ByName<V> {
    return Object.fromEntries(values.map((value) => [value.toUpperCase(), value])) as ByName<V>
}
