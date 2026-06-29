from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any
from urllib.parse import urlencode
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


class RunnerApiError(RuntimeError):
    pass


@dataclass(frozen=True, slots=True)
class RunnerClient:
    api_base: str
    token: str
    host_header: str = ""

    def exchange_api_key_for_runner_token(
        self,
        *,
        api_key: str,
        api_secret: str,
        runner_name: str,
        workspace_root: str = "",
        project_id: str = "",
        capabilities: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        payload = {
            "apiKey": api_key,
            "apiSecret": api_secret,
            "runnerName": runner_name,
            "workspaceRoot": workspace_root,
            "capabilities": capabilities or {},
        }
        if project_id:
            payload["projectId"] = project_id
        return self.post("/runner/v1/auth/exchange/", payload)

    def create_session(self, *, manifest: dict[str, Any]) -> dict[str, Any]:
        payload = self.post("/runner/v1/sessions/", {"manifest": manifest, "workspaceRoot": manifest["workspaceRoot"]})
        return payload["session"]

    def heartbeat(self, *, session_id: str, manifest: dict[str, Any]) -> dict[str, Any]:
        return self.post(f"/runner/v1/sessions/{session_id}/heartbeat/", {"manifest": manifest})

    def lease(self, *, session_id: str) -> dict[str, Any]:
        return self.post("/runner/v1/jobs/lease/", {"sessionId": session_id})

    def start(self, *, session_id: str, job_id: str) -> None:
        self.post(f"/runner/v1/jobs/{job_id}/start/", {"sessionId": session_id})

    def event(
        self,
        *,
        session_id: str,
        job_id: str,
        event_type: str,
        message: str = "",
        stream: str = "",
        payload: dict[str, Any] | None = None,
    ) -> None:
        self.post(
            f"/runner/v1/jobs/{job_id}/events/",
            {
                "sessionId": session_id,
                "eventType": event_type,
                "stream": stream,
                "message": message,
                "payload": payload or {},
            },
        )

    def complete(self, *, session_id: str, job_id: str, result: dict[str, Any]) -> None:
        self.post(f"/runner/v1/jobs/{job_id}/complete/", {"sessionId": session_id, "result": result})

    def fail(self, *, session_id: str, job_id: str, error: dict[str, Any]) -> None:
        self.post(f"/runner/v1/jobs/{job_id}/fail/", {"sessionId": session_id, "error": error})

    def start_workflow_execution(
        self,
        *,
        workflow_id: str,
        session_id: str,
        inputs: dict[str, Any],
        human_input: dict[str, Any] | None,
        human_timeout_seconds: int,
    ) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "sessionId": session_id,
            "inputs": inputs,
            "humanTimeoutSeconds": human_timeout_seconds,
        }
        if human_input is not None:
            payload["humanInput"] = human_input
        return self.post(f"/runner/v1/workflows/{workflow_id}/executions/", payload)

    def get_workflow_execution(self, *, execution_id: str) -> dict[str, Any]:
        return self.get(f"/runner/v1/executions/{execution_id}/")

    def list_workflows(self) -> dict[str, Any]:
        return self.get("/runner/v1/workflows/")

    def get_workflow(self, *, workflow_id: str) -> dict[str, Any]:
        return self.get(f"/runner/v1/workflows/{workflow_id}/")

    def list_human_requests(self, *, execution_id: str, status: str = "pending") -> dict[str, Any]:
        query = f"?{urlencode({'status': status})}" if status else ""
        return self.get(f"/runner/v1/executions/{execution_id}/human-requests/{query}")

    def resolve_human_request(self, *, request_id: str, answer: dict[str, Any]) -> dict[str, Any]:
        return self.post(f"/runner/v1/human-requests/{request_id}/resolve/", {"answer": answer})

    def post(self, path: str, payload: dict[str, Any]) -> dict[str, Any]:
        body = json.dumps(payload).encode("utf-8")
        request = self.request(path=path, method="POST", data=body)
        return self.open_json(request)

    def get(self, path: str) -> dict[str, Any]:
        request = self.request(path=path, method="GET", data=None)
        return self.open_json(request)

    def request(self, *, path: str, method: str, data: bytes | None) -> Request:
        headers = {
            "Content-Type": "application/json",
            "User-Agent": "loomex-runner/0.1",
        }
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        if self.host_header:
            headers["Host"] = self.host_header
        return Request(
            f"{self.api_base.rstrip('/')}{path}",
            data=data,
            headers=headers,
            method=method,
        )

    def open_json(self, request: Request) -> dict[str, Any]:
        try:
            with urlopen(request, timeout=30) as response:
                data = json.loads(response.read().decode("utf-8"))
        except HTTPError as exc:
            error_body = exc.read().decode("utf-8", errors="replace")
            raise RunnerApiError(f"Runner API error {exc.code}: {error_body}") from exc
        except URLError as exc:
            raise RunnerApiError(f"Runner API unavailable: {exc}") from exc
        if not isinstance(data, dict) or "data" not in data:
            raise RunnerApiError(f"Unexpected Runner API response: {data!r}")
        return data["data"]
