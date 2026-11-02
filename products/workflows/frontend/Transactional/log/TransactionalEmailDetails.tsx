import { LemonTag, Link, Tooltip } from '@posthog/lemon-ui'

import { CopyToClipboardInline } from 'lib/components/CopyToClipboard'
import { TZLabel } from 'lib/components/TZLabel'
import { urls } from 'scenes/urls'

import { PersonsTabType } from '~/types'

import type { TransactionalEmailLogDetailApi } from 'products/messaging/frontend/generated/api.schemas'

import { EMAIL_STATUS_DESCRIPTION, EMAIL_STATUS_LABEL, EMAIL_STATUS_TAG, emailReasonText } from './emailLogStatus'

function DetailRow({ label, children }: { label: string; children: React.ReactNode }): JSX.Element {
    return (
        <div className="flex flex-col gap-0.5">
            <dt className="text-xs text-secondary">{label}</dt>
            <dd className="text-sm m-0 break-words min-w-0">{children}</dd>
        </div>
    )
}

export function TransactionalEmailDetails({ email }: { email: TransactionalEmailLogDetailApi }): JSX.Element {
    const reason = emailReasonText(email.reason)
    return (
        <dl className="border rounded bg-surface-primary p-4 flex flex-col gap-3 m-0">
            <DetailRow label="Status">
                <div className="flex flex-col gap-1 items-start">
                    <Tooltip title={EMAIL_STATUS_DESCRIPTION[email.status]}>
                        <LemonTag type={EMAIL_STATUS_TAG[email.status]}>{EMAIL_STATUS_LABEL[email.status]}</LemonTag>
                    </Tooltip>
                    {reason && <span className="text-secondary">{reason}</span>}
                </div>
            </DetailRow>
            <DetailRow label="Email">
                {email.template_id ? (
                    <Link to={urls.transactionalTemplate(email.template_id)}>
                        {email.template_name || 'Unnamed email'}
                    </Link>
                ) : (
                    <span className="text-secondary">Deleted email</span>
                )}
            </DetailRow>
            <DetailRow label="Queued">
                <TZLabel time={email.created_at} />
            </DetailRow>
            {email.sending_domain && <DetailRow label="Sending domain">{email.sending_domain}</DetailRow>}
            <DetailRow label="Message ID">
                <CopyToClipboardInline description="message ID" explicitValue={email.message_id} iconSize="xsmall">
                    <span className="font-mono text-xs break-all">{email.message_id}</span>
                </CopyToClipboardInline>
            </DetailRow>
            {email.distinct_id && (
                <DetailRow label="Person">
                    <div className="flex flex-col gap-1">
                        <Link
                            to={urls.personByDistinctId(email.distinct_id)}
                            className="break-all"
                            data-attr="transactional-email-log-person"
                        >
                            {email.distinct_id}
                        </Link>
                        <Link
                            to={`${urls.personByDistinctId(email.distinct_id)}#activeTab=${PersonsTabType.SESSION_RECORDINGS}`}
                            data-attr="transactional-email-log-recordings"
                        >
                            View their recordings
                        </Link>
                    </div>
                </DetailRow>
            )}
        </dl>
    )
}
