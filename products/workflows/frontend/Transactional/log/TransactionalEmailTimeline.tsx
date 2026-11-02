import clsx from 'clsx'

import { TZLabel } from 'lib/components/TZLabel'
import { dayjs } from 'lib/dayjs'
import { humanFriendlyDuration } from 'lib/utils/durations'

import type {
    EmailLogEventKindEnumApi,
    TransactionalEmailLogEventApi,
} from 'products/messaging/frontend/generated/api.schemas'

import { EMAIL_STATUS_LABEL, emailReasonText } from './emailLogStatus'

const DOT_COLOR: Record<EmailLogEventKindEnumApi, string> = {
    queued: 'bg-border-bold',
    sent: 'bg-accent',
    delivered: 'bg-success',
    opened: 'bg-success',
    clicked: 'bg-success',
    bounced: 'bg-danger',
    complained: 'bg-danger',
    failed: 'bg-danger',
}

export function TransactionalEmailTimeline({
    events,
}: {
    events: readonly TransactionalEmailLogEventApi[]
}): JSX.Element {
    if (events.length === 0) {
        return <p className="text-secondary text-sm m-0">Nothing has happened to this email yet.</p>
    }
    const start = dayjs(events[0].at)
    return (
        <ol className="flex flex-col m-0 p-0 list-none" data-attr="transactional-email-log-timeline">
            {events.map((event, index) => {
                const reason = emailReasonText(event.reason)
                const isLast = index === events.length - 1
                const secondsAfter = dayjs(event.at).diff(start, 'second')
                return (
                    <li key={`${event.kind}-${event.at}-${index}`} className="flex gap-3">
                        <div className="flex flex-col items-center">
                            <span className={clsx('size-2.5 rounded-full mt-1.5 shrink-0', DOT_COLOR[event.kind])} />
                            {!isLast && <span className="w-px flex-1 bg-border my-1" />}
                        </div>
                        <div className={clsx('flex flex-col min-w-0', !isLast && 'pb-3')}>
                            <span className="text-sm font-medium">{EMAIL_STATUS_LABEL[event.kind]}</span>
                            <span className="text-xs text-secondary flex flex-wrap gap-x-1">
                                <TZLabel time={event.at} formatDate="MMM D" formatTime="HH:mm:ss" />
                                {index > 0 && (
                                    <span>
                                        ·{' '}
                                        {secondsAfter < 1
                                            ? 'under a second later'
                                            : `${humanFriendlyDuration(secondsAfter, { maxUnits: 2 })} later`}
                                    </span>
                                )}
                            </span>
                            {reason && <span className="text-xs text-secondary">{reason}</span>}
                        </div>
                    </li>
                )
            })}
        </ol>
    )
}
