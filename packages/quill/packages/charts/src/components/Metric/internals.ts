export interface ChangeColor {
    background: string
    foreground: string
}

// Percent change from the point before `index` to the point at `index`. Returns null when there is
// no usable previous point (first index, missing/non-finite values, or a zero baseline).
export function changeFromPreviousPoint(data: number[], index: number): number | null {
    const prev = data[index - 1]
    const curr = data[index]
    if (index < 1 || prev === 0 || !Number.isFinite(prev) || !Number.isFinite(curr)) {
        return null
    }
    return ((curr - prev) / Math.abs(prev)) * 100
}

// The hover-driven change percent: the hovered point vs the previous point when
// `hoverChangeFromPreviousPoint` is active, otherwise the live value vs the series baseline.
export function computeFallbackChangePercent(
    sparklineData: number[] | null,
    usePrevPointHover: boolean,
    intentIndex: number,
    liveValue: number,
    baselineValue: number | undefined
): number | null {
    if (sparklineData == null) {
        return null
    }
    if (usePrevPointHover) {
        return changeFromPreviousPoint(sparklineData, intentIndex)
    }
    if (baselineValue == null) {
        return null
    }
    return ((liveValue - baselineValue) / Math.abs(baselineValue)) * 100
}
