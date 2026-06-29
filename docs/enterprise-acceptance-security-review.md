# Enterprise Acceptance And Security Review

This document defines the customer-grade acceptance, security review, and
compliance package required before promoting Loomex Runner to an enterprise
release candidate.

The machine-readable plan is available with:

```bash
loomex runner ops enterprise-plan --json
```

The sign-off evaluator consumes an enterprise acceptance report:

```bash
loomex runner ops enterprise-signoff --report enterprise-acceptance-report.json --json
```

When sign-off is blocked, the CLI preserves the
`loomex.cli.enterpriseAcceptanceSignoff/v1` JSON shape and exits with code `41`.

## Enterprise Acceptance Checklist

Every release candidate must provide evidence for:

- install
- login
- bind
- policy enforcement
- approval flow
- local AI workflow
- Playwright, local HTTP, and DB capability test
- audit export
- support bundle
- update and rollback
- full enterprise acceptance on a clean organization
- policy bypass attempts
- workspace escape attempts
- secret leakage scan
- update tamper test
- audit export completeness
- retention and legal hold behavior
- data classification review

Missing, duplicate, or unknown checklist IDs are invalid and fail closed. Each
scenario result must include exactly the evidence keys listed by
`enterprise-plan`; generic evidence such as `["evidence"]`, missing keys, extra
keys, or duplicate keys are rejected before sign-off.

## Security Review Scope

Security review scope is fixed for release candidates:

- runner auth
- protocol
- policy bypass
- workspace sandbox
- artifact leakage
- update chain

Critical and high findings must be zero open. Medium findings may remain open
only when mitigation, owner, and target date are documented.

The enterprise acceptance report must also include `securityReviewResults` for
every scope item. Each result must include the exact required test evidence from
the plan, such as `token_expiry` and `revocation` for runner auth, or
`manifest_signature` and `artifact_signature` for update chain. Missing,
duplicate, or unknown security review evidence fails closed.

## Compliance Package

Customer-facing compliance evidence must include:

- data flow
- retention
- audit fields
- admin controls
- security FAQ
- data classification
- legal hold behavior

Each compliance item must be reviewed and include exactly the required evidence
keys before sign-off can pass. For example, `data_flow` must include both
`diagram` and `processor_notes`; one without the other is rejected.

## Blocking Thresholds

The sign-off gate blocks release when:

- any enterprise acceptance scenario fails or lacks evidence
- any enterprise acceptance scenario has missing, duplicate, or unknown evidence
- any security review scope is missing, fails, or lacks exact required evidence
- any open critical or high security finding exists
- any open medium finding lacks mitigation, owner, or target date
- any compliance item is unreviewed or lacks exact required evidence
- load/chaos acceptance fails
- reconnect recovery p95 exceeds 30 seconds
- transport fallback is not verified
- supported runner version compatibility is missing or failed

These thresholds are intentionally numeric and auditable so a release candidate
is accepted or rejected by evidence rather than narrative status.
