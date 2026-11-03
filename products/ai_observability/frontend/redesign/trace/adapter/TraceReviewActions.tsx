import { useActions, useValues } from 'kea'
import { useState } from 'react'

import { LemonButton, LemonDialog, LemonInput } from '@posthog/lemon-ui'

import { LemonField } from 'lib/lemon-ui/LemonField'
import { LemonMenuItem, LemonMenuOverlay } from 'lib/lemon-ui/LemonMenu'

import type {
    ReviewQueueApi,
    TraceReviewQueueAssignmentApi,
    TraceReviewStatusApi,
} from '../../../generated/api.schemas'
import { reviewQueuePickerModalLogic } from '../../../reviewQueues/reviewQueuePickerModalLogic'
import { TraceReviewButton } from '../../../traceReviews/TraceReviewButton'

export interface TraceReviewActionsProps {
    traceId: string
    status: TraceReviewStatusApi | null
    loading: boolean
    failed: boolean
    removing: boolean
    onQueueChanged: () => void
    onRemoveFromQueue: () => void
}

export function TraceReviewActions({
    traceId,
    status,
    loading,
    failed,
    removing,
    onQueueChanged,
    onRemoveFromQueue,
}: TraceReviewActionsProps): JSX.Element | null {
    if (!status) {
        return loading || !failed ? null : (
            <LemonButton
                type="secondary"
                size="small"
                disabledReason="Couldn't load this trace's review. Reload the page to try again."
            >
                Review
            </LemonButton>
        )
    }
    if (status.review && !status.queue) {
        return (
            <TraceReviewButton traceId={traceId} buttonType="secondary" buttonSize="small" buttonLabel="Edit review" />
        )
    }
    return (
        <ReviewButtonWithQueueMenu
            traceId={traceId}
            reviewed={!!status.review}
            queue={status.queue}
            removing={removing}
            onQueueChanged={onQueueChanged}
            onRemoveFromQueue={onRemoveFromQueue}
        />
    )
}

function ReviewButtonWithQueueMenu({
    traceId,
    reviewed,
    queue,
    removing,
    onQueueChanged,
    onRemoveFromQueue,
}: {
    traceId: string
    reviewed: boolean
    queue: TraceReviewQueueAssignmentApi | null
    removing: boolean
    onQueueChanged: () => void
    onRemoveFromQueue: () => void
}): JSX.Element {
    const picker = reviewQueuePickerModalLogic({
        traceId,
        queueItemId: queue?.itemId ?? null,
        defaultQueueId: queue?.id ?? null,
        onSuccess: onQueueChanged,
    })
    const { queues, isSubmitting } = useValues(picker)
    const { submit } = useActions(picker)
    const [menuOpen, setMenuOpen] = useState(false)
    const closingMenu =
        (action: () => void): (() => void) =>
        () => {
            setMenuOpen(false)
            action()
        }

    const queueTargets = queues.results.filter((candidate) => candidate.id !== queue?.id)
    const items: LemonMenuItem[] = []
    if (!reviewed) {
        items.push({
            label: queue ? 'Move to another queue' : 'Add to queue',
            items: [
                ...queueTargets.map((candidate) => ({
                    label: candidate.name,
                    onClick: closingMenu(() => submit(candidate.id)),
                    'data-attr': 'trace-view-review-queue-option',
                })),
                {
                    label: 'New queue…',
                    onClick: closingMenu(() => openNewQueueForm(queues.results, submit)),
                    'data-attr': 'trace-view-review-new-queue',
                },
            ],
        })
    }
    if (queue) {
        items.push({
            label: 'Remove from queue',
            status: 'danger',
            onClick: closingMenu(() =>
                LemonDialog.open({
                    title: `Remove from "${queue.name}"?`,
                    primaryButton: { status: 'danger', children: 'Remove from queue', onClick: onRemoveFromQueue },
                    secondaryButton: { children: 'Cancel' },
                })
            ),
            'data-attr': 'trace-view-remove-from-queue',
        })
    }

    return (
        <TraceReviewButton
            traceId={traceId}
            queueId={queue?.id ?? null}
            buttonType="secondary"
            buttonSize="small"
            buttonLabel={reviewed ? 'Edit review' : 'Review'}
            sideAction={{
                dropdown: {
                    overlay: <LemonMenuOverlay items={items} />,
                    placement: 'bottom-end',
                    visible: menuOpen,
                    onVisibilityChange: setMenuOpen,
                },
                onClick: () => setMenuOpen(!menuOpen),
                loading: isSubmitting || removing,
                tooltip: 'Queue actions',
                'aria-label': 'Queue actions',
                'data-attr': 'trace-view-review-queue-menu',
            }}
        />
    )
}

function openNewQueueForm(existing: ReviewQueueApi[], submit: (queueName: string) => void): void {
    const taken = new Set(existing.map((queue) => queue.name.trim().toLowerCase()))
    LemonDialog.openForm({
        title: 'New review queue',
        initialValues: { name: '' },
        content: (
            <LemonField name="name">
                <LemonInput placeholder="Queue name" autoFocus data-attr="trace-view-review-new-queue-name" />
            </LemonField>
        ),
        errors: {
            name: (name: string) =>
                !name?.trim()
                    ? 'Enter a queue name.'
                    : taken.has(name.trim().toLowerCase())
                      ? 'A queue with this name exists. Pick it from the menu.'
                      : undefined,
        },
        onSubmit: ({ name }) => submit(name.trim()),
    })
}
