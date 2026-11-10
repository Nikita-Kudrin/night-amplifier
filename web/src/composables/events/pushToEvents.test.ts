import {describe, it, expect} from 'vitest'
import {solveProgress} from './pushToEvents'

describe('solveProgress', () => {
    it('has no steps for a single-strategy solve, or before the first progress', () => {
        expect(solveProgress(null)).toBeNull()
        expect(solveProgress({label: 'full sky (blind FOV)', attempt: 1, total: 1})).toBeNull()
    })

    it('gives the step of a multi-strategy solve', () => {
        expect(solveProgress({label: 'blind', attempt: 2, total: 3})).toEqual({attempt: 2, total: 3})
    })

    it('never puts the step outside the plan', () => {
        expect(solveProgress({label: 'blind', attempt: 5, total: 3})).toEqual({attempt: 3, total: 3})
        expect(solveProgress({label: 'blind', attempt: 0, total: 3})).toEqual({attempt: 1, total: 3})
    })
})
