// Type-level: `tsconfig.vitest.json` is part of `npm run typecheck`. The literal unions
// replaced the runtime prop validators, which only warned in a dev build.
import {describe, it, expect, expectTypeOf} from 'vitest'
import {mount} from '@vue/test-utils'
import type {ComponentMountingOptions} from '@vue/test-utils'
import {BINNING_OPTIONS} from '../../constants/capture'
import {WEIGHTING_PRESET_OPTIONS} from '../../constants/stacking'
import type {WeightingPreset} from '../../composables/api.types'
import {BaseSplitButton, ButtonGroup, SettingSelect} from './index'
import type {BaseAlert, BaseInfoIcon, BaseSpinner, BaseToggle, InstallStatusList} from './index'
import StatusIcon from './StatusIcon.vue'

type Props<C> = NonNullable<ComponentMountingOptions<C>['props']>

describe('ui prop types', () => {
    it('takes only the variants a component draws', () => {
        expectTypeOf<Props<typeof BaseAlert>['type']>().toEqualTypeOf<'error' | 'warning' | 'info' | 'success' | undefined>()
        expectTypeOf<Props<typeof BaseInfoIcon>['type']>().toEqualTypeOf<'info' | 'warning' | undefined>()
        expectTypeOf<Props<typeof BaseSpinner>['size']>().toEqualTypeOf<'xs' | 'sm' | 'md' | 'lg' | undefined>()
        expectTypeOf<Props<typeof BaseToggle>['size']>().toEqualTypeOf<'small' | 'normal' | undefined>()
        expectTypeOf<Props<typeof BaseSplitButton>['variant']>().toEqualTypeOf<'primary' | 'secondary' | 'danger' | undefined>()
        expectTypeOf<Props<typeof InstallStatusList>['size']>().toEqualTypeOf<'sm' | 'md' | undefined>()

        // @ts-expect-error -- no 'ghost' button
        expect(mount(BaseSplitButton, {props: {label: 'Connect', variant: 'ghost'}}).exists()).toBe(true)
    })

    it('names a status icon only by a shape it has', () => {
        expectTypeOf<'camera'>().toExtend<Props<typeof StatusIcon>['name']>()
        // @ts-expect-error -- no 'satellite' icon
        expect(mount(StatusIcon, {props: {name: 'satellite'}}).exists()).toBe(true)
    })

    // A template infers a generic component's `T` by calling it with the props; so do these,
    // which are never run.
    it('types a v-model over an option table as that table', () => {
        const templates = [
            () => ButtonGroup({
                options: BINNING_OPTIONS,
                modelValue: 2,
                'onUpdate:modelValue': (value) => expectTypeOf(value).toEqualTypeOf<number>(),
            }),
            () => SettingSelect({
                label: 'Weighting',
                options: WEIGHTING_PRESET_OPTIONS,
                modelValue: 'balanced',
                'onUpdate:modelValue': (value) => expectTypeOf(value).toEqualTypeOf<WeightingPreset>(),
                onChange: (value) => expectTypeOf(value).toEqualTypeOf<WeightingPreset>(),
            }),
            // @ts-expect-error -- a binning is a number
            () => ButtonGroup({options: BINNING_OPTIONS, modelValue: '2x2'}),
            // @ts-expect-error -- 'sharp' is no weighting preset
            () => SettingSelect({label: 'Weighting', options: WEIGHTING_PRESET_OPTIONS, modelValue: 'sharp'}),
        ]
        expect(templates).toHaveLength(4)
    })

    // A `<select>` reads every value back as a string; `change` hands over the option's own.
    it('hands a select\'s change over as the option\'s value, a number as a number', async () => {
        const wrapper = mount(SettingSelect<number>, {
            props: {label: 'Binning', options: BINNING_OPTIONS, modelValue: 1},
        })

        await wrapper.find('select').setValue('2')

        expect(wrapper.emitted('change')).toEqual([[2]])
    })
})
