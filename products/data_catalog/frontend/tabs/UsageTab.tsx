import { useActions, useValues } from 'kea'

import { IconInfo } from '@posthog/icons'
import { LemonBanner, LemonButton, LemonSwitch, LemonTag, LemonTagType, Spinner } from '@posthog/lemon-ui'

import { CodeSnippet, Language } from 'lib/components/CodeSnippet'
import { TZLabel } from 'lib/components/TZLabel'
import { LemonTable, LemonTableColumns } from 'lib/lemon-ui/LemonTable'
import { Link } from 'lib/lemon-ui/Link'
import { Tooltip } from 'lib/lemon-ui/Tooltip'

import { newMetricUrl } from '../common'
import type { TopicClusterApi } from '../generated/api.schemas'
import { ProposedRow, proposedRowPrefill } from '../proposedRows'
import { usageLogic } from '../usageLogic'
import { MetricUsageRow } from '../usageRows'
import { ProposedMetricSources } from './ProposedMetricSources'

// LemonTable keeps its page in the URL under its id, so tables on one page need their own ids to page apart.
const TOPICS_TABLE_ID = 'usage-topics'
const METRICS_TABLE_ID = 'usage-metrics'
const PROPOSED_TABLE_ID = 'usage-proposed'
const HALF_TABLE_PAGE_SIZE = 6
const PROPOSED_PAGE_SIZE = 10

// The judge step writes the adjusted runs, which the metric table and the topics both read.
const AGENT_COUNTS_MISSING = 'Agent query counts are missing, because the agent query check failed in the last scan.'

function SectionFailed({ reason }: { reason: string }): JSX.Element {
    return (
        <LemonBanner type="warning">
            <div>This section failed in the last scan. Run a new scan to try again.</div>
            <div className="text-xs">{reason}</div>
        </LemonBanner>
    )
}

function CountTags({ counts }: { counts: [string, number][] }): JSX.Element {
    return (
        <div className="flex flex-wrap items-center gap-1">
            {counts
                .filter(([, count]) => count > 0)
                .map(([label, count]) => (
                    <LemonTag key={label} size="small">
                        {label} {count}
                    </LemonTag>
                ))}
        </div>
    )
}

const OUTCOME_LABELS: Record<string, { label: string; type: LemonTagType }> = {
    slice: { label: 'Covered with a filter', type: 'highlight' },
    uncovered: { label: 'Not covered', type: 'warning' },
}

export function UsageTab(): JSX.Element {
    const {
        latest,
        latestLoadFailed,
        scanRunning,
        newestScan,
        includeScouts,
        metricRows,
        proposedRows,
        topics,
        topicSessionsSummary,
        failureReasons,
        recomputedLoading,
    } = useValues(usageLogic)
    const { setIncludeScouts, recompute } = useActions(usageLogic)

    const recomputeButton = (
        <LemonButton
            type="secondary"
            size="small"
            onClick={recompute}
            loading={recomputedLoading}
            disabledReason={scanRunning ? 'A scan is already running' : undefined}
            data-attr="data-catalog-usage-recompute"
        >
            Run a new scan
        </LemonButton>
    )

    if (latestLoadFailed) {
        return (
            <LemonBanner type="error">
                Couldn't load the usage scan. Refresh the page, and if it keeps happening contact support.
            </LemonBanner>
        )
    }
    if (latest === null) {
        return <Spinner />
    }
    if (latest === 'none') {
        return (
            <div className="flex flex-col items-start gap-2">
                <p className="m-0">
                    {scanRunning
                        ? 'A usage scan is running. Results appear here when it finishes.'
                        : 'No usage scan has finished yet. A scan runs every week, or you can start one now.'}
                </p>
                {recomputeButton}
            </div>
        )
    }

    const metricColumns: LemonTableColumns<MetricUsageRow> = [
        {
            title: 'Metric',
            key: 'name',
            render: (_, row) => <span className="inline-block min-w-32">{row.name}</span>,
        },
        {
            title: 'Direct runs',
            key: 'directRuns',
            tooltip: 'Runs of the metric by agents, split by agent. Runs from the web app and the API do not count.',
            render: (_, row) => (
                <div className="flex flex-wrap items-center gap-1">
                    <span className="font-semibold">{row.directRuns}</span>
                    {row.surfaces.map(([surface, runs]) => (
                        <LemonTag key={surface} size="small">
                            {surface} {runs}
                        </LemonTag>
                    ))}
                </div>
            ),
            sorter: (a, b) => a.directRuns - b.directRuns,
        },
        { title: 'People', dataIndex: 'people', key: 'people', sorter: (a, b) => a.people - b.people },
        {
            title: 'Agent SQL',
            key: 'agentSql',
            tooltip:
                'Agent queries that computed the metric in SQL. Adapted: read the metric first, then changed it. Missed: looked at the catalog, then computed it again. Silent: did not look at the catalog.',
            render: (_, row) => (
                <CountTags
                    counts={[
                        ['Adapted', row.adapted],
                        ['Missed', row.missed],
                        ['Silent', row.silent],
                    ]}
                />
            ),
            sorter: (a, b) => a.adapted + a.missed + a.silent - (b.adapted + b.missed + b.silent),
        },
    ]

    const topicColumns: LemonTableColumns<TopicClusterApi> = [
        {
            title: 'Topic',
            key: 'title',
            render: (_, topic) => (
                <div className="flex min-w-48 flex-col py-1">
                    <span className="font-semibold">{topic.title}</span>
                    <span className="text-secondary line-clamp-1 text-xs" title={topic.sample_questions.join(' / ')}>
                        {topic.sample_questions.join(' / ')}
                    </span>
                </div>
            ),
        },
        { title: 'People', dataIndex: 'people', key: 'people' },
        { title: 'Sessions', dataIndex: 'session_count', key: 'session_count' },
        {
            title: 'Coverage',
            key: 'coverage',
            tooltip:
                'How the sessions got their numbers. Ran a metric: ran a catalog metric. Adapted: read a metric, then changed it. Re-derived: computed a metric in SQL without the catalog. Other SQL: SQL that matches no metric. Query tool: a query tool, without SQL.',
            render: (_, topic) => (
                <CountTags
                    counts={[
                        ['Ran a metric', topic.coverage.canonical],
                        ['Adapted', topic.coverage.adapted],
                        ['Re-derived', topic.coverage.silent],
                        ['Other SQL', topic.coverage.raw_sql_only],
                        ['Query tool', topic.coverage.no_sql],
                    ]}
                />
            ),
        },
    ]

    const proposedColumns: LemonTableColumns<ProposedRow> = [
        {
            title: 'Proposed metric',
            key: 'name',
            render: (_, row) => (
                <div className="flex min-w-48 flex-col py-1">
                    <Link
                        to={newMetricUrl(proposedRowPrefill(row))}
                        className={row.name ? 'font-semibold' : 'line-clamp-2 font-mono text-xs'}
                        data-attr="data-catalog-usage-proposed-metric-link"
                    >
                        {row.name ?? row.measure}
                    </Link>
                    {row.description && <span className="text-secondary text-xs">{row.description}</span>}
                </div>
            ),
        },
        {
            title: 'Where it is used',
            key: 'sources',
            tooltip: 'Charts count everyone whose visit ran the chart, including people who only viewed it.',
            render: (_, row) => <ProposedMetricSources metric={row} />,
        },
        {
            title: 'Catalog',
            key: 'outcome',
            render: (_, row) => {
                const outcome = OUTCOME_LABELS[row.outcome] ?? { label: row.outcome, type: 'default' as const }
                return <LemonTag type={outcome.type}>{outcome.label}</LemonTag>
            },
        },
        { title: 'People', dataIndex: 'people', key: 'people' },
        { title: 'Days', dataIndex: 'days', key: 'days' },
        { title: 'Runs', dataIndex: 'runs', key: 'runs' },
    ]

    return (
        <div className="flex flex-col gap-6">
            <div className="flex flex-wrap items-center justify-between gap-2">
                <span className="text-secondary">
                    Usage from <TZLabel time={latest.window_start} /> to <TZLabel time={latest.window_end} />
                </span>
                <div className="flex flex-wrap items-center gap-2">
                    <LemonSwitch
                        checked={includeScouts}
                        onChange={setIncludeScouts}
                        label="Include scouts"
                        bordered
                        size="small"
                        data-attr="data-catalog-usage-include-scouts"
                    />
                    {recomputeButton}
                </div>
            </div>
            {scanRunning && (
                <LemonBanner type="info">
                    A new scan is running. These results are from the last finished scan.
                </LemonBanner>
            )}
            {newestScan?.status === 'error' && (
                <LemonBanner type="warning">
                    <div>The last scan failed, so these results are from an earlier scan.</div>
                    {newestScan.error_message && <div className="text-xs">{newestScan.error_message}</div>}
                </LemonBanner>
            )}
            <div className="@container">
                <div className="grid grid-cols-1 items-start gap-6 @4xl:grid-cols-2">
                    <section className="flex min-w-0 flex-col gap-2">
                        <h3 className="m-0 flex items-center gap-1">
                            Common topics
                            {!failureReasons.topics && topicSessionsSummary && (
                                <Tooltip title={topicSessionsSummary}>
                                    <IconInfo className="text-secondary text-base" />
                                </Tooltip>
                            )}
                        </h3>
                        {failureReasons.topics ? (
                            <SectionFailed reason={failureReasons.topics} />
                        ) : (
                            failureReasons.adjusted_runs && (
                                <LemonBanner type="warning">{AGENT_COUNTS_MISSING}</LemonBanner>
                            )
                        )}
                        {!failureReasons.topics && (
                            <LemonTable
                                id={TOPICS_TABLE_ID}
                                columns={topicColumns}
                                dataSource={topics}
                                rowKey="title"
                                size="small"
                                pagination={{ pageSize: HALF_TABLE_PAGE_SIZE, hideOnSinglePage: true }}
                                emptyState="No agent session asked a data question, worked on a customer request, or investigated a problem in this window."
                                data-attr="data-catalog-usage-topics"
                            />
                        )}
                    </section>
                    <section className="flex min-w-0 flex-col gap-2">
                        <h3 className="m-0">Top used metrics</h3>
                        {failureReasons.metric_runs && <SectionFailed reason={failureReasons.metric_runs} />}
                        {failureReasons.adjusted_runs && (
                            <LemonBanner type="warning">{AGENT_COUNTS_MISSING}</LemonBanner>
                        )}
                        {!failureReasons.metric_runs && (
                            <LemonTable
                                id={METRICS_TABLE_ID}
                                columns={metricColumns}
                                dataSource={metricRows}
                                rowKey="metricId"
                                size="small"
                                pagination={{ pageSize: HALF_TABLE_PAGE_SIZE, hideOnSinglePage: true }}
                                emptyState="No agent ran a catalog metric in this window."
                                data-attr="data-catalog-usage-metric-runs"
                            />
                        )}
                    </section>
                </div>
            </div>
            <section className="flex flex-col gap-2">
                <h3 className="m-0">Proposed metrics</h3>
                {failureReasons.uncovered && <SectionFailed reason={failureReasons.uncovered} />}
                {!failureReasons.uncovered && (
                    <LemonTable
                        id={PROPOSED_TABLE_ID}
                        columns={proposedColumns}
                        dataSource={proposedRows}
                        rowKey="key"
                        size="small"
                        pagination={{ pageSize: PROPOSED_PAGE_SIZE, hideOnSinglePage: true }}
                        emptyState="No measure needs a metric yet. One appears here once several people compute the same measure."
                        expandable={{
                            expandedRowRender: (row) => (
                                <div className="flex flex-col gap-2 p-2">
                                    {row.measures > 1 && (
                                        <p className="m-0">This proposal merges {row.measures} recurring measures.</p>
                                    )}
                                    <CodeSnippet language={Language.SQL} wrap>
                                        {row.sql}
                                    </CodeSnippet>
                                </div>
                            ),
                        }}
                        data-attr="data-catalog-usage-proposed"
                    />
                )}
            </section>
        </div>
    )
}
