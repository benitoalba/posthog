import { Meta, StoryFn } from '@storybook/react'

import { TransactionalEmailTimeline } from './TransactionalEmailTimeline'

const meta: Meta<typeof TransactionalEmailTimeline> = {
    title: 'Scenes-App/Transactional/Email timeline',
    component: TransactionalEmailTimeline,
    parameters: { mockDate: '2026-06-15', testOptions: { snapshotBrowsers: [] } },
}
export default meta

const Template: StoryFn<typeof TransactionalEmailTimeline> = (args) => (
    <div className="w-80 p-4 border rounded bg-surface-primary">
        <TransactionalEmailTimeline {...args} />
    </div>
)

export const DeliveredOpenedClicked = Template.bind({})
DeliveredOpenedClicked.args = {
    events: [
        { kind: 'queued', at: '2026-06-15T09:00:00Z', reason: null },
        { kind: 'sent', at: '2026-06-15T09:00:01Z', reason: null },
        { kind: 'delivered', at: '2026-06-15T09:00:03Z', reason: null },
        { kind: 'opened', at: '2026-06-15T09:12:40Z', reason: null },
        { kind: 'clicked', at: '2026-06-15T09:13:05Z', reason: null },
    ],
}

export const Bounced = Template.bind({})
Bounced.args = {
    events: [
        { kind: 'queued', at: '2026-06-15T09:00:00Z', reason: null },
        { kind: 'sent', at: '2026-06-15T09:00:01Z', reason: null },
        { kind: 'bounced', at: '2026-06-15T09:00:04Z', reason: 'hard' },
    ],
}
