from datetime import timedelta
from typing import Any
from uuid import UUID

import time_machine
from posthog.test.base import APIBaseTest, _create_person, flush_persons_and_events
from unittest.mock import patch

from django.core.files.uploadedfile import SimpleUploadedFile
from django.test import SimpleTestCase
from django.utils import timezone

from parameterized import parameterized
from rest_framework import status

from posthog.api.capture import CaptureInternalResult
from posthog.models import PersonalAPIKey
from posthog.models.integration import Integration
from posthog.models.organization import OrganizationMembership
from posthog.models.team import Team
from posthog.models.utils import generate_random_token_personal, hash_key_value

from products.workflows.backend.facade.contracts import PushTokenImportInvalid
from products.workflows.backend.models.push_token_import import PushTokenImport
from products.workflows.backend.services import push_token_import
from products.workflows.backend.services.push_token_import import (
    check_stalled_push_token_import,
    parse_push_token_csv,
    run_push_token_import_batches,
)

APNS_TOKEN = "a1" * 32


class _MemoryStorage:
    def __init__(self) -> None:
        self.objects: dict[str, str] = {}

    def write(self, key: str, content: str) -> None:
        self.objects[key] = content

    def read(self, key: str, *, missing_ok: bool = False) -> str | None:
        return self.objects.get(key)

    def delete(self, key: str) -> None:
        self.objects.pop(key, None)


class TestParsePushTokenCsv(SimpleTestCase):
    @parameterized.expand(
        [
            ("not_utf8", "distinct_id,device_token\nuser-é,abc\n".encode("latin-1"), "isn't saved as UTF-8"),
            ("missing_column", b"distinct_id,token\nuser-1,abc\n", "needs a distinct_id column"),
            ("only_header", b"distinct_id,device_token\n\n , \n", "only the column names"),
            ("empty", b"", "The file is empty"),
            ("broken_quotes", b'distinct_id,device_token\n"user-1,abc\n', "can't be read"),
            ("too_many_lines", b"distinct_id,device_token\nuser-1,abc" + b"\n" * 400_001, "more than 400,000 lines"),
        ]
    )
    def test_rejects_a_file_it_cannot_use(self, _name: str, content: bytes, message: str) -> None:
        with self.assertRaisesMessage(PushTokenImportInvalid, message):
            parse_push_token_csv(content)

    def test_reads_devices_with_the_line_each_came_from(self) -> None:
        content = (
            "﻿Name,Distinct_ID , DEVICE_TOKEN\n"
            '"Ada\nLovelace",user-1,tok-1\n'
            "\n"
            "Bob,user-2,tok-2\n"
            "Bob again,user-2,tok-2\n"
            "No id,,tok-3\n"
            f"Long,user-4,{'x' * 4097}\n"
            "Ünï 🎉,user-🎉,tok-5\n"
            "Nul,,tok\x00\n"
        ).encode()

        parsed = parse_push_token_csv(content)

        assert parsed.devices == [
            {"line": 2, "distinct_id": "user-1", "device_token": "tok-1"},
            {"line": 5, "distinct_id": "user-2", "device_token": "tok-2"},
            {"line": 9, "distinct_id": "user-🎉", "device_token": "tok-5"},
        ]
        assert [(failure.line, failure.detail) for failure in parsed.rejected] == [
            (7, "No distinct ID"),
            (8, "Device token is longer than 4096 characters"),
            (10, "The line has a NUL character in it"),
        ]
        assert parsed.duplicate_lines == 1


class TestPushTokenImportJob(APIBaseTest):
    def setUp(self) -> None:
        super().setUp()
        self.organization_membership.level = OrganizationMembership.Level.ADMIN
        self.organization_membership.save()
        self.apns = Integration.objects.create(
            team=self.team,
            kind="apns",
            integration_id="TEAM.com.example.app",
            config={"bundle_id": "com.example.app", "team_id": "TEAM", "key_id": "KEY"},
        )
        self.storage = _MemoryStorage()
        patchers: list[Any] = [
            patch.object(push_token_import, "object_storage", self.storage),
            patch.object(push_token_import, "BATCH_SIZE", 2),
            patch.object(push_token_import, "BATCHES_PER_TASK", 1),
            patch("products.workflows.backend.tasks.push_token_import.run_push_token_import.delay"),
            patch("products.workflows.backend.tasks.push_token_import.check_push_token_import_stalled.apply_async"),
            patch("products.workflows.backend.tasks.push_token_import.expire_push_token_import_devices.apply_async"),
            patch.object(push_token_import, "ph_scoped_capture"),
        ]
        for patcher in patchers:
            patcher.start()
            self.addCleanup(patcher.stop)
        self.captured: list[list[dict[str, Any]]] = []
        self.drop_once: set[str] = set()
        capture = patch("posthog.api.integration_push_tokens.capture_batch_internal", side_effect=self._capture)
        capture.start()
        self.addCleanup(capture.stop)

    def _capture(self, *, events: list[dict[str, Any]], **kwargs: Any) -> CaptureInternalResult:
        self.captured.append(events)
        result = CaptureInternalResult(status_code=200)
        for event in events:
            dropped = event["distinct_id"] in self.drop_once
            self.drop_once.discard(event["distinct_id"])
            (result.dropped if dropped else result.ok).append(event["event_uuid"])
        return result

    def _upload(self, lines: list[str], integration_id: int | None = None, **data: Any):
        content = "\n".join(["distinct_id,device_token", *lines]).encode()
        return self.client.post(
            f"/api/projects/{self.team.pk}/push_token_imports/",
            {
                "integration_id": integration_id or self.apns.id,
                "file": SimpleUploadedFile("devices.csv", content, content_type="text/csv"),
                **data,
            },
            format="multipart",
        )

    def _run_to_end(self, import_id: str) -> dict[str, Any]:
        while run_push_token_import_batches(self.team.id, UUID(import_id)):
            pass
        return self.client.get(f"/api/projects/{self.team.pk}/push_token_imports/{import_id}/").json()

    def test_imports_the_file_in_batches_and_reports_each_failed_line(self) -> None:
        for distinct_id in ["user-1", "user-2", "user-3"]:
            _create_person(team=self.team, distinct_ids=[distinct_id])
        flush_persons_and_events()

        response = self._upload(
            [
                f"user-1,{APNS_TOKEN}",
                f"user-2,{APNS_TOKEN}",
                ",missing-id",
                "user-3,ExponentPushToken[abc]",
                f"nobody,{APNS_TOKEN}",
                f"user-3,{'b2' * 32}",
            ]
        )
        assert response.status_code == status.HTTP_201_CREATED, response.json()
        assert response.json()["status"] == "running"

        result = self._run_to_end(response.json()["id"])

        assert result["status"] == "completed"
        assert [len(batch) for batch in self.captured] == [2, 1]
        assert (result["total_devices"], result["imported_devices"], result["failed_devices"]) == (5, 3, 3)
        assert sorted((failure["line"], failure["reason"]) for failure in result["failures"]) == [
            (4, "invalid_row"),
            (5, "token_mismatch"),
            (6, "unknown_person"),
        ]
        assert result["unknown_person_devices"] == 1
        assert result["can_import_unknown_persons"] is True

    def test_imports_the_devices_of_unknown_people_once_with_new_persons(self) -> None:
        _create_person(team=self.team, distinct_ids=["user-1"])
        flush_persons_and_events()
        first = self._run_to_end(
            self._upload([f"user-1,{APNS_TOKEN}", f"new-1,{APNS_TOKEN}", f"new-2,{'c3' * 32}"]).json()["id"]
        )
        self.captured.clear()

        retry = self.client.post(
            f"/api/projects/{self.team.pk}/push_token_imports/{first['id']}/import_unknown_persons/"
        )
        again = self.client.post(
            f"/api/projects/{self.team.pk}/push_token_imports/{first['id']}/import_unknown_persons/"
        )

        assert retry.status_code == status.HTTP_201_CREATED, retry.json()
        assert again.status_code == status.HTTP_400_BAD_REQUEST
        result = self._run_to_end(retry.json()["id"])
        assert result["create_missing_persons"] is True
        assert (result["total_devices"], result["imported_devices"]) == (2, 2)
        assert sorted(event["distinct_id"] for batch in self.captured for event in batch) == ["new-1", "new-2"]
        assert (
            self.client.get(f"/api/projects/{self.team.pk}/push_token_imports/{first['id']}/").json()[
                "can_import_unknown_persons"
            ]
            is False
        )
        assert self.storage.objects == {}

    def test_a_channel_runs_one_import_at_a_time_and_a_cancelled_import_stops_sending(self) -> None:
        first = self._upload([f"user-1,{APNS_TOKEN}"]).json()
        second = self._upload([f"user-2,{APNS_TOKEN}"])
        assert second.status_code == status.HTTP_400_BAD_REQUEST
        assert "already running" in second.json()["detail"]

        cancelled = self.client.post(f"/api/projects/{self.team.pk}/push_token_imports/{first['id']}/cancel/")
        result = self._run_to_end(first["id"])

        assert (cancelled.json()["status"], cancelled.json()["finished_at"]) == ("cancelled", None)
        assert result["status"] == "cancelled"
        assert result["finished_at"] is not None
        assert self.captured == []
        assert self.storage.objects == {}
        assert self._upload([f"user-2,{APNS_TOKEN}"]).status_code == status.HTTP_201_CREATED

    def test_a_stop_that_lands_during_a_batch_still_counts_that_batch(self) -> None:
        import_id = self._upload([f"a,{APNS_TOKEN}", f"b,{APNS_TOKEN}", f"c,{APNS_TOKEN}"], create_missing_persons=True)
        import_id = import_id.json()["id"]

        def stop_while_sending(**kwargs: Any) -> CaptureInternalResult:
            self.client.post(f"/api/projects/{self.team.pk}/push_token_imports/{import_id}/cancel/")
            return self._capture(**kwargs)

        with patch("posthog.api.integration_push_tokens.capture_batch_internal", side_effect=stop_while_sending):
            result = self._run_to_end(import_id)

        assert (result["status"], result["processed_devices"], result["imported_devices"]) == ("cancelled", 2, 2)
        assert result["finished_at"] is not None
        assert self.storage.objects == {}

    @patch.object(push_token_import.time, "sleep")
    def test_a_device_ingestion_did_not_store_is_sent_again_with_its_event_id(self, _sleep: Any) -> None:
        self.drop_once = {"b"}
        import_id = self._upload([f"a,{APNS_TOKEN}", f"b,{APNS_TOKEN}"], create_missing_persons=True).json()["id"]

        result = self._run_to_end(import_id)

        assert (result["imported_devices"], result["failed_devices"]) == (2, 0)
        assert [event["distinct_id"] for event in self.captured[1]] == ["b"]
        assert self.captured[1][0]["event_uuid"] == self.captured[0][1]["event_uuid"]

    def test_a_failed_delete_keeps_the_devices_known_so_cleanup_runs_again(self) -> None:
        import_id = self._upload([f"a,{APNS_TOKEN}"], create_missing_persons=True).json()["id"]
        with (
            patch.object(self.storage, "delete", side_effect=RuntimeError("storage down")),
            patch(
                "products.workflows.backend.tasks.push_token_import.expire_push_token_import_devices.apply_async"
            ) as retry_cleanup,
        ):
            self._run_to_end(import_id)

        assert PushTokenImport.objects.for_team(self.team.id).get(id=import_id).storage_key != ""
        retry_cleanup.assert_called_once()

    def test_a_batch_sent_again_after_a_crash_reuses_its_event_ids(self) -> None:
        lines = [f"a,{APNS_TOKEN}", f"b,{APNS_TOKEN}", f"c,{APNS_TOKEN}"]
        import_id = self._upload(lines, create_missing_persons=True).json()["id"]
        assert run_push_token_import_batches(self.team.id, UUID(import_id)) is True
        first_uuids = [event["event_uuid"] for event in self.captured[0]]
        PushTokenImport.objects.for_team(self.team.id).filter(id=import_id).update(processed_devices=0)
        self.captured.clear()

        run_push_token_import_batches(self.team.id, UUID(import_id))

        assert [event["event_uuid"] for event in self.captured[0]] == first_uuids

    def test_an_import_that_stops_making_progress_is_resumed_then_failed(self) -> None:
        import_id = UUID(self._upload([f"user-1,{APNS_TOKEN}"]).json()["id"])
        assert check_stalled_push_token_import(self.team.id, import_id) is True

        with patch("products.workflows.backend.tasks.push_token_import.run_push_token_import.delay") as resume:
            for minutes in [6, 12, 18]:
                with time_machine.travel(timezone.now() + timedelta(minutes=minutes), tick=False):
                    assert check_stalled_push_token_import(self.team.id, import_id) is True
            assert resume.call_count == 3
            with time_machine.travel(timezone.now() + timedelta(minutes=24), tick=False):
                assert check_stalled_push_token_import(self.team.id, import_id) is False

        job = PushTokenImport.objects.for_team(self.team.id).get(id=import_id)
        assert job.status == "failed"
        assert "stopped after 0 of 1 devices" in job.error
        assert self.storage.objects == {}

    @parameterized.expand(
        [
            (["integration:write"], status.HTTP_403_FORBIDDEN),
            (["integration:write", "person:write"], status.HTTP_201_CREATED),
        ]
    )
    def test_an_api_key_needs_person_access_to_import(self, scopes: list[str], expected: int) -> None:
        token = generate_random_token_personal()
        PersonalAPIKey.objects.create(label="t", user=self.user, secure_value=hash_key_value(token), scopes=scopes)
        self.client.logout()

        response = self.client.post(
            f"/api/projects/{self.team.pk}/push_token_imports/",
            {
                "integration_id": self.apns.id,
                "file": SimpleUploadedFile("d.csv", f"distinct_id,device_token\na,{APNS_TOKEN}".encode()),
            },
            format="multipart",
            HTTP_AUTHORIZATION=f"Bearer {token}",
        )

        assert response.status_code == expected, response.json()

    @parameterized.expand(
        [
            ("member", OrganizationMembership.Level.MEMBER, None, status.HTTP_403_FORBIDDEN),
            ("not_a_push_channel", OrganizationMembership.Level.ADMIN, "slack", status.HTTP_400_BAD_REQUEST),
            ("another_team", OrganizationMembership.Level.ADMIN, "other_team", status.HTTP_400_BAD_REQUEST),
        ]
    )
    def test_refuses_an_upload_into_a_channel_the_user_cannot_change(
        self, _name: str, level: int, channel: str | None, expected: int
    ) -> None:
        self.organization_membership.level = level
        self.organization_membership.save()
        integration_id = self.apns.id
        if channel == "slack":
            integration_id = Integration.objects.create(team=self.team, kind="slack", config={}).id
        if channel == "other_team":
            other_team = Team.objects.create(organization=self.organization)
            integration_id = Integration.objects.create(
                team=other_team, kind="apns", integration_id="x", config={"bundle_id": "com.example.other"}
            ).id

        response = self._upload([f"user-1,{APNS_TOKEN}"], integration_id=integration_id)

        assert response.status_code == expected, response.json()
        assert PushTokenImport.objects.for_team(self.team.id).count() == 0
        assert self.storage.objects == {}
