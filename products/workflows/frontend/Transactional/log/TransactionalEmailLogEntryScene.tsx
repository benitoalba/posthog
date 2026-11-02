import { useValues } from 'kea'

import { LemonButton, LemonSkeleton } from '@posthog/lemon-ui'

import { NotFound } from 'lib/components/NotFound'
import { useFeatureFlag } from 'lib/hooks/useFeatureFlag'
import { SceneExport } from 'scenes/sceneTypes'
import { urls } from 'scenes/urls'

import { SceneContent } from '~/layout/scenes/components/SceneContent'
import { SceneTitleSection } from '~/layout/scenes/components/SceneTitleSection'
import { ProductKey } from '~/queries/schema/schema-general'

import { TransactionalEmailDetails } from './TransactionalEmailDetails'
import {
    TransactionalEmailLogEntryLogicProps,
    transactionalEmailLogEntryLogic,
} from './transactionalEmailLogEntryLogic'
import { TransactionalEmailRendered } from './TransactionalEmailRendered'
import { TransactionalEmailTimeline } from './TransactionalEmailTimeline'

export const scene: SceneExport<TransactionalEmailLogEntryLogicProps> = {
    component: TransactionalEmailLogEntryScene,
    logic: transactionalEmailLogEntryLogic,
    paramsToProps: ({ params: { id } }): TransactionalEmailLogEntryLogicProps => ({ messageId: id }),
    productKey: ProductKey.WORKFLOWS,
}

export function TransactionalEmailLogEntryScene(): JSX.Element {
    const showTransactional = useFeatureFlag('WORKFLOWS_TRANSACTIONAL_EMAIL_API')
    // The scene binds the logic to the message ID in the URL.
    const { email, emailLoading } = useValues(transactionalEmailLogEntryLogic)

    if (!showTransactional) {
        return <NotFound object="page" />
    }
    if (email === null && !emailLoading) {
        return (
            <NotFound
                object="email"
                caption="No send of this project has this message ID, or it is older than 30 days."
            />
        )
    }

    return (
        <SceneContent>
            <SceneTitleSection
                name={email?.recipient || 'Email'}
                description={email ? email.subject || email.template_name || null : null}
                resourceType={{ type: 'transactional' }}
                isLoading={!email}
                actions={
                    email?.template_id ? (
                        <LemonButton
                            type="secondary"
                            size="small"
                            to={urls.transactionalTemplate(email.template_id)}
                            data-attr="transactional-email-log-entry-open-template"
                        >
                            Open email
                        </LemonButton>
                    ) : undefined
                }
            />
            {!email ? (
                <LemonSkeleton className="h-100" />
            ) : (
                <div className="@container">
                    <div className="flex flex-col @3xl:flex-row gap-4 items-start">
                        <div className="w-full @3xl:flex-1 min-w-0">
                            <TransactionalEmailRendered email={email} />
                        </div>
                        <div className="w-full @3xl:w-80 shrink-0 flex flex-col gap-4">
                            <TransactionalEmailDetails email={email} />
                            <div className="border rounded bg-surface-primary p-4">
                                <h3 className="text-sm font-semibold mb-3">Timeline</h3>
                                <TransactionalEmailTimeline events={email.timeline} />
                            </div>
                        </div>
                    </div>
                </div>
            )}
        </SceneContent>
    )
}
