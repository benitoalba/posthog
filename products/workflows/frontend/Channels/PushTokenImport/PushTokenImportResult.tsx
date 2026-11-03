import { useValues } from 'kea'

import { LemonBanner } from '@posthog/lemon-ui'

import { PushTokenImportStatusEnumApi } from 'products/workflows/frontend/generated/api.schemas'

import { PushTokenFailedRowsTable } from './PushTokenFailedRowsTable'
import { pushTokenImportLogic } from './pushTokenImportLogic'

function plural(count: number, one: string, many: string): string {
    return `${count.toLocaleString()} ${count === 1 ? one : many}`
}

/** Shows what a finished import stored and which lines it could not import. */
export function PushTokenImportResult(): JSX.Element | null {
    const { currentImport, error } = useValues(pushTokenImportLogic)
    if (!currentImport) {
        return null
    }
    const { status, imported_devices, failed_devices, duplicate_lines, failures, unknown_person_devices } =
        currentImport
    const imported = `${plural(imported_devices, 'device', 'devices')} imported.`
    const notImported = failed_devices ? ` ${plural(failed_devices, 'line was', 'lines were')} not imported.` : ''
    const duplicates = duplicate_lines
        ? ` ${plural(duplicate_lines, 'repeated line was', 'repeated lines were')} left out.`
        : ''

    return (
        <>
            {status === PushTokenImportStatusEnumApi.Failed ? (
                <LemonBanner type="error">
                    {currentImport.error} {imported}
                </LemonBanner>
            ) : status === PushTokenImportStatusEnumApi.Cancelled ? (
                <LemonBanner type="warning">You stopped the import. {imported}</LemonBanner>
            ) : (
                <LemonBanner type={failed_devices ? 'warning' : 'success'}>
                    {imported}
                    {notImported}
                    {duplicates}
                </LemonBanner>
            )}
            {error ? <LemonBanner type="error">{error}</LemonBanner> : null}
            {currentImport.can_import_unknown_persons ? (
                <span className="text-sm">
                    {plural(unknown_person_devices, 'device has', 'devices have')} a distinct ID that no person in
                    PostHog has. Check that it's the ID your app passes to identify. If the IDs are right, import them
                    anyway to create those persons.
                </span>
            ) : null}
            {failures.length > 0 ? <PushTokenFailedRowsTable rows={failures} /> : null}
            {failed_devices > failures.length ? (
                <span className="text-xs text-secondary">
                    The table shows the first {failures.length.toLocaleString()} lines that were not imported.
                </span>
            ) : null}
        </>
    )
}
