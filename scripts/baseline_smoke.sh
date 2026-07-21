#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER_BIN="${RUNNER_BIN:-loomex-runner}"
CONFIG_DIR="${LOOMEX_RUNNER_SMOKE_HOME:-$(mktemp -d "${TMPDIR:-/tmp}/loomex-runner-smoke.XXXXXX")}"
WORKSPACE_DIR="${LOOMEX_RUNNER_SMOKE_WORKSPACE:-${CONFIG_DIR}/workspace}"
CONFIG_PATH="${CONFIG_DIR}/config.toml"
SERVER="${LOOMEX_RUNNER_SMOKE_SERVER:-http://127.0.0.1:28000/api/v1/runner-control}"
HOST_HEADER="${LOOMEX_RUNNER_SMOKE_HOST_HEADER:-loomex.localhost}"
TOKEN="${LOOMEX_RUNNER_SMOKE_TOKEN:-}"
API_KEY="${LOOMEX_RUNNER_SMOKE_API_KEY:-}"
API_SECRET="${LOOMEX_RUNNER_SMOKE_API_SECRET:-}"
WORKFLOW_ID="${LOOMEX_RUNNER_SMOKE_WORKFLOW_ID:-}"
RUN_LIVE="${LOOMEX_RUNNER_SMOKE_LIVE:-0}"

usage() {
  cat <<'USAGE'
Usage:
  scripts/baseline_smoke.sh

Local-only checks run by default and validate the current Python spike command
surface without contacting Loomex.

Optional live e2e:
  LOOMEX_RUNNER_SMOKE_LIVE=1
  LOOMEX_RUNNER_SMOKE_WORKFLOW_ID=<uuid>
  plus either:
    LOOMEX_RUNNER_SMOKE_TOKEN=<lmxrt_...>
  or:
    LOOMEX_RUNNER_SMOKE_API_KEY=<wfpk_...>
    LOOMEX_RUNNER_SMOKE_API_SECRET=<wfsk_...>

Optional connection settings:
  LOOMEX_RUNNER_SMOKE_SERVER=http://127.0.0.1:28000/api/v1/runner-control
  LOOMEX_RUNNER_SMOKE_HOST_HEADER=loomex.localhost
  LOOMEX_RUNNER_SMOKE_WORKSPACE=/path/to/workspace
  RUNNER_BIN=loomex-runner
USAGE
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

export LOOMEX_RUNNER_CONFIG="${CONFIG_PATH}"
mkdir -p "${WORKSPACE_DIR}"

if ! command -v "${RUNNER_BIN}" >/dev/null 2>&1; then
  RUNNER_BIN=(python3 -m loomex_runner)
  export PYTHONPATH="${ROOT_DIR}/src${PYTHONPATH:+:${PYTHONPATH}}"
else
  RUNNER_BIN=("${RUNNER_BIN}")
fi

run_runner() {
  (cd "${ROOT_DIR}" && "${RUNNER_BIN[@]}" "$@")
}

echo "== local CLI surface =="
run_runner --help >/dev/null
run_runner login --help >/dev/null
run_runner run --help >/dev/null
run_runner connect --help >/dev/null
run_runner start --help >/dev/null
run_runner doctor --help >/dev/null
run_runner workspace add baseline "${WORKSPACE_DIR}" >/dev/null
run_runner doctor >/tmp/loomex-runner-doctor.json || true
python3 - <<'PY'
import json
from pathlib import Path

payload = json.loads(Path("/tmp/loomex-runner-doctor.json").read_text(encoding="utf-8"))
assert payload["profile"] == "default"
assert "baseline" in payload["workspaces"]
assert "tools" in payload and "python3" in payload["tools"]
PY

export WORKSPACE_DIR
python3 - <<'PY'
import os
from pathlib import Path
from loomex_runner.cli import parse_human_input, parse_workflow_inputs
from argparse import Namespace

workspace = Path(os.environ["WORKSPACE_DIR"])
workspace.joinpath("input.json").write_text('{"task":"ship"}', encoding="utf-8")
assert parse_workflow_inputs(Namespace(input='{"task":"ship"}', input_json="")) == {"task": "ship"}
assert parse_workflow_inputs(Namespace(input='["a","b"]', input_json="")) == {"value": ["a", "b"]}
assert parse_workflow_inputs(Namespace(input="true", input_json="")) == {"value": True}
assert parse_workflow_inputs(Namespace(input="42", input_json="")) == {"value": 42}
assert parse_workflow_inputs(Namespace(input="plain text", input_json="")) == {"value": "plain text"}
assert parse_human_input(Namespace(input="", input_json="", human_input='{"approved":true}', prompt="")) == {"approved": True}
assert parse_human_input(Namespace(input="", input_json="", human_input='["approve",true]', prompt="")) == {"value": ["approve", True]}
assert parse_human_input(Namespace(input="", input_json="", human_input="true", prompt="")) == {"value": True}
assert parse_human_input(Namespace(input="", input_json="", human_input="42", prompt="")) == {"value": 42}
assert parse_human_input(Namespace(input="", input_json="", human_input="plain text", prompt="")) == {"value": "plain text"}
PY

echo "== live runner-control e2e =="
if [[ "${RUN_LIVE}" != "1" ]]; then
  echo "skipped: set LOOMEX_RUNNER_SMOKE_LIVE=1 with credentials and workflow id"
  exit 0
fi

if [[ -z "${WORKFLOW_ID}" ]]; then
  echo "LOOMEX_RUNNER_SMOKE_WORKFLOW_ID is required for live smoke" >&2
  exit 2
fi

if [[ -n "${TOKEN}" ]]; then
  run_runner login --server "${SERVER}" --host-header "${HOST_HEADER}" --token "${TOKEN}"
elif [[ -n "${API_KEY}" && -n "${API_SECRET}" ]]; then
  run_runner login \
    --server "${SERVER}" \
    --host-header "${HOST_HEADER}" \
    --api-key "${API_KEY}" \
    --api-secret "${API_SECRET}" \
    --runner-name "Baseline smoke runner" \
    --workspace "${WORKSPACE_DIR}"
else
  echo "Provide LOOMEX_RUNNER_SMOKE_TOKEN or API key/secret for live smoke" >&2
  exit 2
fi

run_runner doctor
run_runner run \
  --workflow "${WORKFLOW_ID}" \
  --workspace baseline \
  --input '{"task":"baseline file write smoke"}' \
  --human-input '{"prompt":"complete the baseline smoke"}' \
  --timeout 900 \
  --human-timeout 120 \
  --poll-interval 0.5 \
  --non-interactive

echo "baseline smoke complete"
