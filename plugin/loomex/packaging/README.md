# Native package assembly

The repository stores no pretend executable and no made-up signature. A release
job must build the real `loomex-mcp` adapter and matching `loomex` Runner for
every macOS/Linux entry in `targets.json`, apply the platform's production
signing/notarization process, and provide both artifacts to the assembler.

After the signed artifacts are present, `codex_plugin_assemble.py` copies both
binaries into each target directory and creates `runtime-manifest.json` from the
schema documented by `runtime-manifest.template.json`. The generated manifest
sets `pluginVersion`, file sizes, lower-case SHA-256 digests of the final signed
bytes, and signing evidence. It then runs:

```bash
node scripts/validate-package.mjs --release
```

Only the assembled release archive contains `bin/` and
`packaging/runtime-manifest.json`. Source validation deliberately succeeds
without them. At launch, the adapter rejects symlinks, unsupported targets,
missing executability, path escapes, absent manifests, and digest mismatches.

The release archive is one install for users: it includes every supported
macOS/Linux MCP adapter and Runner pair. The first `loomex_setup_apply` installs
the durable per-user Runner from the verified artifact for the current target;
it does not ask the user to obtain a second installer.

Assembly also creates `loomex-codex-marketplace-<version>.zip` with the Codex
marketplace layout `.agents/plugins/marketplace.json` plus
`plugins/loomex/**`, and a companion
`loomex-codex-marketplace-<version>.provenance.json`. The assembler computes the
canonical Git tree and orphan commit IDs directly from the packaged bytes with
a fixed release identity, timestamp, and message. The provenance binds those
IDs to the plugin version, repository, archive name, and archive SHA-256.

Production keyless-signs the archives and provenance with Cosign using GitHub's
OIDC token and publishes Sigstore bundles. Verification pins both
`https://token.actions.githubusercontent.com` and the exact
`loomex-app/runner/.github/workflows/codex-plugin-release.yml@refs/tags/v<version>`
workflow identity; a public key shipped beside mutable release assets is not a
trust anchor. The publication job reconstructs the orphan commit, verifies it
equals the authenticated exact commit, and refuses to replace an existing
publication ref. The version-named branch is only a discoverability pointer:
normal installation must verify provenance first and then pass the exact
40-character `marketplace.commit` to Codex's `--ref`. The source-only plugin
directory is never published as an installable marketplace entry.

GitHub Release assets are uploaded only after that exact marketplace commit is
verified and published. Release publication disables asset overwrite and fails
closed if any same-version artifact name already exists, so a rerun or reused
tag cannot replace previously advertised bytes before the commit guard runs.
Both external-write jobs also resolve the live remote `v<version>` tag
immediately before pushing the marketplace ref or publishing assets and require
it to peel to the release-gate SHA. Configure a repository tag ruleset that
forbids updating or deleting `v*` tags and enable immutable GitHub Releases;
the runtime checks remain mandatory even when those platform protections are
enabled. A deleted, recreated, or moved tag fails closed.

The current Codex Git-marketplace interface installs an exact Git SHA-1 commit;
it does not expose a client-side hook for comparing the cloned worktree with the
signed marketplace ZIP's SHA-256. Release CI therefore proves that the signed
ZIP, canonical Git tree, and exact commit are equivalent before publication,
and the online installer verifies signed provenance before passing only that
commit to Codex. The offline path verifies and installs the signed ZIP itself.
If Codex adds a persistent verified-snapshot or SHA-256-object interface, the
online installer should migrate to it.

Keyless signing has no Loomex private signing key to rotate. Sigstore root
rotation is consumed only through a reviewed update of the pinned Cosign
installer/action SHA. A repository or workflow-path change alters the OIDC
identity and therefore requires a reviewed, simultaneous workflow and install
documentation update. A compromised release is never “fixed” by moving its
commit ref: withdraw the release, publish a security advisory and a new signed
version, and keep the old exact commit identifiable for audit and revocation
notices.

Linux binaries are built on Ubuntu 22.04 and the release job rejects symbols
newer than GLIBC 2.35. ARM64 binaries are executed under QEMU before packaging.
macOS signing evidence records the target, Team ID, notarization acceptance,
CDHash, and SHA-256 of both transferred native files; assembly rejects evidence
that does not match the bytes.

## Production release gate

Unsigned validation runs for pull requests and non-production manual runs. A
`v*` tag or a manual run with `production_release` enabled does not receive any
release credential merely because it requested production mode. The workflow
first resolves the requested SHA to a commit and verifies all of the following:

- the commit has exactly two parents;
- it is the exact `merge_commit_sha` of one merged GitHub pull request whose
  base is `stage` or `main`;
- its subject has GitHub's standard `Merge pull request #... from ...` form;
- it is still reachable from the matching remote base branch; and
- the merge commit's second parent exactly matches the pull request head; and
- after reading every review page, at least one non-author reviewer's latest
  decisive review is `APPROVED` for that exact head commit. An approval for an
  older head, or one followed by a dismissal or change request, is rejected.

The same secret-free provenance job queries the GitHub Environments API and
fails closed unless `codex-plugin-production` has at least one required
reviewer, prevents self-review, disables administrator bypass
(`can_admins_bypass` must be exactly `false`), and has custom deployment
policies for the `stage` and `main` branches and `v*` tags. Missing, null,
truthy, or non-boolean administrator-bypass state fails closed. Those three
deployment-policy entries are an exact allowlist: malformed,
wildcard-broadening, or additional branch/tag policies fail the gate. The gate
reads every deployment-policy API page and requires the collected count to
equal one consistent, non-negative `total_count`; truncated or inconsistent
pagination fails closed. Only after those checks succeed may the workflow enter
that protected Environment. The Apple signing and notarization jobs and the
Cosign archive-signing job all reference it. A reviewer must verify the
successful provenance job before approving the deployment.

The required-reviewers rule is schema-checked rather than treated as truthy:
there must be exactly one such rule with one to six unique `User` or `Team`
entries, each containing a reviewer object with a positive integer ID. Null,
empty, boolean-ID, missing-payload, duplicate, unknown-type, and malformed extra
protection-rule entries fail closed.

The checkout deliberately uses `persist-credentials: false`. Reachability from
`stage` or `main` is proven with the authenticated GitHub Compare API using the
job's short-lived, read-only `GITHUB_TOKEN`; the gate performs no network
`git fetch`, writes no HTTP authorization header or credential helper into Git
configuration, and fails closed if the private-repository API check cannot be
completed.

Store these values as **environment secrets on
`codex-plugin-production` only**, never as repository or organization secrets:

- `MACOS_CERTIFICATE_P12_BASE64`, `MACOS_CERTIFICATE_PASSWORD`,
  `MACOS_SIGNING_IDENTITY`, and `MACOS_KEYCHAIN_PASSWORD`;
- `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, and `APPLE_TEAM_ID`.

This placement is part of the security boundary: validation and provenance
jobs cannot read production signing material. Missing environment protection,
required reviewers, deployment-ref rules, or environment-scoped Apple secrets
makes the release configuration incomplete and must block a production
release. Cosign uses the protected job's short-lived GitHub OIDC token rather
than a stored Loomex signing key. The macOS job deletes the imported certificate
and temporary keychain in an `always()` cleanup step, including when signing,
notarization, or evidence generation fails.
