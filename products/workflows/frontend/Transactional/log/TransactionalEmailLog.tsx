import { useActions, useValues } from 'kea'
import { router } from 'kea-router'

import { IconCursorClick, IconEye } from '@posthog/icons'
import { LemonBanner, LemonButton, LemonInput, LemonSelect, LemonTag, Tooltip } from '@posthog/lemon-ui'

import { TZLabel } from 'lib/components/TZLabel'
import { LemonTable } from 'lib/lemon-ui/LemonTable'
import { LemonTableLink } from 'lib/lemon-ui/LemonTable/LemonTableLink'
import { urls } from 'scenes/urls'

import type { TransactionalEmailLogEntryApi } from 'products/messaging/frontend/generated/api.schemas'

import { messageTemplatesLogic } from '../../TemplateLibrary/messageTemplatesLogic'
import { EMAIL_STATUS_DESCRIPTION, EMAIL_STATUS_LABEL, EMAIL_STATUS_TAG, emailReasonText } from './emailLogStatus'
import { LOG_DATE_RANGES, transactionalEmailLogLogic } from './transactionalEmailLogLogic'

export function TransactionalEmailLog(): JSX.Element {
    const { emails, pageLoading, pageFailed, hasMore, filters, isFiltered } = useValues(transactionalEmailLogLogic)
    const { setFilters, loadMore, loadEmails } = useActions(transactionalEmailLogLogic)
    const { transactionalTemplates } = useValues(messageTemplatesLogic)

    const emailOptions = transactionalTemplates
        .filter((template) => template.transactional_hog_flow_id)
        .map((template) => ({
            value: template.transactional_hog_flow_id as string,
            label: template.name || 'Unnamed email',
        }))

    return (
        <div className="flex flex-col gap-2">
            <div className="flex flex-wrap items-center gap-2">
                <LemonInput
                    type="search"
                    placeholder="Search by recipient or message ID"
                    value={filters.search}
                    onChange={(search) => setFilters({ search })}
                    className="min-w-60 flex-1 max-w-100"
                    data-attr="transactional-email-log-search"
                />
                <LemonSelect
                    size="small"
                    value={filters.workflowId}
                    onChange={(workflowId) => setFilters({ workflowId })}
                    options={[{ value: null, label: 'All emails' }, ...emailOptions]}
                    data-attr="transactional-email-log-email-filter"
                />
                <LemonSelect
                    size="small"
                    value={filters.status}
                    onChange={(status) => setFilters({ status })}
                    options={[
                        { value: null, label: 'Any status' },
                        ...(['delivered', 'sent', 'queued', 'bounced', 'complained', 'failed'] as const).map(
                            (status) => ({ value: status, label: EMAIL_STATUS_LABEL[status] })
                        ),
                    ]}
                    data-attr="transactional-email-log-status-filter"
                />
                <LemonSelect
                    size="small"
                    value={filters.days}
                    onChange={(days) => setFilters({ days })}
                    options={LOG_DATE_RANGES.map((range) => ({ value: range.value, label: range.label }))}
                    data-attr="transactional-email-log-date-filter"
                />
            </div>
            {pageFailed && (
                <LemonBanner
                    type="error"
                    action={{
                        children: 'Try again',
                        onClick: () => loadEmails(),
                        'data-attr': 'transactional-email-log-retry',
                    }}
                >
                    The log couldn't load. Check your connection and try again.
                </LemonBanner>
            )}
            <LemonTable<TransactionalEmailLogEntryApi>
                dataSource={emails}
                loading={pageLoading}
                rowKey="message_id"
                data-attr="transactional-email-log-table"
                onRow={(email) => ({
                    onClick: () => router.actions.push(urls.transactionalEmailLogEntry(email.message_id)),
                    className: 'cursor-pointer',
                })}
                columns={[
                    {
                        title: 'Recipient',
                        key: 'recipient',
                        render: (_, email) => (
                            <LemonTableLink
                                to={urls.transactionalEmailLogEntry(email.message_id)}
                                title={<span className="break-all">{email.recipient || 'Unknown recipient'}</span>}
                            />
                        ),
                    },
                    {
                        title: 'Email',
                        key: 'template',
                        render: (_, email) => (
                            <span className="whitespace-nowrap">
                                {email.template_name || <span className="text-secondary">Deleted email</span>}
                            </span>
                        ),
                    },
                    {
                        title: 'Status',
                        key: 'status',
                        width: 0,
                        render: (_, email) => (
                            <Tooltip title={emailReasonText(email.reason) ?? EMAIL_STATUS_DESCRIPTION[email.status]}>
                                <LemonTag type={EMAIL_STATUS_TAG[email.status]} className="whitespace-nowrap">
                                    {EMAIL_STATUS_LABEL[email.status]}
                                </LemonTag>
                            </Tooltip>
                        ),
                    },
                    {
                        title: 'Engagement',
                        key: 'engagement',
                        width: 0,
                        render: (_, email) =>
                            email.opened || email.clicked ? (
                                <div className="flex items-center gap-1">
                                    {email.opened && (
                                        <LemonTag icon={<IconEye />} className="whitespace-nowrap">
                                            Opened
                                        </LemonTag>
                                    )}
                                    {email.clicked && (
                                        <LemonTag icon={<IconCursorClick />} className="whitespace-nowrap">
                                            Clicked
                                        </LemonTag>
                                    )}
                                </div>
                            ) : null,
                    },
                    {
                        title: 'Time',
                        key: 'created_at',
                        width: 0,
                        render: (_, email) => <TZLabel time={email.created_at} className="whitespace-nowrap" />,
                    },
                ]}
                emptyState={
                    pageFailed
                        ? "The log couldn't load"
                        : isFiltered
                          ? 'No sends match these filters'
                          : 'No transactional emails sent in this period. Sends from your code show up here within a few seconds.'
                }
            />
            {hasMore && (
                <div className="flex justify-center">
                    <LemonButton
                        type="secondary"
                        size="small"
                        onClick={loadMore}
                        loading={pageLoading}
                        data-attr="transactional-email-log-load-more"
                    >
                        Load more
                    </LemonButton>
                </div>
            )}
        </div>
    )
}
