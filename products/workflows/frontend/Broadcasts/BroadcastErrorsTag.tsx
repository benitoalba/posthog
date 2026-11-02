import { LemonTag, Tooltip } from '@posthog/lemon-ui'

import { humanFriendlyNumber } from 'lib/utils/numbers'

export function BroadcastErrorsTag({ count }: { count: number }): JSX.Element | null {
    if (count <= 0) {
        return null
    }
    return (
        <Tooltip title="These sends hit an error and never went out. Open the broadcast to see the error.">
            <LemonTag type="danger" data-attr="broadcast-errors-tag">
                {`${humanFriendlyNumber(count)} ${count === 1 ? 'error' : 'errors'}`}
            </LemonTag>
        </Tooltip>
    )
}
