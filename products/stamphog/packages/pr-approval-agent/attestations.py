"""CI results that lift named deny categories for the files they list.

An attestation is a policy entry (see `policy.Attestation`). It names a CI result, the deny
categories it may lift, and `paths`, the files it is expected to judge. A result lifts its
categories only for the files it lists itself, intersected with `paths` and the PR's files.
The engine knows no ecosystem, tool or bot: what a passing result proves is the publisher's
business.

A result lists its files in a `<!-- stamphog:v1 [...] -->` marker: in the check's summary for a
check entry, or in a notice annotation of the job for an Actions entry. Actions jobs cannot
publish a check summary, and their step summary is not readable through the API.

A check entry is read from the head commit's check runs, which GitHub binds to that commit. An
Actions entry needs the run looked up by workflow path, event, PR and base, which only the hosted
server can do, so it arrives as `PRData.attestation_results`. An Actions entry without a result
there was not looked up (a local run) and lifts nothing.
"""

import re
import json
from collections.abc import Iterable, Mapping
from dataclasses import dataclass

from policy import Attestation

_MARKER_RE = re.compile(r"<!--\s*stamphog:v1\s+(\[[^\]]*\])\s*-->")


@dataclass(frozen=True)
class AttestationResult:
    """One attestation's state at the reviewed head. `files` is empty unless it passed."""

    completed: bool
    files: frozenset[str] = frozenset()


def marker_files(text: str) -> frozenset[str]:
    """The paths in the one `stamphog:v1` marker in ``text``. Anything malformed lists nothing.

    A publisher can quote PR content next to its marker, so a second marker means the text is not
    only the publisher's, and it lists nothing.
    """
    markers = _MARKER_RE.findall(text)
    if len(markers) != 1:
        return frozenset()
    try:
        paths = json.loads(markers[0])
    except json.JSONDecodeError:
        return frozenset()
    return frozenset(p for p in paths if isinstance(p, str)) if isinstance(paths, list) else frozenset()


def _check_result(rule: Attestation, check_runs: Iterable[dict]) -> AttestationResult | None:
    completed = [
        run
        for run in check_runs
        if run.get("name") == rule.check
        and run.get("status") == "completed"
        and (rule.app is None or (run.get("app") or {}).get("slug") == rule.app)
    ]
    if not completed:
        return None
    latest = max(completed, key=lambda run: run.get("completed_at") or "")
    if latest.get("conclusion") != "success":
        return AttestationResult(completed=True)
    return AttestationResult(completed=True, files=marker_files((latest.get("output") or {}).get("summary") or ""))


def _actions_result(rule: Attestation, results: Mapping[str, dict]) -> AttestationResult | None:
    raw = results.get(rule.name)
    if not isinstance(raw, dict):
        return AttestationResult(completed=True)
    if not raw.get("completed"):
        return None
    files = raw.get("files") if raw.get("conclusion") == "success" else None
    return AttestationResult(completed=True, files=frozenset(f for f in files or () if isinstance(f, str)))


def resolve_results(
    rules: Iterable[Attestation],
    check_runs: Iterable[dict],
    actions_results: Mapping[str, dict],
    pr_files: Iterable[str],
) -> dict[str, AttestationResult | None]:
    """Each rule's result by name. None means it has not reported for this head yet.

    A ``pull_request`` run executes the PR's own copy of its workflow, so an Actions result for a PR
    that edits that workflow lifts nothing and waits for nothing.
    """
    check_runs = list(check_runs)
    edited = set(pr_files)
    return {
        rule.name: _check_result(rule, check_runs)
        if rule.check
        else AttestationResult(completed=True)
        if rule.actions_workflow in edited
        else _actions_result(rule, actions_results)
        for rule in rules
    }


def _expected_files(rule: Attestation, pr_files: Iterable[str]) -> set[str]:
    return {path for path in pr_files if any(re.search(pattern, path) for pattern in rule.paths)}


def lifted_files(
    rules: Iterable[Attestation], results: Mapping[str, AttestationResult | None], pr_files: Iterable[str]
) -> dict[str, frozenset[str]]:
    """Per deny category, the PR files a passing attestation listed and may lift it for."""
    pr_files = list(pr_files)
    lifted: dict[str, set[str]] = {}
    for rule in rules:
        result = results.get(rule.name)
        if result is None or not result.files:
            continue
        covered = _expected_files(rule, pr_files) & result.files
        for category in rule.lifts:
            lifted.setdefault(category, set()).update(covered)
    return {category: frozenset(files) for category, files in lifted.items() if files}


def assume_passing(
    assumed: Iterable[Attestation], results: Mapping[str, AttestationResult | None], pr_files: Iterable[str]
) -> dict[str, AttestationResult | None]:
    """``results`` with each ``assumed`` attestation passing for every PR file it judges."""
    pr_files = list(pr_files)
    return {
        **results,
        **{
            rule.name: AttestationResult(completed=True, files=frozenset(_expected_files(rule, pr_files)))
            for rule in assumed
        },
    }


def pending(
    rules: Iterable[Attestation], results: Mapping[str, AttestationResult | None], pr_files: Iterable[str]
) -> list[Attestation]:
    """The attestations that have not reported yet although the PR changes a file they judge."""
    pr_files = list(pr_files)
    return [rule for rule in rules if results.get(rule.name) is None and _expected_files(rule, pr_files)]
