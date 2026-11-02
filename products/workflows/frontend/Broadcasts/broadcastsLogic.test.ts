import { router } from 'kea-router'

import api from 'lib/api'

import { useMocks } from '~/mocks/jest'
import type { HogQLQueryResponse } from '~/queries/schema/schema-general'
import { initKeaTests } from '~/test/init'

import type { HogFlowBatchJobApi } from 'products/workflows/frontend/generated/api.schemas'

import { broadcastsLogic, loadRunMetricTotals } from './broadcastsLogic'

describe('broadcastsLogic', () => {
    let logic: ReturnType<typeof broadcastsLogic.build>

    beforeEach(() => {
        useMocks({
            get: {
                '/api/environments/:team_id/hog_flows/': { results: [], count: 0 },
                '/api/projects/:team_id/hog_flows/': { results: [], count: 0 },
            },
        })
        initKeaTests()
        logic = broadcastsLogic()
        logic.mount()
    })

    afterEach(() => logic.unmount())

    it("loads a run's error count with its other totals, for the list's errors tag", async () => {
        const rows: [number, string][] = [
            [0, 'email_sent'],
            [245, 'failed'],
        ]
        jest.spyOn(api, 'queryHogQL').mockImplementation(async (query) => {
            const filtered = String(query).includes('metric_name IN')
            return {
                results: rows.filter(([, name]) => !filtered || String(query).includes(`'${name}'`)),
            } as HogQLQueryResponse
        })

        const totals = await loadRunMetricTotals(
            { id: 'run-1', created_at: '2026-10-28T15:19:47Z' } as HogFlowBatchJobApi,
            'UTC'
        )

        expect(totals.failed).toEqual(245)
        jest.restoreAllMocks()
    })

    it('keeps the filters when a link back to the list carries no query params', () => {
        router.actions.push('/broadcasts?status=sent&search=welcome')
        expect(logic.values.filters.status).toEqual('sent')

        router.actions.push('/broadcasts')

        expect(logic.values.filters.status).toEqual('sent')
        expect(logic.values.filters.search).toEqual('welcome')
        expect(router.values.searchParams).toMatchObject({ status: 'sent', search: 'welcome' })
    })

    it('defaults the tags filter when the saved filters have none', () => {
        logic.actions.setFilters({ search: 'welcome' })
        const key = Object.keys(localStorage).find((k) => k.includes('broadcasts_list_filters'))!
        const { tags: _, ...savedBeforeTags } = JSON.parse(localStorage.getItem(key)!)
        localStorage.setItem(key, JSON.stringify(savedBeforeTags))
        logic.unmount()
        router.actions.push('/broadcasts')

        logic = broadcastsLogic()
        logic.mount()

        expect(logic.values.filters).toMatchObject({ search: 'welcome', tags: [] })
    })
})
