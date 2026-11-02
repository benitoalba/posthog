import { IconLock } from '@posthog/icons'
import { LemonBanner } from '@posthog/lemon-ui'

import type { TransactionalEmailLogDetailApi } from 'products/messaging/frontend/generated/api.schemas'

import { getMessageAssetContentUrl } from '../../Workflows/messageAssetsApi'

export function TransactionalEmailRendered({ email }: { email: TransactionalEmailLogDetailApi }): JSX.Element {
    if (email.sensitive) {
        return (
            <LemonBanner type="info" icon={<IconLock />}>
                This email is marked sensitive, so PostHog keeps no copy of what it contained.
            </LemonBanner>
        )
    }
    if (!email.action_id) {
        return (
            <LemonBanner type="info">
                {email.status === 'failed' || email.status === 'queued'
                    ? "This email wasn't sent, so there's no copy of it."
                    : 'No copy of this email was kept.'}
            </LemonBanner>
        )
    }
    return (
        <div className="border rounded bg-surface-primary overflow-hidden">
            <div className="px-4 py-2 border-b text-sm">
                <span className="text-secondary">Subject: </span>
                <span className="font-medium">{email.subject}</span>
            </div>
            {/* sandbox="" disables scripts so the captured email HTML can't run anything. */}
            <iframe
                title="Rendered email"
                sandbox=""
                src={getMessageAssetContentUrl(email.workflow_id, email.message_id, email.action_id)}
                className="w-full h-[70vh] bg-white block"
            />
        </div>
    )
}
