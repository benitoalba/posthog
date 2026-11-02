import { useValues } from 'kea'

import { LemonBanner } from '@posthog/lemon-ui'

import { dayjs } from 'lib/dayjs'
import { humanFriendlyNumber } from 'lib/utils/numbers'
import { logsViewerLogic } from 'scenes/hog-functions/logs/logsViewerLogic'

import type { BroadcastChannel } from './broadcastsLogic'

/**
 * Sends that stopped at an error, such as a template that can't render, never reach the provider, so
 * they show up in no send count. The reason exists only in the run's logs, which this reads.
 */
export function BroadcastRunErrors({
    runId,
    runStartedAt,
    count,
    channel = 'email',
}: {
    runId: string
    runStartedAt: string
    count: number
    channel?: BroadcastChannel
}): JSX.Element {
    const { unGroupedLogs } = useValues(
        logsViewerLogic({
            logicKey: `broadcast-run-errors-${runId}`,
            sourceType: 'hog_flow',
            // A batch send logs against the run, not the flow.
            sourceId: runId,
            groupByInstanceId: false,
            disableUrlSync: true,
            defaultFilters: {
                levels: ['ERROR'],
                dateFrom: dayjs(runStartedAt).subtract(1, 'day').format('YYYY-MM-DD'),
            },
        })
    )

    const noun = channel === 'push' ? 'push notification' : 'email'
    const latestError = unGroupedLogs[0]?.message

    return (
        <div data-attr="broadcast-run-errors">
            <LemonBanner type="error">
                <div className="flex flex-col gap-2">
                    <span>{`${humanFriendlyNumber(count)} ${count === 1 ? 'person' : 'people'} didn't get this ${noun} because sending it hit an error. Fix the ${noun}, send yourself a test, then send it again as a new broadcast.`}</span>
                    {latestError ? (
                        <div className="flex flex-col gap-1">
                            <span className="text-xs font-semibold">Latest error</span>
                            <code className="whitespace-pre-wrap wrap-anywhere text-xs">{latestError}</code>
                        </div>
                    ) : null}
                </div>
            </LemonBanner>
        </div>
    )
}
