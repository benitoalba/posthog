import { useActions, useValues } from 'kea'

import { LemonButton, LemonModal, Spinner } from '@posthog/lemon-ui'

import { getIntegrationNameFromKind } from 'lib/integrations/utils'

import { PushTokenImportForm } from './PushTokenImportForm'
import { pushTokenImportLogic } from './pushTokenImportLogic'
import { PushTokenImportProgress } from './PushTokenImportProgress'
import { PushTokenImportResult } from './PushTokenImportResult'

/** Registers devices from a CSV export, for example from another push provider, on an APNs or Firebase channel. */
export function PushTokenImportModal(): JSX.Element {
    const {
        isOpen,
        integration,
        loadingLatest,
        currentImport,
        isRunning,
        submitting,
        isStopping,
        startDisabledReason,
    } = useValues(pushTokenImportLogic)
    const { closeImport, startImport, importUnknownPersons, stopImport, importAnotherFile } =
        useActions(pushTokenImportLogic)

    const footer = loadingLatest ? null : isRunning ? (
        <>
            <LemonButton type="secondary" onClick={closeImport}>
                Close
            </LemonButton>
            <LemonButton
                type="secondary"
                status="danger"
                onClick={stopImport}
                disabledReason={isStopping ? 'Stopping the import' : null}
                data-attr="push-token-import-stop"
            >
                Stop import
            </LemonButton>
        </>
    ) : currentImport ? (
        <>
            <LemonButton type="secondary" onClick={importAnotherFile} data-attr="push-token-import-another-file">
                Import another file
            </LemonButton>
            {currentImport.can_import_unknown_persons ? (
                <LemonButton
                    type="secondary"
                    onClick={importUnknownPersons}
                    loading={submitting}
                    data-attr="push-token-import-create-persons"
                >
                    Import anyway and create new persons
                </LemonButton>
            ) : null}
            <LemonButton type="primary" onClick={closeImport}>
                Done
            </LemonButton>
        </>
    ) : (
        <>
            <LemonButton
                type="secondary"
                onClick={closeImport}
                disabledReason={submitting ? 'Uploading the file' : null}
            >
                Cancel
            </LemonButton>
            <LemonButton
                type="primary"
                onClick={startImport}
                loading={submitting}
                disabledReason={submitting ? null : startDisabledReason}
                data-attr="push-token-import-start"
            >
                Import
            </LemonButton>
        </>
    )

    return (
        <LemonModal
            isOpen={isOpen}
            onClose={submitting ? undefined : closeImport}
            title="Import devices"
            description={
                integration
                    ? `${getIntegrationNameFromKind(integration.kind)} channel: ${integration.display_name}`
                    : undefined
            }
            width={720}
            footer={footer}
        >
            {/* Distinct IDs and device tokens stay out of session recordings. */}
            <div className="flex flex-col gap-4 ph-no-capture ph-replay-block" data-attr="push-token-import-modal">
                {loadingLatest ? (
                    <Spinner />
                ) : isRunning ? (
                    <PushTokenImportProgress />
                ) : currentImport ? (
                    <PushTokenImportResult />
                ) : (
                    <PushTokenImportForm />
                )}
            </div>
        </LemonModal>
    )
}
