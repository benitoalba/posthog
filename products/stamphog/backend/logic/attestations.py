"""Find the GitHub Actions runs that the repository's policy trusts as attestations.

An attestation is a CI result that lifts named deny categories for the files it lists (the engine's
``attestations.py`` holds the model). The engine reads a check attestation from the head commit's
check runs. An Actions attestation needs its run found first, with a token the sandbox does not
hold, so the server finds it here and passes the result in the review context.

A run counts only when GitHub says it ran the declared workflow file on a ``pull_request`` event
for this head commit and this PR, and only the named job of its latest attempt is read. A check
with the right name from anywhere else, or a job that skipped, counts for nothing.

The job lists its files in a notice annotation titled with the base branch it proved against, so a
proof made before a base retarget lifts nothing. A run's ``pull_requests`` field cannot answer this:
GitHub fills it at read time from the PR's current state.
"""

import re
import json

import structlog

from posthog.dataclasses import frozen

from ..facade.contracts import StamphogGitHubError
from .github_client import StamphogGitHubClient
from .repo_policy import policy_list

logger = structlog.get_logger(__name__)

ANNOTATION_TITLE_PREFIX = "stamphog attestation: "
_MARKER_RE = re.compile(r"<!--\s*stamphog:v1\s+(\[[^\]]*\])\s*-->")


@frozen
class ActionsAttestation:
    name: str
    workflow: str
    job: str


def actions_attestations(policy_text: str | None) -> list[ActionsAttestation]:
    """The Actions attestations a repository's default-branch policy declares."""
    return [
        ActionsAttestation(name=entry["name"], workflow=entry["actions_workflow"], job=entry["job"])
        for entry in policy_list(policy_text, "attestations")
        if all(isinstance(entry.get(key), str) for key in ("name", "actions_workflow", "job"))
    ]


def _marker_files(annotations: list[dict], base_ref: str) -> list[str]:
    # A job that prints PR content can emit a workflow command too, so a second marker means the
    # job's output was not its own, and the proof lifts nothing.
    markers = [
        match.group(1)
        for annotation in annotations
        if annotation.get("annotation_level") == "notice"
        and annotation.get("title") == ANNOTATION_TITLE_PREFIX + base_ref
        for match in _MARKER_RE.finditer(str(annotation.get("message") or ""))
    ]
    if len(markers) != 1:
        return []
    try:
        paths = json.loads(markers[0])
    except json.JSONDecodeError:
        return []
    return sorted({p for p in paths if isinstance(p, str)}) if isinstance(paths, list) else []


def _result(client: StamphogGitHubClient, repo: str, pr: dict, rule: ActionsAttestation, runs: list[dict]) -> dict:
    head_sha = (pr.get("head") or {}).get("sha")
    candidates = sorted(
        (
            run
            for run in runs
            if run.get("path") == rule.workflow
            and run.get("event") == "pull_request"
            and run.get("head_sha") == head_sha
            and any(item.get("number") == pr.get("number") for item in run.get("pull_requests") or [])
        ),
        key=lambda run: (run.get("created_at") or "", run.get("id") or 0),
        reverse=True,
    )
    for run in candidates:
        if run.get("status") != "completed":
            return {"completed": False}
        jobs = [job for job in client.get_workflow_run_jobs(repo, run["id"]) if job.get("name") == rule.job]
        if len(jobs) != 1:
            return {"completed": True, "conclusion": "missing_job"}
        job = jobs[0]
        # A run that skipped the job (an edit that changed nothing it proves) leaves the older proof standing.
        if job.get("conclusion") == "skipped":
            continue
        if job.get("conclusion") != "success":
            return {"completed": True, "conclusion": job.get("conclusion") or "unknown"}
        base_ref = (pr.get("base") or {}).get("ref") or ""
        files = _marker_files(client.get_check_run_annotations(repo, job["id"]), base_ref)
        return {"completed": True, "conclusion": "success", "files": files}
    return {"completed": False}


def actions_attestation_results(
    client: StamphogGitHubClient, repo: str, pr: dict, rules: list[ActionsAttestation]
) -> dict[str, dict]:
    """Each Actions attestation's state at the PR head, keyed by name, in the engine's context shape.

    No run yet reads as not completed, so the engine waits for it when the PR changes a file the
    attestation judges. An installation without Actions read gets a 403, which lifts nothing and
    waits for nothing, so the review still runs. Other errors raise and the activity retries.
    """
    try:
        runs = client.get_pull_request_workflow_runs(repo, (pr.get("head") or {}).get("sha") or "")
        return {rule.name: _result(client, repo, pr, rule, runs) for rule in rules}
    except StamphogGitHubError as e:
        if e.status_code != 403:
            raise
        logger.warning("stamphog_actions_attestations_forbidden", repository=repo)
        return {rule.name: {"completed": True, "conclusion": "forbidden"} for rule in rules}
