from __future__ import annotations

import argparse
import getpass
import json
import shutil
import sys
import threading
import time
from pathlib import Path
from typing import Any

from loomex_runner.client import RunnerClient
from loomex_runner.config import (
    DEFAULT_PROFILE,
    add_workspace,
    config_migration_status,
    load_config,
    profile_settings,
    resolve_workspace,
    set_login,
)
from loomex_runner.jobs import RunnerJobError, run_job, runner_manifest


DEFAULT_LOCAL_SERVER = "http://127.0.0.1:28000/api/v1/runner-control"
LEGACY_COMPATIBILITY_WINDOW = (
    "dev-smoke compatibility until Rust/gRPC `loomex` runner passes Phase 50 acceptance, "
    "then one stable `loomex` release"
)
REST_LONG_POLL_REMOVAL_WINDOW = "after Phase 80 migration sign-off"
LEGACY_COMMAND_MAP = {
    "login": "loomex login",
    "workspace": "loomex bind --workspace PATH",
    "doctor": "loomex runner doctor",
    "start": "loomex runner start",
    "run": "loomex workflow run WORKFLOW_ID",
    "connect": "loomex bind --workspace PATH && loomex runner start",
}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="loomex-runner")
    subparsers = parser.add_subparsers(dest="command")

    login_parser = subparsers.add_parser("login", help="Authenticate this machine as a Loomex runner.")
    add_connection_args(login_parser)
    login_parser.add_argument("--api-key", default="", help="Organization API public key.")
    login_parser.add_argument("--api-secret", default="", help="Organization API secret key.")
    login_parser.add_argument("--runner-name", default="", help="Runner name stored in Loomex.")
    login_parser.add_argument("--workspace", default="", help="Default local workspace root for the runner.")
    login_parser.add_argument("--project", default="", help="Optional project id to scope this runner.")
    login_parser.add_argument("--browser", action="store_true", help="Use browser/device login when enabled by server.")

    workspace_parser = subparsers.add_parser("workspace", help="Manage local workspaces.")
    workspace_subparsers = workspace_parser.add_subparsers(dest="workspace_command", required=True)
    workspace_add = workspace_subparsers.add_parser("add", help="Register a local workspace path.")
    workspace_add.add_argument("name")
    workspace_add.add_argument("path")

    doctor_parser = subparsers.add_parser("doctor", help="Print runner environment diagnostics.")
    doctor_parser.add_argument("--profile", default="", help="Config profile name.")

    start_parser = subparsers.add_parser("start", help="Start polling Loomex for jobs.")
    add_connection_args(start_parser)
    start_parser.add_argument("--workspace", required=True, help="Workspace name or absolute path.")
    start_parser.add_argument("--once", action="store_true")
    start_parser.add_argument("--poll-interval", type=float, default=1.0)
    start_parser.add_argument("--heartbeat-interval", type=float, default=15.0)

    run_parser = subparsers.add_parser("run", help="Connect this workspace and execute one workflow.")
    add_run_args(run_parser)
    connect_parser = subparsers.add_parser("connect", help="Alias for run.")
    add_run_args(connect_parser)

    args = parser.parse_args(argv)
    if args.command is None:
        args = default_run_args()
    emit_legacy_warning(args.command)
    if args.command == "login":
        return handle_login(args)
    if args.command == "workspace":
        return handle_workspace(args)
    if args.command == "doctor":
        return handle_doctor(args)
    if args.command == "start":
        return handle_start(args)
    if args.command in {"run", "connect"}:
        return handle_run(args)
    parser.error("unknown command")
    return 2


def add_connection_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--profile", default="", help="Config profile name.")
    parser.add_argument("--server", default="", help="Runner control API base URL.")
    parser.add_argument("--token", default="", help="Runner token issued by Loomex.")
    parser.add_argument("--host-header", default="", help="Optional Host header for local nginx routes.")


def add_run_args(parser: argparse.ArgumentParser) -> None:
    add_connection_args(parser)
    parser.add_argument("--workflow", default="", help="Workflow id to execute.")
    parser.add_argument("--workspace", default="", help="Workspace name or absolute local path.")
    parser.add_argument("--input", default="", help="Workflow JSON input. Prefix with @ to read from a file.")
    parser.add_argument("--input-json", default="", help=argparse.SUPPRESS)
    parser.add_argument("--human-input", default="", help="Human-node JSON answer. Prefix with @ to read from a file.")
    parser.add_argument("--prompt", default="", help="Shortcut for --human-input '{\"prompt\":\"...\"}'.")
    parser.add_argument("--timeout", type=int, default=900)
    parser.add_argument("--human-timeout", type=int, default=120)
    parser.add_argument("--poll-interval", type=float, default=1.0)
    parser.add_argument("--heartbeat-interval", type=float, default=15.0)
    parser.add_argument("--non-interactive", action="store_true")


def default_run_args() -> argparse.Namespace:
    return argparse.Namespace(
        command="run",
        profile="",
        server="",
        token="",
        host_header="",
        workflow="",
        workspace="",
        input="",
        input_json="",
        human_input="",
        prompt="",
        timeout=900,
        human_timeout=120,
        poll_interval=1.0,
        heartbeat_interval=15.0,
        non_interactive=False,
    )


def handle_login(args: argparse.Namespace) -> int:
    if getattr(args, "browser", False):
        raise SystemExit("Browser login is not enabled by the Loomex server yet. Use API key/secret or --token.")
    config = load_config()
    profile = profile_name(args)
    existing = profile_settings(config, profile)
    server = normalize_server_url(args.server or prompt_text("Server", existing.get("server") or DEFAULT_LOCAL_SERVER))
    host_header = args.host_header or prompt_text(
        "Host header",
        existing.get("hostHeader") or default_host_header(server),
        allow_empty=True,
    )
    workspace = args.workspace or ""
    if workspace:
        workspace = str(Path(workspace).expanduser().resolve())

    if args.token:
        saved = set_login(server=server, token=args.token, host_header=host_header, profile=profile)
        print(json.dumps({"status": "ok", "profile": profile, "server": profile_settings(saved, profile)["server"]}))
        return 0

    api_key = args.api_key or prompt_text("API key", "")
    api_secret = args.api_secret or getpass.getpass("API secret: ")
    runner_name = args.runner_name or prompt_text("Runner name", "Local runner")
    client = RunnerClient(api_base=server, token="", host_header=host_header)
    response = client.exchange_api_key_for_runner_token(
        api_key=api_key,
        api_secret=api_secret,
        runner_name=runner_name,
        workspace_root=workspace,
        project_id=args.project,
        capabilities={"cli": True},
    )
    runner = response["runner"]
    saved = set_login(
        server=server,
        token=response["runnerToken"],
        host_header=host_header,
        profile=profile,
        runner_id=str(runner.get("id") or ""),
        organization_id=str(response.get("organizationId") or ""),
        project_id=str(response.get("projectId") or ""),
    )
    payload = {
        "status": "ok",
        "profile": profile,
        "server": profile_settings(saved, profile)["server"],
        "runner": {"id": runner.get("id"), "name": runner.get("name")},
    }
    print(json.dumps(payload, ensure_ascii=False))
    return 0


def handle_workspace(args: argparse.Namespace) -> int:
    config = add_workspace(name=args.name, workspace_path=args.path)
    print(json.dumps({"status": "ok", "workspace": args.name, "path": config["workspaces"][args.name]}))
    return 0


def handle_doctor(args: argparse.Namespace) -> int:
    config = load_config()
    settings = profile_settings(config, profile_name(args))
    diagnostics = {
        "python": sys.version.split()[0],
        "profile": profile_name(args) or config.get("defaultProfile") or DEFAULT_PROFILE,
        "serverConfigured": bool(settings.get("server")),
        "tokenConfigured": bool(settings.get("token")),
        "workspaces": config.get("workspaces") if isinstance(config.get("workspaces"), dict) else {},
        "tools": {
            "git": shutil.which("git") or "",
            "python3": shutil.which("python3") or "",
            "node": shutil.which("node") or "",
            "npm": shutil.which("npm") or "",
        },
        "configMigration": config_migration_status(),
    }
    print(json.dumps(diagnostics, indent=2, sort_keys=True))
    return 0 if diagnostics["serverConfigured"] and diagnostics["tokenConfigured"] else 1


def handle_start(args: argparse.Namespace) -> int:
    config = load_config()
    settings = profile_settings(config, profile_name(args))
    server = normalize_server_url(args.server or str(settings.get("server") or ""))
    token = args.token or str(settings.get("token") or "")
    host_header = args.host_header or str(settings.get("hostHeader") or "")
    if not server or not token:
        raise SystemExit("Run `loomex-runner login` or pass --server and --token.")
    workspace = resolve_workspace(config, args.workspace)
    workspace.mkdir(parents=True, exist_ok=True)
    client = RunnerClient(api_base=server, token=token, host_header=host_header)
    session = client.create_session(manifest=runner_manifest(workspace))
    session_id = session["id"]
    print_event({"event": "connected", "sessionId": session_id, "workspace": str(workspace)})

    processed = 0
    last_heartbeat = 0.0
    while True:
        now = time.monotonic()
        if now - last_heartbeat >= args.heartbeat_interval:
            client.heartbeat(session_id=session_id, manifest=runner_manifest(workspace))
            last_heartbeat = now
        leased = client.lease(session_id=session_id)
        job = leased.get("job")
        if not job:
            if args.once and processed:
                return 0
            time.sleep(max(args.poll_interval, 0.1))
            continue
        processed += 1
        process_job(client=client, session_id=session_id, workspace=workspace, job=job)
        if args.once:
            return 0


def handle_run(args: argparse.Namespace) -> int:
    interactive = not bool(getattr(args, "non_interactive", False)) and sys.stdin.isatty()
    config = load_config()
    profile = profile_name(args)
    settings = profile_settings(config, profile)
    server = normalize_server_url(args.server or str(settings.get("server") or ""))
    token = args.token or str(settings.get("token") or "")
    host_header = args.host_header or str(settings.get("hostHeader") or "")
    if not server or not token:
        if not interactive:
            raise SystemExit("Run `loomex-runner login` or pass --server and --token.")
        login_args = argparse.Namespace(
            browser=False,
            profile=profile,
            server=server,
            token="",
            host_header=host_header,
            api_key="",
            api_secret="",
            runner_name="",
            workspace=args.workspace,
            project="",
        )
        handle_login(login_args)
        config = load_config()
        settings = profile_settings(config, profile)
        server = normalize_server_url(str(settings.get("server") or ""))
        token = str(settings.get("token") or "")
        host_header = str(settings.get("hostHeader") or "")

    client = RunnerClient(api_base=server, token=token, host_header=host_header)
    workflow_id = args.workflow or select_workflow(client, interactive=interactive)
    workflow_detail = client.get_workflow(workflow_id=workflow_id)
    workspace = resolve_run_workspace(config, args.workspace, interactive=interactive)
    manifest = runner_manifest(workspace)
    session = client.create_session(manifest=manifest)
    session_id = session["id"]
    print_event({"event": "connected", "sessionId": session_id, "workspace": str(workspace)})

    stop_event = threading.Event()
    worker_errors: list[str] = []
    worker = threading.Thread(
        target=runner_loop,
        kwargs={
            "client": client,
            "session_id": session_id,
            "workspace": workspace,
            "poll_interval": args.poll_interval,
            "heartbeat_interval": args.heartbeat_interval,
            "stop_event": stop_event,
            "errors": worker_errors,
        },
        daemon=True,
    )
    worker.start()

    human_input = parse_human_input(args)
    try:
        started = client.start_workflow_execution(
            workflow_id=workflow_id,
            session_id=session_id,
            inputs=parse_workflow_inputs(args, workflow_detail=workflow_detail, interactive=interactive),
            human_input=human_input,
            human_timeout_seconds=max(args.human_timeout, 1),
        )
        execution = started["execution"]
        execution_id = execution["id"]
        print_event({"event": "workflow_started", "executionId": execution_id, "status": execution["status"]})
        final_execution = wait_for_workflow(
            client=client,
            execution_id=execution_id,
            timeout_seconds=max(args.timeout, 1),
            poll_interval=max(args.poll_interval, 0.1),
            human_input=human_input,
            interactive=interactive,
        )
        print_event({"event": "workflow_finished", "execution": final_execution})
        return 0 if workflow_succeeded(final_execution) else 1
    finally:
        stop_event.set()
        worker.join(timeout=5)
        if worker_errors:
            print_event({"event": "runner_worker_error", "errors": worker_errors[-3:]})


def resolve_run_workspace(config: dict[str, Any], raw_workspace: str, *, interactive: bool) -> Path:
    workspace = raw_workspace
    if not workspace:
        if not interactive:
            raise SystemExit("--workspace is required in non-interactive mode.")
        workspace = prompt_text("Workspace path", str(Path.cwd()))
    path = resolve_workspace(config, workspace)
    path.mkdir(parents=True, exist_ok=True)
    return path


def select_workflow(client: RunnerClient, *, interactive: bool) -> str:
    workflows = client.list_workflows().get("workflows") or []
    if not workflows:
        raise SystemExit("No active workflows are available for this runner.")
    if not interactive:
        raise SystemExit("--workflow is required in non-interactive mode.")
    if len(workflows) == 1:
        workflow = workflows[0]
        print_event({"event": "workflow_selected", "workflowId": workflow["id"], "name": workflow.get("name")})
        return str(workflow["id"])
    print("Available workflows:", file=sys.stderr)
    for index, workflow in enumerate(workflows, start=1):
        print(f"  {index}. {workflow.get('name')} ({workflow.get('id')})", file=sys.stderr)
    while True:
        raw = prompt_text("Workflow number or id", "1")
        if raw.isdigit() and 1 <= int(raw) <= len(workflows):
            return str(workflows[int(raw) - 1]["id"])
        matching = [item for item in workflows if str(item.get("id")) == raw]
        if matching:
            return str(matching[0]["id"])
        print("Invalid workflow selection.", file=sys.stderr)


def runner_loop(
    *,
    client: RunnerClient,
    session_id: str,
    workspace: Path,
    poll_interval: float,
    heartbeat_interval: float,
    stop_event: threading.Event,
    errors: list[str],
) -> None:
    last_heartbeat = 0.0
    while not stop_event.is_set():
        try:
            now = time.monotonic()
            if now - last_heartbeat >= heartbeat_interval:
                client.heartbeat(session_id=session_id, manifest=runner_manifest(workspace))
                last_heartbeat = now
            leased = client.lease(session_id=session_id)
            job = leased.get("job")
            if job:
                process_job(client=client, session_id=session_id, workspace=workspace, job=job)
                continue
        except Exception as exc:
            errors.append(str(exc))
            print_event({"event": "runner_worker_error", "error": str(exc)})
            time.sleep(max(poll_interval, 1.0))
            continue
        stop_event.wait(max(poll_interval, 0.1))


def wait_for_workflow(
    *,
    client: RunnerClient,
    execution_id: str,
    timeout_seconds: int,
    poll_interval: float,
    human_input: dict[str, Any] | None = None,
    interactive: bool = False,
) -> dict[str, Any]:
    terminal = {"succeeded", "completed", "failed", "canceled"}
    deadline = time.monotonic() + timeout_seconds
    last_status = ""
    while time.monotonic() < deadline:
        payload = client.get_workflow_execution(execution_id=execution_id)
        execution = payload["execution"]
        status = str(execution.get("status") or "")
        if status != last_status:
            print_event({"event": "workflow_status", "executionId": execution_id, "status": status})
            last_status = status
        if status in terminal:
            return execution
        resolve_pending_human_requests(
            client=client,
            execution_id=execution_id,
            human_input=human_input,
            interactive=interactive,
        )
        time.sleep(max(poll_interval, 0.1))
    raise SystemExit(f"Timed out waiting for workflow execution {execution_id}")


def resolve_pending_human_requests(
    *,
    client: RunnerClient,
    execution_id: str,
    human_input: dict[str, Any] | None,
    interactive: bool,
) -> None:
    pending = client.list_human_requests(execution_id=execution_id).get("humanRequests") or []
    for item in pending:
        answer = human_input
        if answer is None:
            if not interactive:
                continue
            answer = prompt_human_answer(item)
        response = client.resolve_human_request(request_id=item["id"], answer=answer)
        print_event({"event": "human_request_resolved", "requestId": item["id"], "response": response})


def workflow_succeeded(execution: dict[str, Any]) -> bool:
    return str(execution.get("status") or "") in {"succeeded", "completed"}


def parse_workflow_inputs(
    args: argparse.Namespace,
    *,
    workflow_detail: dict[str, Any] | None = None,
    interactive: bool = False,
) -> dict[str, Any]:
    raw = getattr(args, "input_json", "") or getattr(args, "input", "")
    if raw:
        return normalize_json_object(parse_json_argument(raw, "--input"))
    if interactive:
        schema = (workflow_detail or {}).get("inputSchema")
        if schema_requires_prompt(schema):
            return normalize_json_object(prompt_value_for_schema("Workflow input", schema))
    return {}


def parse_human_input(args: argparse.Namespace) -> dict[str, Any] | None:
    if getattr(args, "human_input", ""):
        return normalize_human_answer(parse_json_argument(args.human_input, "--human-input"))
    if getattr(args, "prompt", ""):
        return {"prompt": args.prompt}
    if getattr(args, "input_json", ""):
        return normalize_human_answer(parse_json_argument(args.input_json, "--input-json"))
    if not hasattr(args, "human_input") and getattr(args, "input", ""):
        return {"prompt": args.input}
    return None


def parse_json_argument(raw: str, option_name: str) -> Any:
    text = raw
    if text.startswith("@"):
        text = Path(text[1:]).expanduser().read_text(encoding="utf-8")
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        if option_name in {"--input", "--human-input"}:
            return text
        raise SystemExit(f"{option_name} must be valid JSON")


def normalize_json_object(value: Any) -> dict[str, Any]:
    if value is None:
        return {}
    if isinstance(value, dict):
        return value
    return {"value": value}


def normalize_human_answer(value: Any) -> dict[str, Any]:
    if isinstance(value, dict):
        return value
    return {"value": value}


def prompt_human_answer(item: dict[str, Any]) -> dict[str, Any]:
    title = item.get("question") or item.get("title") or "Human input"
    print(f"\n{title}", file=sys.stderr)
    schema = item.get("outputSchema") if isinstance(item.get("outputSchema"), dict) else None
    return normalize_human_answer(prompt_value_for_schema("Answer", schema))


def prompt_value_for_schema(label: str, schema: Any) -> Any:
    if not isinstance(schema, dict):
        return parse_json_argument(prompt_text(label, "{}"), label)
    schema_type = schema.get("type")
    if isinstance(schema_type, list):
        schema_type = next((item for item in schema_type if item != "null"), schema_type[0] if schema_type else "object")
    if schema_type == "object" or (schema_type is None and isinstance(schema.get("properties"), dict)):
        properties = schema.get("properties") if isinstance(schema.get("properties"), dict) else {}
        if not properties:
            return parse_json_argument(prompt_text(label, "{}"), label)
        result = {}
        required = set(schema.get("required") if isinstance(schema.get("required"), list) else [])
        for key, child_schema in properties.items():
            prompt_label = f"{label}.{key}"
            if key not in required:
                raw = prompt_text(prompt_label, "", allow_empty=True)
                if raw == "":
                    continue
                result[key] = parse_scalar_or_json(raw, child_schema)
                continue
            result[key] = prompt_value_for_schema(prompt_label, child_schema)
        return result
    if schema_type in {"array", "boolean", "integer", "number"}:
        return parse_json_argument(prompt_text(label, "[]"), label)
    return prompt_text(label, "")


def parse_scalar_or_json(raw: str, schema: Any) -> Any:
    if isinstance(schema, dict) and schema.get("type") == "string":
        return raw
    return parse_json_argument(raw, "value")


def schema_requires_prompt(schema: Any) -> bool:
    if not isinstance(schema, dict):
        return False
    properties = schema.get("properties")
    required = schema.get("required")
    return bool(properties or required or schema.get("type") not in {None, "object"})


def prompt_text(label: str, default: str, *, allow_empty: bool = False) -> str:
    suffix = f" [{default}]" if default else ""
    while True:
        value = input(f"{label}{suffix}: ").strip()
        if value:
            return value
        if default or allow_empty:
            return default


def profile_name(args: argparse.Namespace) -> str:
    return str(getattr(args, "profile", "") or DEFAULT_PROFILE)


def normalize_server_url(value: str) -> str:
    server = str(value or "").strip().rstrip("/")
    if not server:
        return ""
    if server.endswith("/api/v1/runner-control"):
        return server
    if server.endswith("/api/v1"):
        return f"{server}/runner-control"
    return f"{server}/api/v1/runner-control"


def default_host_header(server: str) -> str:
    if "127.0.0.1:28000" in server or "localhost:28000" in server:
        return "loomex.localhost"
    return ""


def process_job(*, client: RunnerClient, session_id: str, workspace: Path, job: dict[str, Any]) -> None:
    job_id = job["id"]
    kind = job.get("kind")
    print_event({"event": "job", "jobId": job_id, "kind": kind})
    try:
        client.start(session_id=session_id, job_id=job_id)
        result = run_job(workspace=workspace, job=job)
        client.event(
            session_id=session_id,
            job_id=job_id,
            event_type=f"{kind}.completed",
            message="job completed",
            payload={"resultKeys": sorted(result.keys())},
        )
        client.complete(session_id=session_id, job_id=job_id, result=result)
    except (RunnerJobError, RuntimeError, OSError, ValueError) as exc:
        error = {"code": "RUNNER_JOB_FAILED", "message": str(exc)}
        try:
            client.fail(session_id=session_id, job_id=job_id, error=error)
        finally:
            print_event({"event": "job_failed", "jobId": job_id, "error": error})


def print_event(payload: dict[str, Any]) -> None:
    print(json.dumps(payload, ensure_ascii=False), flush=True)


def legacy_warning_message(command: str | None) -> str:
    mapped = LEGACY_COMMAND_MAP.get(command or "run", "loomex")
    return (
        "DEPRECATION: `loomex-runner` is a legacy Python spike compatibility command; "
        f"use `{mapped}` for the production CLI path. Compatibility window: {LEGACY_COMPATIBILITY_WINDOW}. "
        f"REST long-poll removal target: {REST_LONG_POLL_REMOVAL_WINDOW}. "
        "This warning never prints token or secret values."
    )


def emit_legacy_warning(command: str | None) -> None:
    if str(command or "") in {"", "help"}:
        return
    print(legacy_warning_message(command), file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
