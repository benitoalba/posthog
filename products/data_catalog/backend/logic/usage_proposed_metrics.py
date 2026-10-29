"""Roll drafted measures up into proposed metrics, and say where people use each one.

A source answers why a metric is worth proposing: a saved chart, a place in the web app, or agent
sessions. Only drafted measures get sources, so the extra read and the extra payload stay small.
"""

from collections import defaultdict
from collections.abc import Iterable, Mapping
from datetime import datetime
from typing import Any

from posthog.clickhouse.client.execute import sync_execute
from posthog.clickhouse.query_tagging import Feature, Product, tags_context
from posthog.clickhouse.workload import Workload
from posthog.dataclasses import frozen
from posthog.models import Team

from products.product_analytics.backend.facade.models import Insight

from ..facade.contracts import AgentSource, ChartSource, ProposedMetric, SurfaceSource, UncoveredGroup

MAX_CHART_SOURCES = 5


@frozen
class MeasureEvidence:
    people: frozenset[str]
    days: frozenset[str]
    shape_hashes: frozenset[int]
    agent_people: frozenset[str]
    agent_sessions: frozenset[str]
    agent_runs: int


@frozen
class ShapeSourceRow:
    query_hash: int
    insight_id: int
    dashboard_id: int
    product: str
    user_ids: frozenset[int]
    runs: int


@frozen
class ChartName:
    short_id: str
    name: str
    dashboards: Mapping[int, str]


def fetch_shape_sources(
    *,
    team: Team,
    window_start: datetime,
    window_end: datetime,
    query_hashes: set[int],
    where: str,
    parameters: dict[str, Any],
) -> list[ShapeSourceRow]:
    if not query_hashes:
        return []
    with tags_context(product=Product.DATA_CATALOG, feature=Feature.ENRICHMENT, team_id=team.id):
        rows = sync_execute(
            f"""
            SELECT
                normalized_query_hash,
                lc_insight_id,
                lc_dashboard_id,
                lc_product,
                groupUniqArray(200)(lc_user_id) AS user_ids,
                count() AS runs
            FROM query_log_archive
            WHERE {where} AND normalized_query_hash IN %(query_hashes)s
            GROUP BY normalized_query_hash, lc_insight_id, lc_dashboard_id, lc_product
            """,
            {
                **parameters,
                "team_id": team.id,
                "window_start": window_start,
                "window_end": window_end,
                "query_hashes": sorted(query_hashes),
            },
            team_id=team.id,
            workload=Workload.OFFLINE,
        )
    return [
        ShapeSourceRow(
            query_hash=query_hash,
            insight_id=insight_id,
            dashboard_id=dashboard_id,
            product=product,
            user_ids=frozenset(user_ids),
            runs=runs,
        )
        for query_hash, insight_id, dashboard_id, product, user_ids, runs in rows
    ]


def fetch_chart_names(*, team: Team, insight_ids: set[int]) -> dict[int, ChartName]:
    """Names of the team's live charts and their live dashboards. A deleted chart gets no name, so it is not shown."""
    if not insight_ids:
        return {}
    rows = Insight.objects.filter(team=team, id__in=insight_ids, deleted=False).values_list(
        "id", "short_id", "name", "derived_name", "dashboards__id", "dashboards__name", "dashboards__deleted"
    )
    charts: dict[int, tuple[str, str]] = {}
    dashboards: dict[int, dict[int, str]] = defaultdict(dict)
    for insight_id, short_id, name, derived_name, dashboard_id, dashboard_name, dashboard_deleted in rows:
        charts[insight_id] = (short_id, name or derived_name or "Unnamed chart")
        if dashboard_id and not dashboard_deleted:
            dashboards[insight_id][dashboard_id] = dashboard_name or "Unnamed dashboard"
    return {
        insight_id: ChartName(short_id=short_id, name=name, dashboards=dashboards[insight_id])
        for insight_id, (short_id, name) in charts.items()
    }


def _chart_sources(
    rows: Iterable[ShapeSourceRow], names: Mapping[int, ChartName], people_of: Mapping[int, str]
) -> list[ChartSource]:
    runs: dict[tuple[int, int], int] = defaultdict(int)
    people: dict[tuple[int, int], set[str]] = defaultdict(set)
    for row in rows:
        if row.insight_id in names:
            key = (row.insight_id, row.dashboard_id)
            runs[key] += row.runs
            people[key].update(people_of[user_id] for user_id in row.user_ids if user_id in people_of)
    sources = []
    for (insight_id, dashboard_id), chart_runs in runs.items():
        chart = names[insight_id]
        dashboard_name = chart.dashboards.get(dashboard_id)
        sources.append(
            ChartSource(
                insight_short_id=chart.short_id,
                insight_name=chart.name,
                # A dashboard that no longer holds the chart has no name to show or page to open.
                dashboard_id=dashboard_id if dashboard_name else None,
                dashboard_name=dashboard_name,
                people=len(people[(insight_id, dashboard_id)]),
                runs=chart_runs,
            )
        )
    sources.sort(key=lambda source: (-source.people, -source.runs, source.insight_name, source.dashboard_id or 0))
    return sources


def _surface_sources(rows: Iterable[ShapeSourceRow], people_of: Mapping[int, str]) -> list[SurfaceSource]:
    runs: dict[str, int] = defaultdict(int)
    people: dict[str, set[str]] = defaultdict(set)
    for row in rows:
        if not row.insight_id:
            runs[row.product] += row.runs
            people[row.product].update(people_of[user_id] for user_id in row.user_ids if user_id in people_of)
    sources = [
        SurfaceSource(product=product, people=len(people[product]), runs=surface_runs)
        for product, surface_runs in runs.items()
    ]
    sources.sort(key=lambda source: (-source.people, -source.runs, source.product))
    return sources


def build_proposed_metrics(
    drafted: Iterable[UncoveredGroup],
    evidence: Mapping[str, MeasureEvidence],
    rows: Iterable[ShapeSourceRow],
    names: Mapping[int, ChartName],
    people_of: Mapping[int, str],
) -> list[ProposedMetric]:
    """One proposal per draft name, ranked by people. Groups that share a draft are one measure written several ways."""
    by_name: dict[str, list[UncoveredGroup]] = defaultdict(list)
    for group in drafted:
        if group.draft_name:
            by_name[group.draft_name].append(group)
    rows_by_hash: dict[int, list[ShapeSourceRow]] = defaultdict(list)
    for row in rows:
        rows_by_hash[row.query_hash].append(row)

    proposals = []
    for name, groups in by_name.items():
        items = [evidence[group.fingerprint_digest] for group in groups]
        # A query that computes several measures of one proposal still ran once, so its rows count once.
        shape_hashes = frozenset().union(*(item.shape_hashes for item in items))
        shape_rows = [row for query_hash in shape_hashes for row in rows_by_hash[query_hash]]
        charts = _chart_sources(shape_rows, names, people_of)
        agent_runs = max(item.agent_runs for item in items)
        proposals.append(
            ProposedMetric(
                name=name,
                description=groups[0].draft_description or "",
                definition=groups[0].draft_definition or "",
                measures=len(groups),
                people=len(frozenset().union(*(item.people for item in items))),
                active_days=len(frozenset().union(*(item.days for item in items))),
                runs=max(group.runs for group in groups),
                charts=charts[:MAX_CHART_SOURCES],
                other_charts=max(0, len(charts) - MAX_CHART_SOURCES),
                surfaces=_surface_sources(shape_rows, people_of),
                agents=(
                    AgentSource(
                        people=len(frozenset().union(*(item.agent_people for item in items))),
                        sessions=len(frozenset().union(*(item.agent_sessions for item in items))),
                        runs=agent_runs,
                    )
                    if agent_runs
                    else None
                ),
            )
        )
    proposals.sort(key=lambda proposal: (-proposal.people, -proposal.runs, proposal.name))
    return proposals
