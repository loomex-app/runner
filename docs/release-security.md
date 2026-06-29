# Release Security

Loomex Runner releases are distributed through an explicit trust chain:

1. Build CI creates CLI binaries and app installers for each OS/architecture.
2. Each artifact is hashed with SHA-256 and signed with the release Ed25519 key.
3. CI emits a signed release manifest containing artifacts, channel, rollout,
   rollback metadata, SBOM packages, and build provenance.
4. Installers and update clients verify the manifest signature before trusting
   any artifact metadata, URL, checksum, or provenance.
5. The artifact checksum, size, and signature are verified before install.

The release manifest schema is `loomex.runner.releaseManifest/v1`. The helper
commands are:

```bash
loomex runner release sign-artifact \
  --name loomex-cli-macos-aarch64 \
  --os macos \
  --arch aarch64 \
  --path target/release/loomex \
  --signing-key-env LOOMEX_RELEASE_SIGNING_KEY

loomex runner release sign-manifest \
  --manifest release-manifest.json \
  --signing-key-env LOOMEX_RELEASE_SIGNING_KEY

loomex runner release verify-manifest \
  --manifest release-manifest.signed.json \
  --public-key "$LOOMEX_RELEASE_PUBLIC_KEY"

loomex runner release verify-artifact \
  --manifest release-manifest.signed.json \
  --name loomex-cli-macos-aarch64 \
  --path target/release/loomex \
  --public-key "$LOOMEX_RELEASE_PUBLIC_KEY"
```

Do not pass release signing keys as command-line values. `--signing-key` is
rejected because argv can leak through shell history and process inspection. Use
`--signing-key-env`, `--signing-key-file`, or `--signing-key-stdin`.

## Channels

- `stable`: default production channel.
- `beta`: opt-in pre-release channel.
- `nightly_internal`: internal builds and local dogfood only.
- `enterprise_pinned`: pinned customer-controlled channel; a pinned version does
  not auto-upgrade unless the enterprise policy changes.

Rollouts are staged through `rollout_percent` and a deterministic client
rollout bucket. Rollback requires a signed manifest with `rollback_to_version`
listed in `previous_versions`; normal downgrade attempts are rejected.

## macOS Signing And Notarization

The packaging smoke script still performs local app assembly and internal
signing. Public distribution requires Developer ID signing, hardened runtime,
notarization, and staple verification before the DMG is published. A
notarization delay or failure blocks public channel promotion; the previous
signed manifest remains the active stable manifest.

## Windows And Linux

Windows artifacts use the same SHA-256 and Ed25519 artifact signature contract.
SmartScreen reputation is treated as a rollout gate: a signed artifact may stay
in beta until reputation is established. Linux packages publish the same
manifest, checksum, and signature records beside the package artifact.

## Key Rotation And Compromise

Release public keys are versioned by channel. Rotation publishes a transition
manifest signed by the old key and the new key. A suspected key compromise
freezes rollout, removes the compromised public key from update policy, and
requires a manual enterprise verification path for the next trusted build.

## Offline And Partial Downloads

Offline clients keep their current verified version and retry later. Partial
downloads are discarded because artifact verification requires the exact
manifest `size_bytes`, SHA-256 digest, and signature before install.

## SBOM And Provenance

Every signed manifest carries:

- sorted SBOM package entries,
- build system identity,
- source repository and revision,
- build start/finish timestamps,
- CI workflow run id.

Enterprise customers can verify a binary by checking the manifest signature,
artifact checksum/signature, SBOM, and provenance fields together.

## Installers And Compatibility

The official installer/channel matrix is documented in
[release-channels-installers.md](release-channels-installers.md) and exposed as
machine-readable JSON:

```bash
loomex runner release installer-plan --json
loomex runner release validate-compatibility --matrix compatibility.json
```

Installer uninstall paths must remove binaries/app bundles/service units without
silently deleting user config, credentials, audit logs, support bundles, or local
trace data.
