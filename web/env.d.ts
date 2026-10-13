/// <reference types="vite/client" />

import type {AriaAttributes} from 'vue'

declare module 'vue' {
    // These fall through to a component's root, as `class` does: `App.vue` makes a whole view
    // inert while the hardware benchmark runs, whichever view it is, and a decorative
    // spinner is `aria-hidden`.
    interface AllowedComponentProps extends AriaAttributes {
        inert?: boolean
    }
}
