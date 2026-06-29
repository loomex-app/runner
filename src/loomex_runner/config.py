from __future__ import annotations

import json
import os
import tomllib
from pathlib import Path
from typing import Any


DEFAULT_PROFILE = "default"
DEFAULT_CONFIG_PATH = Path(os.environ.get("LOOMEX_RUNNER_CONFIG") or "~/.loomex-runner/config.toml").expanduser()
LEGACY_CONFIG_PATH = Path("~/.loomex-runner/config.json").expanduser()
NEW_LOOMEX_CONFIG_PATH = Path(os.environ.get("LOOMEX_CONFIG") or "~/.loomex/config.toml").expanduser()


def empty_config() -> dict[str, Any]:
    return {
        "defaultProfile": DEFAULT_PROFILE,
        "profiles": {DEFAULT_PROFILE: {"server": "", "hostHeader": "", "token": ""}},
        "workspaces": {},
    }


def load_config(path: Path = DEFAULT_CONFIG_PATH) -> dict[str, Any]:
    if path.exists():
        data = tomllib.loads(path.read_text(encoding="utf-8"))
        return normalize_config(data if isinstance(data, dict) else {})
    if path == DEFAULT_CONFIG_PATH and LEGACY_CONFIG_PATH.exists():
        data = json.loads(LEGACY_CONFIG_PATH.read_text(encoding="utf-8"))
        return normalize_config(data if isinstance(data, dict) else {})
    return empty_config()


def save_config(config: dict[str, Any], path: Path = DEFAULT_CONFIG_PATH) -> None:
    normalized = normalize_config(config)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(render_toml(normalized), encoding="utf-8")
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass


def normalize_config(config: dict[str, Any]) -> dict[str, Any]:
    if "profiles" not in config:
        default_profile = str(config.get("defaultProfile") or DEFAULT_PROFILE)
        config = {
            "defaultProfile": default_profile,
            "profiles": {
                default_profile: {
                    "server": str(config.get("server") or ""),
                    "hostHeader": str(config.get("hostHeader") or ""),
                    "token": str(config.get("token") or ""),
                }
            },
            "workspaces": config.get("workspaces") if isinstance(config.get("workspaces"), dict) else {},
        }
    default_profile = str(config.get("defaultProfile") or DEFAULT_PROFILE)
    raw_profiles = config.get("profiles") if isinstance(config.get("profiles"), dict) else {}
    profiles: dict[str, dict[str, Any]] = {}
    for name, profile in raw_profiles.items():
        if not isinstance(profile, dict):
            continue
        profiles[str(name)] = {
            "server": str(profile.get("server") or "").rstrip("/"),
            "hostHeader": str(profile.get("hostHeader") or profile.get("host_header") or ""),
            "token": str(profile.get("token") or ""),
            "runnerId": str(profile.get("runnerId") or profile.get("runner_id") or ""),
            "organizationId": str(profile.get("organizationId") or profile.get("organization_id") or ""),
            "projectId": str(profile.get("projectId") or profile.get("project_id") or ""),
        }
    profiles.setdefault(DEFAULT_PROFILE, {"server": "", "hostHeader": "", "token": ""})
    if default_profile not in profiles:
        default_profile = DEFAULT_PROFILE
    workspaces = {
        str(name): str(value)
        for name, value in (config.get("workspaces") if isinstance(config.get("workspaces"), dict) else {}).items()
    }
    return {"defaultProfile": default_profile, "profiles": profiles, "workspaces": workspaces}


def render_toml(config: dict[str, Any]) -> str:
    lines = [f"defaultProfile = {toml_string(config['defaultProfile'])}", ""]
    for name, profile in sorted(config["profiles"].items()):
        lines.append(f"[profiles.{toml_key(name)}]")
        for key in ("server", "hostHeader", "token", "runnerId", "organizationId", "projectId"):
            value = str(profile.get(key) or "")
            if value:
                lines.append(f"{key} = {toml_string(value)}")
        lines.append("")
    lines.append("[workspaces]")
    for name, value in sorted(config.get("workspaces", {}).items()):
        lines.append(f"{toml_key(name)} = {toml_string(value)}")
    lines.append("")
    return "\n".join(lines)


def toml_key(value: str) -> str:
    return json.dumps(str(value))


def toml_string(value: str) -> str:
    return json.dumps(str(value))


def profile_settings(config: dict[str, Any], profile: str = "") -> dict[str, Any]:
    normalized = normalize_config(config)
    profile_name = profile or str(normalized.get("defaultProfile") or DEFAULT_PROFILE)
    profiles = normalized.get("profiles") if isinstance(normalized.get("profiles"), dict) else {}
    settings = profiles.get(profile_name) or profiles.get(DEFAULT_PROFILE) or {}
    return dict(settings)


def set_login(
    *,
    server: str,
    token: str,
    host_header: str = "",
    profile: str = DEFAULT_PROFILE,
    runner_id: str = "",
    organization_id: str = "",
    project_id: str = "",
    path: Path = DEFAULT_CONFIG_PATH,
) -> dict[str, Any]:
    config = load_config(path)
    profile_name = profile or str(config.get("defaultProfile") or DEFAULT_PROFILE)
    profiles = dict(config.get("profiles") or {})
    current = dict(profiles.get(profile_name) or {})
    current.update(
        {
            "server": server.rstrip("/"),
            "token": token.strip(),
            "hostHeader": host_header.strip(),
        }
    )
    if runner_id:
        current["runnerId"] = runner_id
    if organization_id:
        current["organizationId"] = organization_id
    if project_id:
        current["projectId"] = project_id
    profiles[profile_name] = current
    config["profiles"] = profiles
    config["defaultProfile"] = profile_name
    config.setdefault("workspaces", {})
    save_config(config, path)
    return normalize_config(config)


def add_workspace(*, name: str, workspace_path: str, path: Path = DEFAULT_CONFIG_PATH) -> dict[str, Any]:
    workspace = Path(workspace_path).expanduser().resolve()
    workspace.mkdir(parents=True, exist_ok=True)
    config = load_config(path)
    workspaces = dict(config.get("workspaces") or {})
    workspaces[name] = str(workspace)
    config["workspaces"] = workspaces
    save_config(config, path)
    return normalize_config(config)


def resolve_workspace(config: dict[str, Any], value: str) -> Path:
    workspaces = config.get("workspaces") if isinstance(config.get("workspaces"), dict) else {}
    raw = workspaces.get(value, value)
    return Path(str(raw)).expanduser().resolve()


def config_migration_status(
    *,
    legacy_runner_path: Path = DEFAULT_CONFIG_PATH,
    new_loomex_path: Path = NEW_LOOMEX_CONFIG_PATH,
) -> dict[str, Any]:
    legacy_exists = legacy_runner_path.exists()
    new_exists = new_loomex_path.exists()
    if legacy_exists and new_exists:
        status = "new_config_preferred"
        action = (
            "Keep using ~/.loomex/config.toml for production `loomex`; "
            "keep ~/.loomex-runner/config.toml only for smoke compatibility."
        )
    elif legacy_exists:
        status = "legacy_config_only"
        action = (
            "Install `loomex`, run `loomex login`, then recreate bindings with "
            "`loomex bind`; do not copy tokens into the new config."
        )
    elif new_exists:
        status = "new_config_only"
        action = "Use `loomex`; `loomex-runner` has no legacy config to migrate."
    else:
        status = "no_config"
        action = (
            "Run `loomex login` for production CLI or `loomex-runner login` "
            "only for compatibility smoke."
        )
    return {
        "status": status,
        "legacyRunnerConfig": str(legacy_runner_path),
        "newLoomexConfig": str(new_loomex_path),
        "legacyRunnerConfigExists": legacy_exists,
        "newLoomexConfigExists": new_exists,
        "action": action,
    }
