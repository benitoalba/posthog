from datetime import UTC, datetime, timedelta
from uuid import UUID

from posthog.test.base import BaseTest, ClickhouseTestMixin

from django.test import SimpleTestCase

from parameterized import parameterized

from posthog.clickhouse.client import sync_execute
from posthog.clickhouse.workload import Workload

from products.data_catalog.backend.facade.enums import MetricStatus, ShapeOutcome
from products.data_catalog.backend.logic.measure_fingerprint import extract_measures
from products.data_catalog.backend.logic.metric_fingerprint import CatalogIndex, CatalogMeasure
from products.data_catalog.backend.logic.usage_adjusted import AgentQuery
from products.data_catalog.backend.logic.usage_uncovered import (
    DraftFailed,
    HumanShape,
    MeasureFolder,
    ProposalDraft,
    apply_drafts,
    fetch_human_shapes,
    is_bare_measure,
    parse_draft_reply,
)

SIGNUPS = "SELECT count() FROM events WHERE event = 'signup'"
REVENUE = "SELECT sum(properties.amount) FROM events WHERE event = 'purchase'"
ERRORS = "SELECT count() FROM events WHERE event = '$exception'"
SIGNUPS_ID = UUID("00000000-0000-0000-0000-000000000001")
AT = datetime(2026, 10, 20, 12, 0, tzinfo=UTC)


def _agent(sql: str, distinct_id: str, *, is_scout: bool = False) -> AgentQuery:
    return AgentQuery(occurred_at=AT, trace_id="t", distinct_id=distinct_id, intent="", sql=sql, is_scout=is_scout)


def _index() -> CatalogIndex:
    return CatalogIndex(
        CatalogMeasure(metric_id=SIGNUPS_ID, status=MetricStatus.APPROVED, created_at=AT, fingerprint=fingerprint)
        for fingerprint in extract_measures(SIGNUPS)
    )


class TestMeasureFolder(SimpleTestCase):
    def test_people_union_across_surfaces_and_scouts_count_apart(self) -> None:
        folder = MeasureFolder()
        folder.add_human(
            HumanShape(
                query_hash=1, sample_sql=REVENUE, user_ids=frozenset({1}), active_days=frozenset({"2026-10-19"}), runs=4
            ),
            {1: "alice"},
        )
        folder.add_agent(_agent(REVENUE, "alice"))
        folder.add_agent(_agent(REVENUE, "bob"))
        folder.add_agent(_agent(REVENUE, "scout", is_scout=True))
        folder.add_agent(_agent(SIGNUPS, "alice"))
        folder.add_agent(_agent(SIGNUPS, "bob"))
        folder.add_agent(_agent(ERRORS, "scout", is_scout=True))
        folder.add_agent(_agent("SELECT uniq(person_id) FROM events", "alice"))

        groups = folder.groups(_index())

        assert [(group.outcome, group.distinct_people, group.runs, group.scout_runs) for group in groups] == [
            (ShapeOutcome.UNCOVERED, 2, 6, 1),
            (ShapeOutcome.COVERED, 2, 2, 0),
            (ShapeOutcome.UNCOVERED, 0, 0, 1),
        ]
        assert groups[1].matched_metric_id == SIGNUPS_ID

    def test_evidence_keeps_web_queries_and_agent_sessions_apart_from_scouts(self) -> None:
        folder = MeasureFolder()
        folder.add_human(
            HumanShape(query_hash=42, sample_sql=REVENUE, user_ids=frozenset({1}), active_days=frozenset(), runs=2),
            {1: "alice"},
        )
        folder.add_agent(_agent(REVENUE, "bob"))
        folder.add_agent(_agent(REVENUE, "scout", is_scout=True))

        evidence = folder.evidence(extract_measures(REVENUE)[0].digest)

        assert evidence.shape_hashes == frozenset({42})
        assert evidence.people == frozenset({"alice", "bob"})
        assert evidence.agent_people == frozenset({"bob"})
        assert evidence.agent_sessions == frozenset({"t"})
        assert evidence.agent_runs == 1

    def test_scout_only_groups_keep_their_share_of_the_cap(self) -> None:
        folder = MeasureFolder()
        for sql in (REVENUE, SIGNUPS):
            folder.add_agent(_agent(sql, "alice"))
            folder.add_agent(_agent(sql, "bob"))
        folder.add_agent(_agent(ERRORS, "scout", is_scout=True))
        folder.add_agent(_agent("SELECT sum(amount) FROM invoices", "scout", is_scout=True))
        folder.add_agent(_agent("SELECT sum(amount) FROM invoices", "scout", is_scout=True))

        groups = folder.groups(_index(), max_groups=2, scout_groups=1)

        assert [(group.distinct_people, group.scout_runs) for group in groups] == [(2, 0), (0, 2)]


class TestBareMeasures(SimpleTestCase):
    @parameterized.expand(
        [
            ("count_of_a_table", "SELECT count() FROM events", True),
            ("count_star_is_the_same_measure", "SELECT count(*) FROM persons", True),
            ("unique_people", "SELECT uniq(person_id) FROM events", True),
            ("newest_row", "SELECT max(timestamp) FROM events", True),
            ("any_value", "SELECT any(key) FROM groups", True),
            ("dashboard_placeholder_only", "SELECT count() FROM events WHERE '{filters}'", True),
            ("team_scope_only", "SELECT count() FROM events WHERE team_id = 2", True),
            ("test_account_exclusion_only", "SELECT count() FROM persons WHERE email NOT ILIKE '%@example.com'", True),
            ("non_empty_check_only", "SELECT count() FROM orgs WHERE org_id != ''", True),
            ("single_event_count", "SELECT count() FROM events WHERE event = 'user signed up'", False),
            ("count_with_a_business_filter_on_email", "SELECT count() FROM orgs WHERE email_type = 'work'", False),
            ("placeholder_beside_an_event", "SELECT count() FROM events WHERE '{filters}' AND event = 'x'", False),
            ("conditional_count", "SELECT countIf(event = 'purchase') FROM events", False),
            ("sum_of_a_value", "SELECT sum(amount) FROM invoices", False),
            ("largest_value", "SELECT max(amount) FROM invoices", False),
            ("count_of_a_column", "SELECT count(paid_at_epoch) FROM invoices", False),
        ]
    )
    def test_is_bare_measure(self, _name: str, sql: str, expected: bool) -> None:
        assert is_bare_measure(extract_measures(sql)[0]) is expected

    def test_folder_drops_a_bare_measure_unless_the_catalog_covers_it(self) -> None:
        covered = "SELECT count() FROM persons"
        index = CatalogIndex(
            CatalogMeasure(metric_id=SIGNUPS_ID, status=MetricStatus.APPROVED, created_at=AT, fingerprint=fingerprint)
            for fingerprint in extract_measures(covered)
        )
        folder = MeasureFolder()
        for sql in ("SELECT count() FROM events", covered, SIGNUPS):
            folder.add_agent(_agent(sql, "alice"))
            folder.add_agent(_agent(sql, "bob"))
        folder.add_agent(_agent("SELECT max(timestamp) FROM events", "scout", is_scout=True))

        groups = folder.groups(index)

        assert {group.measure: group.outcome for group in groups} == {
            "count() on persons": ShapeOutcome.COVERED,
            "count() on events where event = 'signup'": ShapeOutcome.UNCOVERED,
        }


class TestProposalDrafts(SimpleTestCase):
    @parameterized.expand(
        [
            ("noise", '{"keep": false}', None),
            ("kept_without_a_name", '{"keep": true, "name": ""}', None),
            (
                "kept",
                '{"keep": true, "name": "revenue", "description": "d", "definition": "q", "merge_key": "Total Revenue"}',
                ProposalDraft(name="revenue", description="d", definition="q", merge_key="total revenue"),
            ),
        ]
    )
    def test_parse_draft_reply(self, _name: str, reply: str, expected: ProposalDraft | None) -> None:
        assert parse_draft_reply(reply) == expected

    @parameterized.expand(
        [("not_json", "Keep it."), ("cut_off", '{"keep": true, "name": "revenue", "definition": "SELECT')]
    )
    def test_an_unreadable_reply_fails_instead_of_reading_as_noise(self, _name: str, reply: str) -> None:
        with self.assertRaises(DraftFailed):
            parse_draft_reply(reply)

    def test_noise_drops_and_a_shared_merge_key_shares_one_draft(self) -> None:
        folder = MeasureFolder()
        for sql in (
            REVENUE,
            "SELECT sum(properties.amount) FROM events WHERE event = 'purchase_completed'",
            ERRORS,
            SIGNUPS,
        ):
            folder.add_agent(_agent(sql, "alice"))
            folder.add_agent(_agent(sql, "bob"))
        revenue, revenue_again, errors, signups = (
            {group.sample_sql: group for group in folder.groups(_index())}[sql]
            for sql in (
                REVENUE,
                "SELECT sum(properties.amount) FROM events WHERE event = 'purchase_completed'",
                ERRORS,
                SIGNUPS,
            )
        )
        first = ProposalDraft(name="revenue", description="d", definition="q", merge_key="revenue")
        second = ProposalDraft(name="purchase_revenue", description="d2", definition="q2", merge_key="revenue")

        result = apply_drafts(
            [revenue, revenue_again, errors, signups],
            {
                revenue.fingerprint_digest: first,
                revenue_again.fingerprint_digest: second,
                errors.fingerprint_digest: None,
            },
        )

        assert [(group.sample_sql, group.draft_name) for group in result] == [
            (REVENUE, "revenue"),
            ("SELECT sum(properties.amount) FROM events WHERE event = 'purchase_completed'", "revenue"),
            (SIGNUPS, None),
        ]


class TestFetchHumanShapes(ClickhouseTestMixin, BaseTest):
    def test_keeps_only_queries_people_write_in_the_app(self) -> None:
        sources = [
            ("product_analytics", "insight", ""),
            ("product_analytics", "dashboard", ""),
            ("sql_editor", "query", ""),
            ("notebooks", "query", ""),
            ("canvas", "query", ""),
            ("warehouse", "query", ""),
            ("today", "query", ""),
            ("", "query", ""),
            ("product_analytics", "event_definition_scene", ""),
            ("mcp_analytics", "query", ""),
            ("max_ai", "management_command", ""),
            ("product_analytics", "insight", "api"),
        ]
        sync_execute(
            """INSERT INTO sharded_query_log_archive_v3
            (initial_query_start_time, initial_query_id, event_time_microseconds, team_id, type, is_initial_query,
             normalized_query_hash, lc_query__query, lc_user_id, lc_access_method, lc_product, lc_feature) VALUES""",
            [
                (
                    AT,
                    f"q{hash_}",
                    AT,
                    self.team.id,
                    "QueryFinish",
                    1,
                    hash_,
                    f"SELECT {hash_}",
                    7,
                    method,
                    product,
                    feature,
                )
                for hash_, (product, feature, method) in enumerate(sources, start=1)
            ],
            workload=Workload.OFFLINE,
        )

        shapes = fetch_human_shapes(
            team=self.team, window_start=AT - timedelta(days=1), window_end=AT + timedelta(days=1)
        )

        assert sorted(shape.sample_sql for shape in shapes) == [f"SELECT {hash_}" for hash_ in range(1, 9)]
