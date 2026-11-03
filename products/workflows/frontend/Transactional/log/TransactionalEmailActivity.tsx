import { useActions, useValues } from 'kea'

import { LemonBanner, LemonButton, LemonSkeleton } from '@posthog/lemon-ui'

import { dayjs } from 'lib/dayjs'
import { pluralize } from 'lib/utils/strings'

import type { TransactionalEmailLogDetailApi } from 'products/messaging/frontend/generated/api.schemas'

import { ACTIVITY_WINDOW_DAYS, buildActivityTimeline } from './emailActivity'
import { TransactionalEmailActivityRow } from './TransactionalEmailActivityRow'
import { TransactionalEmailActivitySummary } from './TransactionalEmailActivitySummary'
import { transactionalEmailLogEntryLogic } from './transactionalEmailLogEntryLogic'

/** How many events before the send show until the reader asks for more. */
const BEFORE_PREVIEW = 5

export function TransactionalEmailActivity({ email }: { email: TransactionalEmailLogDetailApi }): JSX.Element {
    const { person, personLoading, activity, activityLoading, activityFailed, beforeExpanded, otherEmails } = useValues(
        transactionalEmailLogEntryLogic
    )
    const { loadActivity, showAllBefore } = useActions(transactionalEmailLogEntryLogic)

    const firstName = person?.properties?.name ? String(person.properties.name).split(' ')[0] : null
    const title = firstName ? `What ${firstName} did around this email` : 'Activity around this email'

    let content: JSX.Element
    if (!email.distinct_id) {
        content = (
            <p className="text-sm text-secondary m-0">
                This send has no person attached. Pass a <code>distinct_id</code> when you send, and the person's
                activity before and after the email shows up here.
            </p>
        )
    } else if (person === null) {
        content = (
            <p className="text-sm text-secondary m-0">
                No events from this person yet. Once your app captures events for them, what they did before and after
                the email shows up here.
            </p>
        )
    } else if (activityFailed && !activityLoading && person) {
        content = (
            <LemonBanner
                type="error"
                action={{
                    children: 'Try again',
                    onClick: () => loadActivity({ personId: person.id, sentAt: email.created_at }),
                    'data-attr': 'transactional-email-activity-retry',
                }}
            >
                The activity couldn't load. Check your connection and try again.
            </LemonBanner>
        )
    } else if (!activity || activityLoading || personLoading) {
        content = (
            <div className="flex flex-col gap-3">
                {Array.from({ length: 6 }).map((_, index) => (
                    <LemonSkeleton key={index} className="h-5" />
                ))}
            </div>
        )
    } else {
        const before = beforeExpanded ? activity.before : activity.before.slice(-BEFORE_PREVIEW)
        const hiddenBefore = activity.before.length - before.length
        const emailEvents = email.timeline.filter((event) => event.kind !== 'queued')
        // Other emails before the send only show alongside the events the reader chose to show.
        const earliestShown = before[0]?.timestamp ?? email.created_at
        const shownOtherEmails = otherEmails.filter((entry) => !dayjs(entry.created_at).isBefore(earliestShown))
        // Times read relative to when the email went out, which is what a reader compares against.
        const sentAt = email.timeline.find((event) => event.kind === 'sent')?.at ?? email.created_at
        const timeline = buildActivityTimeline([...before, ...activity.after], emailEvents, shownOtherEmails, sentAt)
        const nothingAround = !activity.before.length && !activity.after.length
        content = (
            <div className="flex flex-col gap-4">
                <TransactionalEmailActivitySummary after={activity.after} timeline={email.timeline} />
                {timeline.before.length > 0 && (
                    <div className="flex flex-col gap-2">
                        <div className="flex items-center justify-between gap-2">
                            <h4 className="text-sm font-semibold m-0">Before the email</h4>
                            {hiddenBefore > 0 && (
                                <LemonButton
                                    size="xsmall"
                                    type="tertiary"
                                    onClick={showAllBefore}
                                    data-attr="transactional-email-activity-show-earlier"
                                >
                                    Show {pluralize(hiddenBefore, 'earlier event')}
                                </LemonButton>
                            )}
                        </div>
                        <ol className="flex flex-col gap-1 m-0 p-0 list-none">
                            {timeline.before.map((row) => (
                                <TransactionalEmailActivityRow key={row.key} row={row} sentAt={sentAt} />
                            ))}
                        </ol>
                    </div>
                )}
                <div className="flex flex-col gap-2">
                    <h4 className="text-sm font-semibold m-0">After the email</h4>
                    <ol className="flex flex-col gap-1 m-0 p-0 list-none" data-attr="transactional-email-activity">
                        {timeline.after.map((row) => (
                            <TransactionalEmailActivityRow key={row.key} row={row} sentAt={sentAt} />
                        ))}
                    </ol>
                    {!activity.after.length && (
                        <p
                            className="text-sm text-secondary m-0 rounded border border-dashed px-3 py-2"
                            data-attr="transactional-email-activity-nothing-after"
                        >
                            {nothingAround
                                ? `No events from this person in the ${pluralize(ACTIVITY_WINDOW_DAYS, 'day')} before or after this email.`
                                : 'No activity in your product since this email.'}
                        </p>
                    )}
                    {activity.hasMoreAfter && (
                        <p className="text-xs text-secondary m-0">
                            Showing the first {activity.after.length} events after the email. See the rest on the
                            person's page.
                        </p>
                    )}
                </div>
            </div>
        )
    }

    return (
        <section className="border rounded bg-surface-primary p-4 flex flex-col gap-3">
            <div className="flex flex-col gap-0.5">
                <h3 className="text-base font-semibold m-0">{title}</h3>
                <p className="text-xs text-secondary m-0">
                    The person's events in your product, with this email's delivery and engagement in between.
                </p>
            </div>
            {content}
        </section>
    )
}
