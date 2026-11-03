import { dayjs } from 'lib/dayjs'
import { humanFriendlyDuration } from 'lib/utils/durations'

import type {
    EmailLogEventKindEnumApi,
    TransactionalEmailLogDetailApi,
    TransactionalEmailLogEventApi,
} from 'products/messaging/frontend/generated/api.schemas'

import { EMAIL_STATUS_LABEL, emailReasonText } from './emailLogStatus'

export type DeliveryStepState = 'done' | 'failed' | 'pending'

export interface DeliveryStep {
    key: string
    label: string
    state: DeliveryStepState
    at: string | null
    detail: string | null
    previousAt: string | null
}

const FAILURE_KINDS: EmailLogEventKindEnumApi[] = ['bounced', 'complained', 'failed']

function firstOf(
    timeline: readonly TransactionalEmailLogEventApi[],
    kind: EmailLogEventKindEnumApi
): TransactionalEmailLogEventApi | undefined {
    return timeline.find((event) => event.kind === kind)
}

function countOf(timeline: readonly TransactionalEmailLogEventApi[], kind: EmailLogEventKindEnumApi): number {
    return timeline.filter((event) => event.kind === kind).length
}

/** The steps an email went through, ending at its first failure. */
export function buildDeliverySteps(email: TransactionalEmailLogDetailApi): DeliveryStep[] {
    const { timeline } = email
    const queued = firstOf(timeline, 'queued')
    const sent = firstOf(timeline, 'sent')
    const delivered = firstOf(timeline, 'delivered')
    const opened = firstOf(timeline, 'opened')
    const clicked = firstOf(timeline, 'clicked')
    const failure = timeline.find((event) => FAILURE_KINDS.includes(event.kind))
    const queuedAt = queued?.at ?? email.created_at
    const opens = countOf(timeline, 'opened')
    const clicks = countOf(timeline, 'clicked')

    // A send that went out shows Sent as its first step. Queued only matters while it waits, or when it stopped there.
    const steps: DeliveryStep[] =
        sent && email.status !== 'failed'
            ? []
            : [{ key: 'queued', label: 'Queued', state: 'done', at: queuedAt, detail: null, previousAt: null }]
    if (email.status === 'failed') {
        steps.push({
            key: 'failed',
            label: EMAIL_STATUS_LABEL.failed,
            state: 'failed',
            at: failure?.at ?? null,
            detail: emailReasonText(failure?.reason ?? email.reason),
            previousAt: queuedAt,
        })
        return steps
    }
    steps.push({
        key: 'sent',
        label: 'Sent',
        state: sent ? 'done' : 'pending',
        at: sent?.at ?? null,
        detail: null,
        previousAt: steps.length ? queuedAt : null,
    })
    if (email.status === 'bounced') {
        steps.push({
            key: 'bounced',
            label: EMAIL_STATUS_LABEL.bounced,
            state: 'failed',
            at: failure?.at ?? null,
            detail: emailReasonText(failure?.reason ?? email.reason),
            previousAt: sent?.at ?? null,
        })
        return steps
    }
    steps.push({
        key: 'delivered',
        label: 'Delivered',
        state: delivered || email.status === 'complained' ? 'done' : 'pending',
        at: delivered?.at ?? null,
        detail: null,
        previousAt: sent?.at ?? null,
    })
    if (email.status === 'complained') {
        steps.push({
            key: 'complained',
            label: EMAIL_STATUS_LABEL.complained,
            state: 'failed',
            at: failure?.at ?? null,
            detail: 'The recipient reported this email as spam',
            previousAt: delivered?.at ?? sent?.at ?? null,
        })
        return steps
    }
    steps.push(
        {
            key: 'opened',
            label: 'Opened',
            state: opened ? 'done' : 'pending',
            at: opened?.at ?? null,
            detail: opens > 1 ? `Opened ${opens} times` : null,
            previousAt: delivered?.at ?? null,
        },
        {
            key: 'clicked',
            label: 'Clicked',
            state: clicked ? 'done' : 'pending',
            at: clicked?.at ?? null,
            detail: clicks > 1 ? `Clicked ${clicks} times` : null,
            previousAt: opened?.at ?? delivered?.at ?? null,
        }
    )
    return steps
}

/** "3s later", for the time between a step and the one before it. */
export function gapLabel(at: string, previousAt: string | null): string | null {
    if (!previousAt) {
        return null
    }
    const seconds = dayjs(at).diff(dayjs(previousAt), 'second')
    if (seconds < 1) {
        return 'under a second later'
    }
    return `${humanFriendlyDuration(seconds, { maxUnits: 1 })} later`
}
