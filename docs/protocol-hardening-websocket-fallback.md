# Protocol Hardening And WebSocket Fallback

Phase 80 Task 01 moves the runner data-plane toward network-hardening without
changing the canonical message contract.

## Transport Order

The runner transport policy is:

1. gRPC is preferred.
2. WebSocket is used only when configured or when gRPC fails with a retryable
   network/proxy/TLS condition.
3. REST remains management/control only and is not a production execution
   transport.

The fallback implementation uses `loomex_core::transport::negotiate_transport`.
Permanent failures such as auth, version mismatch, or malformed messages do not
silently fall back to WebSocket.

## gRPC Hardening

`GrpcClientConfig` now carries explicit:

- connect timeout,
- request timeout,
- HTTP/2 keepalive interval,
- keepalive timeout,
- keepalive while idle,
- gzip send/accept compression,
- structured proxy diagnostics.

Tonic proxy support is still fail-fast for gRPC. When a proxy is required and
gRPC cannot be used directly, transport negotiation can select WebSocket if the
WebSocket endpoint is configured and available.

## WebSocket Contract

WebSocket fallback uses binary protobuf frames, not a JSON mirror. The first
byte is a direction tag:

- `1` = `RunnerToServer`
- `2` = `ServerToRunner`

The remaining bytes are the generated protobuf payload from
`proto/loomex/runner/v1/runner_stream.proto`. This keeps gRPC and WebSocket
contract-compatible and avoids schema drift.

## Flow Control

`FlowControlWindow` enforces a bounded in-flight byte window before large output
is emitted over either transport. Output chunking remains owned by
`StreamSupervisor`, and transport-level backpressure returns
`TRANSPORT_BACKPRESSURE`.

## Metrics

`TransportMetrics` tracks:

- reconnect count,
- fallback count,
- stream latency,
- message lag,
- accepted events,
- duplicate events,
- dropped or out-of-order events.

`StreamSupervisor` exposes transport metrics and records accepted server
messages plus reconnects.

## Remaining Server Work

The current backend workspace still exposes the legacy `runner_control`
long-poll compatibility path. A production server-side implementation still
needs:

- gRPC `RunnerDataPlane.Open` server endpoint,
- WebSocket endpoint that reads/writes the binary protobuf frames above,
- ingress/proxy WebSocket upgrade configuration,
- protocol conformance tests that run the same scenarios through both server
  transports.

No backend or infra files were changed for this task because those production
server endpoints are not present in the current codebase and infra is checked
out on `stage`.
