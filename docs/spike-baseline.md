# Loomex Runner Spike Baseline

This document freezes the current Python `loomex-runner` and backend
`runner-control` behavior as reference evidence for the Rust/gRPC rewrite.
The Python runner is a spike and compatibility reference only. It is not the
production target; the production command will be `loomex`.

## Current CLI Surface

The current executable is `loomex-runner`.

| Command | Current purpose | Baseline validation |
| --- | --- | --- |
| `loomex-runner login` | Stores a runner token from either API key exchange or an explicit legacy token. | `tests/test_cli.py::test_toml_config_snapshot_matches_current_runner_profile_shape` |
| `loomex-runner run` | Creates a session, starts one workflow, runs local jobs, and resolves human input. | `scripts/baseline_smoke.sh` |
| `loomex-runner connect` | Alias for `run`. | `scripts/baseline_smoke.sh` |
| `loomex-runner start` | Starts long-poll job execution for a registered workspace. | `scripts/baseline_smoke.sh` |
| `loomex-runner doctor` | Prints local config and tool diagnostics. | `scripts/baseline_smoke.sh` |
| no command | Starts the same interactive flow as `run`. | Manual baseline; requires a TTY. |

## Current Runner-Control Surface

The spike uses REST long polling under `/api/v1/runner-control/runner/v1/`.

| Endpoint family | Current purpose | Baseline validation |
| --- | --- | --- |
| `POST /auth/exchange/` | Exchange organization API credentials for a scoped runner token. | Backend `test_runner_api_exchanges_organization_api_key_for_runner_token` |
| `POST /sessions/` and `POST /sessions/{id}/heartbeat/` | Register and heartbeat a runner session with a local manifest. | Backend `test_runner_api_leases_and_completes_command_job` |
| `POST /jobs/lease/` | Lease pending local jobs for a session. | Backend `test_runner_api_leases_and_completes_command_job` |
| `POST /jobs/{id}/start/` | Mark a leased job as started. | Backend `test_runner_api_leases_and_completes_command_job` |
| `POST /jobs/{id}/events/` | Append stdout/stderr/progress events for audit. | Backend `test_runner_api_leases_and_completes_command_job` |
| `POST /jobs/{id}/complete/` and `POST /jobs/{id}/fail/` | Finish a local job with result or error payload. | Backend `test_runner_api_leases_and_completes_command_job` |
| `GET /workflows/` and `GET /workflows/{id}/` | List active workflows and expose input/human-input metadata. | Backend `test_runner_api_lists_and_describes_active_workflows` |
| `POST /workflows/{id}/executions/` | Start a workflow using runner session scope and JSON inputs. | Backend `test_runner_api_preserves_json_workflow_inputs` |
| `GET /executions/{id}/` | Poll workflow execution status. | Backend `test_runner_api_starts_workflow_execution_with_runner_scope` |
| `GET /executions/{id}/human-requests/` | Find pending human requests, including child subworkflow requests. | Backend `test_runner_api_lists_child_subworkflow_human_requests_from_root_execution` |
| `POST /human-requests/{id}/resolve/` | Resolve pending human input from the runner. | Backend `test_runner_api_resolves_human_request_with_non_object_answer` |

## Current Local Capabilities

The Python spike exposes these job kinds:

| Job kind | Behavior kept as reference |
| --- | --- |
| `command.run` | Run a bounded shell/list command inside the configured workspace. |
| `file.list` | List files under the workspace with a limit. |
| `file.read_many` | Read selected UTF-8 files under the workspace. |
| `file.write_many` | Write text or base64 file content under the workspace. |

Filesystem paths must remain inside the workspace. Path traversal is part of
the frozen regression surface and is covered by
`tests/test_jobs.py::test_file_write_many_rejects_traversal`.

## Current Workflow Baselines

The spike evidence currently covers:

- API key login.
- One-shot workflow execution.
- No-option interactive run as the default command path.
- Local file write within a workspace.
- JSON workflow input for object, array, boolean, text, and number values.
- Human input for object, array, boolean, text, and number values.
- Subworkflow child human requests surfaced from the root execution.

The node set validated by the runner product spike is:

- condition
- switch
- git tool
- http request tool
- ai agent
- person
- subworkflow with human input

Run `scripts/baseline_smoke.sh --help` for the exact repeatable smoke command
contract. The script intentionally requires live credentials and a workflow id
for networked e2e execution; without those, it runs local CLI regression checks
only.

