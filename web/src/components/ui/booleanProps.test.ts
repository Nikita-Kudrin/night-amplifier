import {describe, it, expect} from 'vitest'
import {mount} from '@vue/test-utils'
import type {Component} from 'vue'
import {
    BaseAlert,
    BaseModal,
    BasePanel,
    BaseSlider,
    BaseSpinner,
    BaseSplitButton,
    BaseToggle,
    ButtonGroup,
    CollapseToggle,
    InstallProgressBar,
    SettingSelect,
    SetupSection,
} from './index'

/**
 * Vue casts a prop to a boolean only when the runtime type it derives from the declared
 * type says `Boolean`. A prop typed with something the SFC compiler cannot resolve gets
 * no runtime type, and `<BaseToggle disabled>` then arrives as `''`, which is what a
 * template hands a bare attribute. So every boolean prop, given `''`, arrives as `true`,
 * and left out, as its default.
 */
const CASES: [name: string, component: Component, required: Record<string, unknown>, defaults: Record<string, boolean>][] = [
    ['BaseAlert', BaseAlert, {}, {dismissible: true, bordered: false}],
    ['BaseModal', BaseModal, {}, {noPadding: false, persistent: false, showCloseButton: true}],
    ['BasePanel', BasePanel, {}, {bordered: true}],
    ['BaseSlider', BaseSlider, {modelValue: 46}, {showInput: false, disabled: false, largeGap: false}],
    ['BaseSpinner', BaseSpinner, {}, {light: false}],
    ['BaseSplitButton', BaseSplitButton, {label: 'Connect'}, {disabled: false}],
    ['BaseToggle', BaseToggle, {}, {modelValue: false, disabled: false}],
    ['ButtonGroup', ButtonGroup, {options: []}, {disabled: false}],
    ['CollapseToggle', CollapseToggle, {}, {modelValue: false}],
    ['InstallProgressBar', InstallProgressBar, {}, {compact: false, spinner: true}],
    ['SettingSelect', SettingSelect, {label: 'Mode', modelValue: 'a', options: []}, {locked: false, disabled: false}],
    ['SetupSection', SetupSection, {number: 1, title: 'Install'}, {done: false}],
]

function propsOf(component: Component, props: Record<string, unknown>): Record<string, unknown> {
    return mount(component, {props}).props()
}

describe.each(CASES)('%s boolean props', (_name, component, required, defaults) => {
    it.each(Object.keys(defaults))('takes a bare `%s` as true', (prop) => {
        expect(propsOf(component, {...required, [prop]: ''})[prop]).toBe(true)
    })

    it.each(Object.entries(defaults))('defaults `%s` to %s', (prop, value) => {
        expect(propsOf(component, required)[prop]).toBe(value)
    })
})
