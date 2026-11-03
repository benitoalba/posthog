import clsx from 'clsx'

import { IconCheck, IconX } from '@posthog/icons'
import { Tooltip } from '@posthog/lemon-ui'

import { TZLabel } from 'lib/components/TZLabel'

import type { TransactionalEmailLogDetailApi } from 'products/messaging/frontend/generated/api.schemas'

import { buildDeliverySteps, gapLabel } from './emailDeliverySteps'

export function TransactionalEmailDeliveryProgress({ email }: { email: TransactionalEmailLogDetailApi }): JSX.Element {
    const steps = buildDeliverySteps(email)
    return (
        <ol
            className="flex flex-col @2xl:flex-row gap-3 @2xl:gap-0 m-0 p-4 list-none border rounded bg-surface-primary"
            data-attr="transactional-email-delivery-progress"
        >
            {steps.map((step, index) => {
                const gap = step.at ? gapLabel(step.at, step.previousAt) : null
                return (
                    <li key={step.key} className="flex @2xl:flex-col gap-3 @2xl:gap-2 flex-1 min-w-0">
                        <div className="flex items-center @2xl:w-full">
                            <span
                                className={clsx(
                                    'flex items-center justify-center size-6 rounded-full shrink-0 border',
                                    step.state === 'done' && 'bg-success border-success text-white',
                                    step.state === 'failed' && 'bg-danger border-danger text-white',
                                    step.state === 'pending' && 'bg-surface-primary border-dashed border-border-bold'
                                )}
                            >
                                {step.state === 'done' && <IconCheck className="size-4" />}
                                {step.state === 'failed' && <IconX className="size-4" />}
                            </span>
                            {index < steps.length - 1 && (
                                <span
                                    className={clsx(
                                        'hidden @2xl:block h-px flex-1 mx-2',
                                        steps[index + 1].state === 'pending' ? 'bg-border' : 'bg-border-bold'
                                    )}
                                />
                            )}
                        </div>
                        <div className="flex flex-col min-w-0 @2xl:pr-4">
                            <span
                                className={clsx(
                                    'text-sm font-semibold',
                                    step.state === 'pending' && 'text-secondary',
                                    step.state === 'failed' && 'text-danger'
                                )}
                            >
                                {step.label}
                            </span>
                            {step.at ? (
                                <span className="text-xs text-secondary flex flex-wrap gap-x-1">
                                    <TZLabel time={step.at} formatDate="MMM D" formatTime="HH:mm:ss" />
                                    {gap && <span>· {gap}</span>}
                                </span>
                            ) : (
                                step.state === 'pending' && (
                                    <span className="text-xs text-secondary">
                                        {step.key === 'opened' || step.key === 'clicked' ? 'Not yet' : 'Waiting'}
                                    </span>
                                )
                            )}
                            {step.detail &&
                                (step.detail.length > 40 ? (
                                    <Tooltip title={step.detail}>
                                        <span className="text-xs text-secondary truncate">{step.detail}</span>
                                    </Tooltip>
                                ) : (
                                    <span className="text-xs text-secondary">{step.detail}</span>
                                ))}
                        </div>
                    </li>
                )
            })}
        </ol>
    )
}
