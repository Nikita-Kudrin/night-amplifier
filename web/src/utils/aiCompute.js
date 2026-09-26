/**
 * What the "AI compute" selector shows, from `GET /api/ai-compute`: Auto with its pick,
 * then one option per rung, greyed out with the reason when this computer cannot use it.
 */

export const AI_COMPUTE_RUNGS = [
    {value: 'npu', label: 'NPU'},
    {value: 'discrete_gpu', label: 'Dedicated GPU'},
    {value: 'integrated_gpu', label: 'Integrated GPU'},
    {value: 'cpu', label: 'CPU'},
]

/** Where the manual lists what each rung needs installed. */
export const SYSTEM_DEPENDENCIES_URL = '/night-amplifier/system-dependencies'

export function rungLabel(value) {
    return AI_COMPUTE_RUNGS.find((r) => r.value === value)?.label ?? value
}

export function formatMs(ms) {
    if (typeof ms !== 'number' || !Number.isFinite(ms)) return ''
    return ms < 10 ? `${ms.toFixed(1)} ms` : `${Math.round(ms)} ms`
}

function rungReport(report, value) {
    return report?.rungs?.find((r) => r.rung === value) ?? null
}

export function isReady(report) {
    return report?.state === 'ready'
}

export function aiComputeOptions(report) {
    const ready = isReady(report)
    const auto = ready && report.auto
        ? `Auto — ${rungLabel(report.auto)}${timing(rungReport(report, report.auto))}`
        : ready ? 'Auto' : 'Auto (checking hardware…)'
    const rungs = AI_COMPUTE_RUNGS.map(({value, label}) => {
        if (!ready) return {value, label, disabled: false, reason: null}
        const rung = rungReport(report, value)
        if (rung?.usable) {
            return {value, label: `${label} — ${rung.device}${timing(rung)}`, disabled: false, reason: null}
        }
        return {value, label: `${label} — not available`, disabled: true, reason: rung?.reason ?? 'Not found'}
    })
    return [{value: 'auto', label: auto, disabled: false, reason: null}, ...rungs]
}

function timing(rung) {
    const ms = formatMs(rung?.ms_per_frame)
    return ms ? ` · ${ms}` : ''
}

/** One line on where the network runs now, or the report's notice when there is one. */
export function aiComputeSummary(report) {
    if (!isReady(report)) return ''
    if (report.notice) return report.notice
    const rung = rungReport(report, report.effective)
    if (!rung?.usable) return ''
    const details = [rung.api, rung.precision].filter(Boolean).join(', ')
    const ms = formatMs(rung.ms_per_frame)
    return `Runs on the ${rungLabel(rung.rung)}: ${rung.device}${details ? ` (${details})` : ''}${ms ? `, ${ms} per frame` : ''}.`
}

/** The rungs greyed out, with the device found (if any) and why. */
export function unusableRungs(report) {
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
