import type {
    ChartSourceApi,
    ProposedMetricApi,
    SurfaceSourceApi,
    AgentSourceApi,
    UsageSnapshotPayloadApi,
} from './generated/api.schemas'

export interface ProposedRow {
    key: string
    /** The drafted name, or null when the measure has no draft yet. */
    name: string | null
    measure: string | null
    description: string
    sql: string
    measures: number
    outcome: string
    people: number
    days: number
    runs: number
    charts: ChartSourceApi[]
    other_charts: number
    surfaces: SurfaceSourceApi[]
    agents: AgentSourceApi | null
}

export interface NewMetricPrefill {
    display_name: string
    name: string
    description: string
    sql: string
}

function proposalRow(proposal: ProposedMetricApi, runs: number): ProposedRow {
    return {
        key: `proposal-${proposal.name}`,
        name: proposal.name,
        measure: null,
        description: proposal.description,
        sql: proposal.definition,
        measures: proposal.measures,
        outcome: 'uncovered',
        people: proposal.people,
        days: proposal.active_days,
        runs,
        charts: proposal.charts,
        other_charts: proposal.other_charts,
        surfaces: proposal.surfaces,
        agents: proposal.agents,
    }
}

/** One row per proposal, then one per remaining measure the catalog does not cover, ranked as the scan ranks them: people, days, runs. */
export function buildProposedRows(payload: UsageSnapshotPayloadApi, includeScouts: boolean): ProposedRow[] {
    const proposals = payload.proposed_metrics ?? []
    const proposalNames = new Set(proposals.map((proposal) => proposal.name))
    const groups = payload.uncovered ?? []

    // A proposal merges measures that one query can compute, so its scout runs are the largest measure's too.
    const scoutRuns = new Map<string, number>()
    for (const group of groups) {
        if (group.draft_name) {
            scoutRuns.set(group.draft_name, Math.max(scoutRuns.get(group.draft_name) ?? 0, group.scout_runs))
        }
    }

    const rows = proposals.map((proposal) =>
        proposalRow(proposal, proposal.runs + (includeScouts ? (scoutRuns.get(proposal.name) ?? 0) : 0))
    )
    for (const group of groups) {
        if (group.draft_name && proposalNames.has(group.draft_name)) {
            continue
        }
        // The catalog already has this measure, so there is nothing to propose.
        if (group.outcome === 'covered') {
            continue
        }
        // Only scouts run this measure, so with scouts hidden nothing in the row is left to show.
        if (!includeScouts && group.distinct_people === 0) {
            continue
        }
        rows.push({
            key: group.fingerprint_digest,
            name: group.draft_name,
            measure: group.measure,
            description: group.draft_description ?? '',
            sql: group.draft_definition || group.sample_sql,
            measures: 1,
            outcome: group.outcome,
            people: group.distinct_people,
            days: group.active_days,
            runs: group.runs + (includeScouts ? group.scout_runs : 0),
            charts: [],
            other_charts: 0,
            surfaces: [],
            agents: null,
        })
    }
    return rows.sort((a, b) => b.people - a.people || b.days - a.days || b.runs - a.runs || a.key.localeCompare(b.key))
}

export function proposedRowPrefill(row: ProposedRow): NewMetricPrefill {
    const words = (row.name ?? '').replace(/_+/g, ' ').trim()
    return {
        display_name: words ? words[0].toUpperCase() + words.slice(1) : '',
        name: row.name ?? '',
        description: row.description,
        sql: row.sql,
    }
}
