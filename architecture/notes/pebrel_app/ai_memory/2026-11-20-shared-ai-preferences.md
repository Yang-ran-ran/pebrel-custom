# Shared AI preferences across documents

## Status

Accepted for native prompt suggestions.

## Context

Prompt suggestions are prepared from a bounded conversation snapshot and a selected provider. The user wants writing and workflow habits to remain consistent across documents, while keeping document presentation mode independent. Automatically collecting document bodies would create an unnecessary privacy boundary and could turn unrelated content into persistent memory.

## Evidence

- `prompt_suggestions/client.rs` prepares an immutable provider/credential snapshot before inference.
- `prompt_suggestions/service.rs` owns snapshot invalidation and refresh.
- The provider settings page already exposes prompt-suggestion configuration.

## Decision

Store a single user-authored preference note in `ai-habits.md` under Pebrel's private user data directory. Expose it beside prompt-suggestion settings. Bound it to 4,000 characters, save it atomically with private file permissions, and load it when preparing the immutable inference client. Include it as style/workflow guidance for native prompt suggestions across all documents and the built-in command-fix assistant; it cannot override the system instructions, safety rules, or output format. Prompt suggestions load it into the prepared inference snapshot, and saving invalidates that snapshot so the next request uses the updated note. The command-fix assistant reads it on its background request path.

The note is explicit user input. Document bodies and transcripts are not automatically collected into it.

## Rejected alternatives

- Per-document memory: the user explicitly wants habits shared across documents.
- Automatic ingestion of document bodies: unrelated content could become persistent and be sent to an external provider without a clear user action.
- Reading the preference file during each inference request: this would add disk I/O to the request path and violate the prepared-client ownership contract.

## Consequences

The same preference note is available to built-in AI inference regardless of the active document and is included in the existing Assistant backup category. It is not a general memory layer for arbitrary external CLI agents; those tools retain their own configuration and memory semantics. The user can edit or clear the note in Settings.

## Validation

Unit tests cover bounded preference normalization, prompt payload inclusion, and preservation of the command-fix JSON contract. The settings interaction and native provider request paths require GPUI/native validation before release.

## Supersedes

None.

## Revisit when

Pebrel adds a first-class document chat or another native AI inference surface that should consume the same shared preferences.
