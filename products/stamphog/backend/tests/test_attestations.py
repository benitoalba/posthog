import pytest
from unittest.mock import MagicMock

from products.stamphog.backend.facade.contracts import StamphogGitHubError
from products.stamphog.backend.logic.attestations import (
    ANNOTATION_TITLE_PREFIX,
    ActionsAttestation,
    actions_attestation_results,
    actions_attestations,
)

_WORKFLOW = ".github/workflows/dependency-proof.yml"
_RULE = ActionsAttestation(name="Dependency proof", workflow=_WORKFLOW, job="prove")
_PR = {"number": 7, "head": {"sha": "head1"}, "base": {"ref": "master"}}
_MARKER = '<!-- stamphog:v1 ["pnpm-lock.yaml"] -->'


def _run(run_id: int, created_at: str, **changes: object) -> dict:
    return {
        "id": run_id,
        "created_at": created_at,
        "path": _WORKFLOW,
        "event": "pull_request",
        "head_sha": "head1",
        "status": "completed",
        "pull_requests": [{"number": 7}],
        **changes,
    }


def _annotation(title: str = ANNOTATION_TITLE_PREFIX + "master", level: str = "notice") -> dict:
    return {"annotation_level": level, "title": title, "message": _MARKER}


class _FakeClient:
    def __init__(self, runs: list[dict], jobs: dict[int, list[dict]], annotations: dict[int, list[dict]]) -> None:
        self.runs, self.jobs, self.annotations = runs, jobs, annotations

    def get_pull_request_workflow_runs(self, repo: str, head_sha: str) -> list[dict]:
        return self.runs

    def get_workflow_run_jobs(self, repo: str, run_id: int) -> list[dict]:
        return self.jobs[run_id]

    def get_check_run_annotations(self, repo: str, check_run_id: int) -> list[dict]:
        return self.annotations.get(check_run_id, [])


_PASSED = {"completed": True, "conclusion": "success", "files": ["pnpm-lock.yaml"]}
_NO_RUN = {"completed": False}


@pytest.mark.parametrize(
    "runs, jobs, annotations, expected",
    [
        pytest.param(
            [_run(1, "t1")],
            {1: [{"id": 11, "name": "prove", "conclusion": "success"}]},
            {11: [_annotation()]},
            _PASSED,
            id="passed",
        ),
        pytest.param([_run(1, "t1", path=".github/workflows/other.yml")], {}, {}, _NO_RUN, id="other-workflow"),
        pytest.param([_run(1, "t1", event="push")], {}, {}, _NO_RUN, id="other-event"),
        pytest.param([_run(1, "t1", head_sha="head0")], {}, {}, _NO_RUN, id="other-head"),
        pytest.param([_run(1, "t1", pull_requests=[{"number": 8}])], {}, {}, _NO_RUN, id="other-pr"),
        pytest.param([_run(1, "t1", status="in_progress")], {}, {}, _NO_RUN, id="running"),
        pytest.param(
            [_run(1, "t1"), _run(2, "t2")],
            {
                1: [{"id": 11, "name": "prove", "conclusion": "success"}],
                2: [{"id": 21, "name": "prove", "conclusion": "skipped"}],
            },
            {11: [_annotation()]},
            _PASSED,
            id="skipped-newer-run-keeps-older-proof",
        ),
        pytest.param(
            [_run(1, "t1"), _run(2, "t2")],
            {
                1: [{"id": 11, "name": "prove", "conclusion": "success"}],
                2: [{"id": 21, "name": "prove", "conclusion": "failure"}],
            },
            {11: [_annotation()]},
            {"completed": True, "conclusion": "failure"},
            id="newest-run-failed",
        ),
        pytest.param(
            [_run(1, "t1")],
            {1: [{"id": 11, "name": "prove", "conclusion": "success"}]},
            {11: [_annotation(title=ANNOTATION_TITLE_PREFIX + "release")]},
            {"completed": True, "conclusion": "success", "files": []},
            id="proved-against-another-base",
        ),
        pytest.param(
            [_run(1, "t1")],
            {1: [{"id": 11, "name": "prove", "conclusion": "success"}]},
            {11: [_annotation(level="warning")]},
            {"completed": True, "conclusion": "success", "files": []},
            id="not-a-notice",
        ),
        pytest.param(
            [_run(1, "t1")],
            {1: [{"id": 11, "name": "prove", "conclusion": "success"}]},
            {11: [_annotation(), _annotation()]},
            {"completed": True, "conclusion": "success", "files": []},
            id="injected-second-marker",
        ),
        pytest.param(
            [_run(1, "t1")],
            {1: [{"id": 11, "name": "build", "conclusion": "success"}]},
            {},
            {"completed": True, "conclusion": "missing_job"},
            id="job-not-found",
        ),
    ],
)
def test_actions_attestation_trusts_only_the_declared_run(
    runs: list[dict], jobs: dict[int, list[dict]], annotations: dict[int, list[dict]], expected: dict
) -> None:
    client = _FakeClient(runs, jobs, annotations)

    assert actions_attestation_results(client, "posthog/posthog", _PR, [_RULE]) == {"Dependency proof": expected}  # type: ignore[arg-type]


def test_actions_attestation_without_actions_read_lifts_and_waits_for_nothing() -> None:
    client = _FakeClient([], {}, {})
    client.get_pull_request_workflow_runs = MagicMock(side_effect=StamphogGitHubError("forbidden", status_code=403))  # type: ignore[method-assign]

    assert actions_attestation_results(client, "posthog/posthog", _PR, [_RULE]) == {  # type: ignore[arg-type]
        "Dependency proof": {"completed": True, "conclusion": "forbidden"}
    }


def test_actions_attestations_reads_only_actions_entries_from_both_lists() -> None:
    policy = """
attestations:
  - {name: Migration risk, check: Migration risk, lifts: [migrations], paths: ['x']}
extend_attestations:
  - {name: Dependency proof, actions_workflow: .github/workflows/dependency-proof.yml, job: prove, lifts: [deps_toolchain], paths: ['y']}
"""

    assert actions_attestations(policy) == [_RULE]
