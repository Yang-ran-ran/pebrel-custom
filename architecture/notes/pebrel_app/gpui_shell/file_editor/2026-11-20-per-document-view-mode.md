# Per-document Markdown view mode

## Status

Accepted for the GPUI document editor.

## Context

Markdown source and reading mode share one `TextFileView` for local files, SSH/SFTP files, and Markdown opened through code/document tabs. A global toggle would make unrelated documents unexpectedly change presentation. A view-local toggle without persistence would also lose the user's choice when a tab is closed.

## Evidence

- `gpui_shell/file_editor/chrome.rs` owns the compact source/reading control.
- `gpui_shell/workspace/documents.rs` and `gpui_shell/code_tab.rs` route document tabs through `TextFileView`.
- `DocumentSource` distinguishes local paths from SSH destination/path identities.

## Decision

Keep source mode as the default. Persist a reading/source boolean per document identity in `document-view-modes.json` under Pebrel's existing user data directory. Hash the local canonical path or SSH destination/path before writing it, so the preference store does not expose document names or paths. Reuse the same control for local and remote Markdown; remote previews do not resolve image paths against the local filesystem.

## Rejected alternatives

- A global mode preference: unrelated documents would change together.
- A preference stored only in a live tab: closing and reopening a document would lose the selection.
- Plain-text path keys: the settings file would reveal local and remote document names.

## Consequences

The preference is independent for each local/remote document and is shared by all UI routes that construct the same `TextFileView`. Missing or unreadable state safely falls back to source mode. The state file is small and atomically replaced; a brief cross-process write conflict is reported rather than blocking the UI.

## Validation

Unit tests cover round-trip persistence, isolation between document identities, and hashed identities. GPUI regression coverage asserts source mode by default and the compact control's presence. Full native GPUI execution remains required before release.

## Supersedes

None.

## Revisit when

A second independent Markdown viewer is introduced that does not use `TextFileView`, or document identity semantics change for remote connections.
