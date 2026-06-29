from __future__ import annotations

import base64
import os
import platform
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


class RunnerJobError(RuntimeError):
    pass


def runner_manifest(workspace: Path) -> dict[str, Any]:
    return {
        "runnerVersion": "0.1.0",
        "transport": "long_poll",
        "workspaceRoot": str(workspace),
        "os": platform.system().lower(),
        "platform": platform.platform(),
        "python": sys.version.split()[0],
        "cwd": os.getcwd(),
        "capabilities": {
            "command.run": True,
            "file.list": True,
            "file.read_many": True,
            "file.write_many": True,
        },
    }


def run_job(*, workspace: Path, job: dict[str, Any]) -> dict[str, Any]:
    kind = str(job.get("kind") or "")
    payload = job.get("payload") if isinstance(job.get("payload"), dict) else {}
    if kind == "command.run":
        return execute_command(workspace=workspace, payload=payload)
    if kind == "file.list":
        return list_files(workspace=workspace, payload=payload)
    if kind == "file.read_many":
        return read_many(workspace=workspace, payload=payload)
    if kind == "file.write_many":
        return write_many(workspace=workspace, payload=payload)
    raise RunnerJobError(f"Unsupported runner job kind: {kind}")


def execute_command(*, workspace: Path, payload: dict[str, Any]) -> dict[str, Any]:
    cwd = safe_cwd(workspace, str(payload.get("cwd") or "."))
    command = payload.get("command")
    shell = bool(payload.get("shell", not isinstance(command, list)))
    timeout_seconds = int(payload.get("timeoutSeconds") or 60)
    max_output_bytes = int(payload.get("maxOutputBytes") or 200000)
    if isinstance(command, list):
        argv: str | list[str] = [str(item) for item in command]
    else:
        argv = str(command or "").strip()
    if not argv:
        raise RunnerJobError("command.run payload.command is required")
    started = time.monotonic()
    try:
        completed = subprocess.run(
            argv,
            cwd=str(cwd),
            shell=shell,
            text=True,
            capture_output=True,
            timeout=max(timeout_seconds, 1),
            check=False,
        )
        exit_code = completed.returncode
        stdout = truncate_output(completed.stdout, max_output_bytes)
        stderr = truncate_output(completed.stderr, max_output_bytes)
    except subprocess.TimeoutExpired as exc:
        exit_code = 124
        stdout = truncate_output(exc.stdout or "", max_output_bytes)
        stderr = truncate_output((exc.stderr or "") + f"\nTimeout after {timeout_seconds}s", max_output_bytes)
    return {
        "exitCode": exit_code,
        "stdout": stdout,
        "stderr": stderr,
        "cwd": str(cwd),
        "durationSeconds": round(time.monotonic() - started, 4),
    }


def list_files(*, workspace: Path, payload: dict[str, Any]) -> dict[str, Any]:
    root = safe_cwd(workspace, str(payload.get("path") or "."))
    limit = int(payload.get("limit") or 200)
    include_hidden = bool(payload.get("includeHidden", False))
    files: list[dict[str, Any]] = []
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(workspace).as_posix()
        if not include_hidden and any(part.startswith(".") for part in path.relative_to(workspace).parts):
            continue
        files.append(
            {
                "path": relative,
                "type": "directory" if path.is_dir() else "file",
                "sizeBytes": path.stat().st_size if path.is_file() else 0,
            }
        )
        if len(files) >= limit:
            break
    return {"files": files, "truncated": len(files) >= limit}


def read_many(*, workspace: Path, payload: dict[str, Any]) -> dict[str, Any]:
    max_bytes = int(payload.get("maxBytesPerFile") or 200000)
    files = payload.get("files") if isinstance(payload.get("files"), list) else []
    result = []
    for item in files:
        path = safe_file_path(workspace, str(item))
        data = path.read_bytes()
        truncated = len(data) > max_bytes
        data = data[:max_bytes]
        result.append(
            {
                "path": path.relative_to(workspace).as_posix(),
                "content": data.decode("utf-8", errors="replace"),
                "truncated": truncated,
            }
        )
    return {"files": result}


def write_many(*, workspace: Path, payload: dict[str, Any]) -> dict[str, Any]:
    files = payload.get("files") if isinstance(payload.get("files"), list) else []
    written = []
    for item in files:
        if not isinstance(item, dict):
            raise RunnerJobError("file.write_many files must contain objects")
        path = safe_file_path(workspace, str(item.get("path") or ""))
        path.parent.mkdir(parents=True, exist_ok=True)
        encoding = str(item.get("encoding") or "utf-8")
        if encoding == "base64":
            data = base64.b64decode(str(item.get("content") or "").encode("ascii"))
            path.write_bytes(data)
            size = len(data)
        else:
            content = str(item.get("content") or "")
            path.write_text(content, encoding="utf-8")
            size = len(content.encode("utf-8"))
        written.append({"path": path.relative_to(workspace).as_posix(), "sizeBytes": size})
    return {"status": "succeeded", "writtenFiles": written}


def safe_cwd(workspace: Path, raw_cwd: str) -> Path:
    candidate = (workspace / raw_cwd).resolve()
    try:
        candidate.relative_to(workspace)
    except ValueError as exc:
        raise RunnerJobError("cwd must stay inside runner workspace") from exc
    candidate.mkdir(parents=True, exist_ok=True)
    return candidate


def safe_file_path(workspace: Path, raw_path: str) -> Path:
    if not raw_path.strip():
        raise RunnerJobError("file path is required")
    candidate = (workspace / raw_path).resolve()
    try:
        candidate.relative_to(workspace)
    except ValueError as exc:
        raise RunnerJobError("file path must stay inside runner workspace") from exc
    return candidate


def truncate_output(value: str | bytes, max_bytes: int) -> str:
    if isinstance(value, bytes):
        value = value.decode("utf-8", errors="replace")
    encoded = value.encode("utf-8")
    if len(encoded) <= max_bytes:
        return value
    return encoded[:max_bytes].decode("utf-8", errors="replace") + "\n<truncated>"

