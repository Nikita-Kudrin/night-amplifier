import {effectScope, ref} from 'vue'
import {useClickOutside} from './useClickOutside.js'

describe('useClickOutside', () => {
    let root
    let menu
    let elsewhere

    beforeEach(() => {
        root = document.createElement('div')
        root.appendChild(document.createElement('button'))
        menu = document.createElement('ul')
        elsewhere = document.createElement('p')
        document.body.append(root, menu, elsewhere)
    })

    afterEach(() => document.body.replaceChildren())

    it('fires for a click outside every target, not inside any', () => {
        const onOutside = vi.fn()
        const scope = effectScope()
        scope.run(() => useClickOutside([ref(root), menu], onOutside))

        root.firstChild.click()
        menu.click()
        expect(onOutside).not.toHaveBeenCalled()

        elsewhere.click()
        expect(onOutside).toHaveBeenCalledTimes(1)
        scope.stop()
    })

    it('stops listening with its scope, and skips a target not mounted yet', () => {
        const onOutside = vi.fn()
        const scope = effectScope()
        scope.run(() => useClickOutside([ref(null)], onOutside))

        elsewhere.click()
        scope.stop()
        elsewhere.click()

        expect(onOutside).toHaveBeenCalledTimes(1)
    })
})
