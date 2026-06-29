# Operational Readiness, SLO, And Runbooks

This document is the runner operational contract for enterprise release gates.
It mirrors Phase 100 Task 02 and the measurable enterprise criteria plan.

## SLOs

The official runner SLO set is emitted by:

```bash
loomex runner ops readiness-plan --json
```

The required SLOs are:

| SLO | Target |
| --- | --- |
| Runner stream connect success | 99.5% over 30 days |
| Workflow local dispatch latency | p95 under 2 seconds |
| Stream reconnect recovery | p95 under 30 seconds |
| Approval delivery latency | p95 under 3 seconds |
| Trace upload success | 99.9% over 30 days |
| Run validation API availability | 99.9% over 30 days |

## Metrics

Release readiness requires these metrics to be present in dashboards and alert
evaluation:

- `runner_active_total`
- `runner_disconnected_total`
- `runner_stream_connect_total`
- `local_tool_call_total`
- `workflow_local_dispatch_latency_ms`
- `stream_reconnect_recovery_ms`
- `approval_delivery_latency_ms`
- `policy_denied_total`
- `approval_timeout_total`
- `transport_fallback_total`
- `trace_upload_total`
- `run_validation_api_total`

The Rust core exposes `OperationalMetricsRecorder` for deterministic local
tests around connect/disconnect, local tool success/failure, policy denies,
approval timeouts, transport fallback, and trace upload.

## Dashboards

The machine-readable readiness plan defines the canonical dashboard query IDs:

- `runner_fleet_health`
- `local_execution`
- `policy_approval`
- `transport`
- `trace_validation`

Each dashboard must have a query that can be executed by the monitoring backend
before enterprise promotion.

## Alerts

The official alert rule IDs are:

- `stream_failure_threshold`
- `error_budget_burn_7d`
- `policy_deny_spike`
- `approval_timeout_spike`
- `trace_upload_failure`

The release gate blocks risky rollout when 7-day burn exceeds 50% of the
monthly error budget, when any SLO result fails, or when security validation
checks are not clean.

## Runbooks

The required runbooks are:

- `grpc_outage`
- `auth_outage`
- `policy_misconfiguration`
- `update_rollout_failure`
- `trace_storage_growth`
- `support_escalation`

Runbook drills must attach evidence for the success criteria in the readiness
plan. For example, the gRPC outage drill requires evidence that ingress failure
was detected, fallback or reconnect was validated, and customer impact was
assessed.

## Capacity Planning

The official capacity plan assumes a reconnect-storm multiplier of `1.5`.
Recommended pods are calculated as:

```text
ceil(expected_runner_connections * 1.5 / max_streams_per_pod)
```

The default readiness plan uses 10,000 expected runner connections and 2,000
streams per pod. Override expected runners with:

```bash
loomex runner ops readiness-plan --expected-runners 25000 --json
```

## Release Gate

The release gate consumes an `OperationalReadinessReport` JSON document:

```bash
loomex runner ops release-gate --report operational-readiness-report.json --json
```

When `decision.allowed` is false, the CLI preserves the
`loomex.cli.operationalReleaseGate/v1` JSON shape and exits with code `40` so
release automation can block promotion while still uploading the structured
decision.

Reports must include exactly one SLO result and exactly one 7-day error-budget
burn entry for every official SLO ID. Missing, duplicate, or unknown SLO IDs are
invalid and fail closed before promotion can proceed.
