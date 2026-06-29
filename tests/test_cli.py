from __future__ import annotations

import argparse

from loomex_runner.cli import (
    legacy_warning_message,
    parse_human_input,
    parse_json_argument,
    parse_workflow_inputs,
    workflow_succeeded,
)
from loomex_runner.config import (
    config_migration_status,
    load_config,
    profile_settings,
    render_toml,
    set_login,
)


def test_parse_human_input_keeps_legacy_plain_input_as_prompt():
    args = argparse.Namespace(input="create README", input_json="")

    assert parse_human_input(args) == {"prompt": "create README"}


def test_parse_human_input_accepts_json_object():
    args = argparse.Namespace(input="", input_json='{"prompt":"create README","priority":"high"}')

    assert parse_human_input(args) == {"prompt": "create README", "priority": "high"}


def test_parse_human_input_accepts_human_input_json_object():
    args = argparse.Namespace(input="", input_json="", human_input='{"approved":true,"notes":"ship"}', prompt="")

    assert parse_human_input(args) == {"approved": True, "notes": "ship"}


def test_parse_human_input_accepts_json_array_as_value():
    args = argparse.Namespace(input="", input_json="", human_input='["approve", true]', prompt="")

    assert parse_human_input(args) == {"value": ["approve", True]}


def test_parse_human_input_accepts_bool_text_and_number_as_value():
    assert parse_human_input(argparse.Namespace(input="", input_json="", human_input="true", prompt="")) == {
        "value": True
    }
    assert parse_human_input(argparse.Namespace(input="", input_json="", human_input="42", prompt="")) == {"value": 42}
    assert parse_human_input(argparse.Namespace(input="", input_json="", human_input="plain text", prompt="")) == {
        "value": "plain text"
    }


def test_parse_workflow_inputs_accepts_json_object_array_bool_text_and_number():
    assert parse_workflow_inputs(argparse.Namespace(input='{"task":"ship"}', input_json="")) == {"task": "ship"}
    assert parse_workflow_inputs(argparse.Namespace(input='["a","b"]', input_json="")) == {"value": ["a", "b"]}
    assert parse_workflow_inputs(argparse.Namespace(input="true", input_json="")) == {"value": True}
    assert parse_workflow_inputs(argparse.Namespace(input="42", input_json="")) == {"value": 42}
    assert parse_workflow_inputs(argparse.Namespace(input="plain text", input_json="")) == {"value": "plain text"}


def test_parse_json_argument_reads_file(tmp_path):
    payload = tmp_path / "input.json"
    payload.write_text('{"task":"from file"}', encoding="utf-8")

    assert parse_json_argument(f"@{payload}", "--input") == {"task": "from file"}


def test_toml_config_round_trip_profiles(tmp_path):
    path = tmp_path / "config.toml"

    set_login(
        server="http://127.0.0.1:28080/api/v1/runner-control",
        token="lmxrt_test",
        host_header="loomex.localhost",
        profile="local",
        runner_id="runner-1",
        organization_id="org-1",
        project_id="project-1",
        path=path,
    )

    config = load_config(path)
    settings = profile_settings(config, "local")
    assert config["defaultProfile"] == "local"
    assert settings["token"] == "lmxrt_test"
    assert settings["hostHeader"] == "loomex.localhost"
    assert settings["runnerId"] == "runner-1"


def test_legacy_token_config_round_trip_still_works_until_removal(tmp_path):
    path = tmp_path / ".loomex-runner" / "config.toml"

    set_login(
        server="http://127.0.0.1:28080/api/v1/runner-control",
        token="lmxrt_legacy_token",
        host_header="loomex.localhost",
        profile="default",
        path=path,
    )

    settings = profile_settings(load_config(path), "default")
    assert settings["token"] == "lmxrt_legacy_token"
    assert settings["server"] == "http://127.0.0.1:28080/api/v1/runner-control"


def test_toml_config_snapshot_matches_current_runner_profile_shape():
    config = {
        "defaultProfile": "local",
        "profiles": {
            "local": {
                "server": "http://127.0.0.1:28080/api/v1/runner-control",
                "hostHeader": "loomex.localhost",
                "token": "lmxrt_test",
                "runnerId": "runner-1",
                "organizationId": "org-1",
                "projectId": "project-1",
            }
        },
        "workspaces": {"demo": "/srv/my-app"},
    }

    assert render_toml(config) == (
        'defaultProfile = "local"\n'
        "\n"
        '[profiles."local"]\n'
        'server = "http://127.0.0.1:28080/api/v1/runner-control"\n'
        'hostHeader = "loomex.localhost"\n'
        'token = "lmxrt_test"\n'
        'runnerId = "runner-1"\n'
        'organizationId = "org-1"\n'
        'projectId = "project-1"\n'
        "\n"
        "[workspaces]\n"
        '"demo" = "/srv/my-app"\n'
    )


def test_legacy_deprecation_warning_snapshot_maps_commands_without_secrets():
    warning = legacy_warning_message("run")

    assert warning == (
        "DEPRECATION: `loomex-runner` is a legacy Python spike compatibility command; "
        "use `loomex workflow run WORKFLOW_ID` for the production CLI path. Compatibility window: "
        "dev-smoke compatibility until Rust/gRPC `loomex` runner passes Phase 50 acceptance, "
        "then one stable `loomex` release. REST long-poll removal target: after Phase 80 migration sign-off. "
        "This warning never prints token or secret values."
    )
    assert "lmxrt_" not in warning
    assert "wfsk_" not in warning


def test_config_migration_status_prefers_new_loomex_config_when_both_exist(tmp_path):
    legacy_path = tmp_path / ".loomex-runner" / "config.toml"
    new_path = tmp_path / ".loomex" / "config.toml"
    legacy_path.parent.mkdir()
    new_path.parent.mkdir()
    legacy_path.write_text('defaultProfile = "default"\n', encoding="utf-8")
    new_path.write_text('defaultProfile = "default"\n', encoding="utf-8")

    status = config_migration_status(legacy_runner_path=legacy_path, new_loomex_path=new_path)

    assert status["status"] == "new_config_preferred"
    assert status["legacyRunnerConfigExists"] is True
    assert status["newLoomexConfigExists"] is True
    assert "do not copy tokens" not in status["action"].lower()


def test_config_migration_status_guides_legacy_only_without_printing_tokens(tmp_path):
    legacy_path = tmp_path / ".loomex-runner" / "config.toml"
    new_path = tmp_path / ".loomex" / "config.toml"
    legacy_path.parent.mkdir()
    legacy_path.write_text('token = "lmxrt_secret"\n', encoding="utf-8")

    status = config_migration_status(legacy_runner_path=legacy_path, new_loomex_path=new_path)

    assert status["status"] == "legacy_config_only"
    assert "do not copy tokens" in status["action"].lower()
    assert "lmxrt_secret" not in str(status)


def test_workflow_succeeded_accepts_panel_and_runtime_success_statuses():
    assert workflow_succeeded({"status": "succeeded"}) is True
    assert workflow_succeeded({"status": "completed"}) is True
    assert workflow_succeeded({"status": "failed"}) is False
