import { computeFallbackChangePercent } from './internals'

describe('computeFallbackChangePercent', () => {
    it.each([
        { name: 'no series', data: null, prevPoint: false, index: -1, live: 5, baseline: 1, expected: null },
        {
            name: 'no baseline',
            data: [0, 0],
            prevPoint: false,
            index: -1,
            live: 0,
            baseline: undefined,
            expected: null,
        },
        {
            name: 'live vs baseline',
            data: [100, 150],
            prevPoint: false,
            index: -1,
            live: 150,
            baseline: 100,
            expected: 50,
        },
        {
            name: 'a negative baseline that rises',
            data: [-100, -50],
            prevPoint: false,
            index: -1,
            live: -50,
            baseline: -100,
            expected: 50,
        },
        {
            name: 'hovered point vs previous',
            data: [100, 150, 75],
            prevPoint: true,
            index: 2,
            live: 75,
            baseline: 100,
            expected: -50,
        },
        {
            name: 'hovered first point',
            data: [100, 150],
            prevPoint: true,
            index: 0,
            live: 100,
            baseline: 100,
            expected: null,
        },
    ])('returns $expected for $name', ({ data, prevPoint, index, live, baseline, expected }) => {
        expect(computeFallbackChangePercent(data, prevPoint, index, live, baseline)).toBe(expected)
    })
})
