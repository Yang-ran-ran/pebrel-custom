# Manual Windows custom build artifacts

## Status

Accepted as a maintainer/owner-triggered workflow for custom source builds.

## Context

Users who consume a customized Pebrel source tree may not have Rust, Cargo, the MSVC toolchain, or Inno Setup installed locally. The repository already owns the authoritative Windows product build and package scripts, but the stable release workflow also validates release metadata and can publish public releases.

## Evidence

- `scripts/build-windows-product.ps1` builds the GPUI app, CLI, and hook using the pinned feature graph.
- `scripts/package-release.ps1` creates the portable ZIP and verifies the bundled executable is the GPUI product.
- `scripts/build-installer.ps1` creates the Windows installer and validates the application version and runtime files.
- `INSTALL.md` documents the Windows x64 portable ZIP and installer formats.

## Decision

Add a separate `workflow_dispatch` workflow that checks out the selected branch/commit, uses a Windows x64 runner and the pinned Rust toolchain, prepares the pinned ConPTY runtime, builds the GPUI product, and runs the existing package scripts. Upload the portable ZIP and setup EXE as a short-lived Actions artifact. Do not create a GitHub Release, move tags, or bypass the normal stable release validation.

## Rejected alternatives

- Asking users to install Rust/Cargo and the native Windows toolchain: this defeats the goal of a directly runnable Windows package.
- Reusing the stable release workflow: that path is coupled to stable version metadata and publication gates, while this is a private/custom build artifact.
- Renaming an unrelated upstream binary: it would not contain the user's source changes.

## Consequences

The user still needs to put the modified source on a GitHub branch and manually start the workflow. The generated artifact is downloadable from that workflow run for 14 days. The portable ZIP runs without a separate Rust installation; the installer EXE is an optional install path. The artifact is not code-signed by this workflow and Windows SmartScreen may display an unfamiliar-app warning.

## Validation

Workflow syntax and source scripts must be checked in CI. The actual Windows build, packaging checks, and launch/conformance validation run on the Windows Actions runner; they have not run in the Linux editing environment.

## Supersedes

None.

## Revisit when

The project adopts a supported self-hosted Windows build runner or a dedicated private artifact service with a longer retention policy.
