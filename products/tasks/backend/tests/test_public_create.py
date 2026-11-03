from typing import Any

from django.test import SimpleTestCase

from parameterized import parameterized

from products.tasks.backend.presentation.serializers import TaskRunOptionsRequestSerializer
from products.tasks.backend.public_create import (
    create_fields_from_options,
    options_from_request,
    public_idempotency_key,
    request_fingerprint,
)

OPTION_SAMPLES: dict[str, tuple[Any, dict[str, Any]]] = {
    "runtime": ("pi", {"runtime": "pi"}),
    "adapter": ("codex", {"runtime_adapter": "codex"}),
    "model": ("claude-sonnet-5", {"model": "claude-sonnet-5"}),
    "reasoning_effort": ("high", {"reasoning_effort": "high"}),
    "repositories": (
        [{"name": "posthog/posthog", "initial_branch": "main"}],
        {"repositories": ["posthog/posthog"], "branch": "main"},
    ),
    "output_schema": ({"type": "object"}, {"json_schema": {"type": "object"}}),
    "permission_mode": ("plan", {"initial_permission_mode": "plan"}),
    "auto_publish": (True, {"auto_publish": True}),
}


class TestPublicCreateOptions(SimpleTestCase):
    def test_every_option_the_endpoint_accepts_has_a_sample(self) -> None:
        self.assertEqual(set(TaskRunOptionsRequestSerializer().fields), set(OPTION_SAMPLES))

    @parameterized.expand([(name, value, expected) for name, (value, expected) in OPTION_SAMPLES.items()])
    def test_each_option_reaches_the_create_fields(self, name: str, value: Any, expected: dict[str, Any]) -> None:
        options = options_from_request({name: value})

        self.assertEqual(create_fields_from_options(options), expected)

    @parameterized.expand(
        [
            ("without_a_branch", [{"name": "posthog/posthog"}], {"repositories": ["posthog/posthog"]}),
            (
                "branch_of_the_first_repository",
                [
                    {"name": "posthog/posthog", "initial_branch": "main"},
                    {"name": "posthog/code", "initial_branch": "dev"},
                ],
                {"repositories": ["posthog/posthog", "posthog/code"], "branch": "main"},
            ),
            (
                "branch_of_a_later_repository_only",
                [{"name": "posthog/posthog"}, {"name": "posthog/code", "initial_branch": "dev"}],
                {"repositories": ["posthog/posthog", "posthog/code"]},
            ),
        ]
    )
    def test_repositories_start_the_first_run_from_the_first_branch(
        self, _name: str, repositories: list[dict[str, str]], expected: dict[str, Any]
    ) -> None:
        options = options_from_request({"repositories": repositories})

        self.assertEqual(create_fields_from_options(options), expected)

    def test_options_left_unset_create_no_fields(self) -> None:
        self.assertEqual(create_fields_from_options(options_from_request({})), {})


class TestPublicCreateIdempotency(SimpleTestCase):
    @parameterized.expand(
        [
            ("same_body", {"title": "a", "options": {"model": "m"}}, {"title": "a", "options": {"model": "m"}}, True),
            (
                "key_order",
                {"title": "a", "description": "b"},
                {"description": "b", "title": "a"},
                True,
            ),
            ("key_ignored", {"title": "a", "idempotency_key": "one"}, {"title": "a", "idempotency_key": "two"}, True),
            ("top_level_field", {"title": "a"}, {"title": "b"}, False),
            ("nested_field", {"options": {"model": "m"}}, {"options": {"model": "n"}}, False),
            ("extra_field", {"title": "a"}, {"title": "a", "description": "b"}, False),
        ]
    )
    def test_request_fingerprint_follows_the_body(
        self, _name: str, first: dict[str, Any], second: dict[str, Any], same: bool
    ) -> None:
        self.assertEqual(request_fingerprint(first) == request_fingerprint(second), same)

    @parameterized.expand(
        [
            ("same_user_same_key", 1, "k", 1, "k", True),
            ("other_user", 1, "k", 2, "k", False),
            ("other_key", 1, "k", 1, "j", False),
        ]
    )
    def test_public_keys_are_per_user(
        self, _name: str, first_user: int, first_key: str, second_user: int, second_key: str, same: bool
    ) -> None:
        self.assertEqual(
            public_idempotency_key(first_user, first_key) == public_idempotency_key(second_user, second_key), same
        )
