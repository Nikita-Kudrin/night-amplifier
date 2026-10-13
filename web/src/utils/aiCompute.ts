/**
 * What the "AI compute" selector shows, from `GET /api/ai-compute`: Auto with its pick,
 * then one option per rung, greyed out with the reason when this computer cannot use it.
 */
import type {AiComputePreference, AiComputeReport, ComputeRung, RungReport} from '../composables/api.types.js'
import type {Option} from '../constants'
import {formatMs} from './format.js'

/** An AI compute choice as the selector shows it; a disabled one says why. */
export interface AiComputeOption {
    value: AiComputePreference
    label: string
    disabled: boolean
    reason: string | null
}

/** A rung this computer cannot use, as the panel lists it. */
export interface UnusableRung {
    rung: ComputeRung
    label: string
    device: string | null
    reason: string
    installable: boolean
}

type Report = AiComputeReport | null | undefined

export const AI_COMPUTE_RUNGS = [
    {value: 'npu', label: 'NPU'},
    {value: 'discrete_gpu', label: 'Dedicated GPU'},
    {value: 'integrated_gpu', label: 'Integrated GPU'},
    {value: 'cpu', label: 'CPU'},
] satisfies readonly Option<ComputeRung>[]

/** Where the manual lists what each rung needs installed. */
export const SYSTEM_DEPENDENCIES_URL = '/night-amplifier/system-dependencies'

export function rungLabel(value: ComputeRung): string {
    return AI_COMPUTE_RUNGS.find((r) => r.value === value)?.label ?? value
}

function rungReport(report: Report, value: ComputeRung | null): RungReport | null {
    return report?.rungs?.find((r) => r.rung === value) ?? null
}

export function isReady(report: Report): report is AiComputeReport {
    return report?.state === 'ready'
}

/** Still looking: not an answer yet, unlike `unavailable` (no Pro, no licence), which never becomes one. */
function checking(report: Report): boolean {
    return report?.state === 'checking' || report?.state === 'benchmarking'
}

export function aiComputeOptions(report: Report): AiComputeOption[] {
    const ready = isReady(report)
    const auto = ready && report.auto
        ? `Auto — ${rungLabel(report.auto)}${timing(rungReport(report, report.auto))}`
        : checking(report) ? 'Auto (checking hardware…)' : 'Auto'
    const rungs = AI_COMPUTE_RUNGS.map(({value, label}): AiComputeOption => {
        if (!ready) return {value, label, disabled: false, reason: null}
        const rung = rungReport(report, value)
        if (rung?.usable) {
            return {value, label: `${label} — ${rung.device}${timing(rung)}`, disabled: false, reason: null}
        }
        return {value, label: `${label} — not available`, disabled: true, reason: rung?.reason ?? 'Not found'}
    })
    return [{value: 'auto', label: auto, disabled: false, reason: null}, ...rungs]
}

function timing(rung: RungReport | null): string {
    const ms = formatMs(rung?.ms_per_frame)
    return ms ? ` · ${ms}` : ''
}

/** One line on where the network runs now, or the report's notice when there is one. */
export function aiComputeSummary(report: Report): string {
    if (!isReady(report)) return ''
    if (report.notice) return report.notice
    const rung = rungReport(report, report.effective)
    if (!rung?.usable) return ''
    const details = [rung.api, rung.precision].filter(Boolean).join(', ')
    const ms = formatMs(rung.ms_per_frame)
    return `Runs on the ${rungLabel(rung.rung)}: ${rung.device}${details ? ` (${details})` : ''}${ms ? `, ${ms} per frame` : ''}.`
}

/** The rungs greyed out, with the device found (if any) and why. */
export function unusableRungs(report: Report): UnusableRung[] {
    if (!isReady(report)) return []
    return (report.rungs ?? [])
        .filter((r) => !r.usable)
        .map((r) => ({
            rung: r.rung,
            label: rungLabel(r.rung),
            device: r.device ?? null,
            reason: r.reason ?? 'Not available',
            installable: /install|driver|runtime|System dependencies/i.test(r.reason ?? ''),
        }))
}
