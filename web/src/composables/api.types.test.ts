import type {Settings, StartCaptureRequest, UpdateSettingsRequest} from './api.types.js'
import {SERVER_DEFAULT_SETTINGS} from './api.settings.js'

// Type-level: `tsconfig.vitest.json` is part of `npm run typecheck`.
describe('api.types', () => {
    // Binning sent without `camera_role` once rebinned the imaging camera while the guide was selected.
    it('types a settings update apart from the settings answer', () => {
        expectTypeOf<UpdateSettingsRequest>().toHaveProperty('camera_role')
        expectTypeOf<Settings>().not.toHaveProperty('camera_role')
        expectTypeOf({bin: 2, camera_role: 'guide'} as const).toExtend<UpdateSettingsRequest>()
        // @ts-expect-error the server knows no `imaging` role
        expectTypeOf({bin: 2, camera_role: 'imaging'} as const).toExtend<UpdateSettingsRequest>()
    })

    // A panel posts back the groups it read (`eyepiece` whole), so an answer must be a valid body.
    it('takes back every setting it sends', () => {
        expectTypeOf<Settings>().toExtend<UpdateSettingsRequest>()
        expectTypeOf(SERVER_DEFAULT_SETTINGS).toExtend<UpdateSettingsRequest>()
    })

    it('leaves out what the server defaults', () => {
        expectTypeOf({}).toExtend<StartCaptureRequest>()
    })
})
