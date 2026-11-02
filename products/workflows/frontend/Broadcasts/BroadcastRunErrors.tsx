import { useValues } from 'kea'

import { LemonBanner } from '@posthog/lemon-ui'

import { dayjs } from 'lib/dayjs'
import { humanFriendlyNumber } from 'lib/utils/numbers'
import { logsViewerLogic } from 'scenes/hog-functions/logs/logsViewerLogic'

import { BroadcastPerformanceLogicProps, broadcastPerformanceLogic } from './broadcastPerformanceLogic'

function RunErrorsBanner({
    runId,
    runStartedAt,
    channel,
    count,
}: BroadcastPerformanceLogicProps & { count: number }): JSX.Element {
    const { unGroupedLogs } = useValues(
        logsViewerLogic({
            logicKey: `broadcast-run-errors-${runId}`,
            sourceType: 'hog_flow',
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

export function BroadcastRunErrors(props: BroadcastPerformanceLogicProps): JSX.Element | null {
    const { errored } = useValues(broadcastPerformanceLogic(props))
    return errored > 0 ? <RunErrorsBanner {...props} count={errored} /> : null
}
