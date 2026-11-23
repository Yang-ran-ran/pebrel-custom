# Pebrel ↔ DeepSeek Harness bridge

This is a local DeepSeek Harness bundle, not a DeepSeek model-provider adapter. It subscribes to Harness's documented `session/event` feed and forwards lifecycle-only records to Pebrel's existing `pebrel-hook` helper.

Mapped events:

| Harness session event | Pebrel hook event |
| --- | --- |
| `user/message` | `UserPromptSubmit` |
| `tool/result` | `PostToolUse` |
| `turn/end` | `Stop` |

The bridge intentionally does not forward message bodies, tool arguments, or tool results. It serializes forwarding per session so event order is preserved. It requires the Harness Desktop process to inherit Pebrel's private hook-pipe environment and an absolute `PEBREL_HOOK_EXE` path. Pebrel passes an explicit `PEBREL_PANE_ID` when one is available; otherwise it routes by the Harness session's absolute workspace `cwd`, preferring a matching focused Pane. If neither identity nor a matching workspace path is available, the existing focused-Pane fallback applies.

Install this local bundle into the initialized Desktop profile with:

```powershell
dsh plugin --profile desktop add <persistent-path-to-this-directory>
```

Restart DeepSeek Harness after adding or removing the bundle. Do not install from a temporary directory: the Desktop profile records the local package path.

The Pebrel helper source allowlist and parser support for `deepseek-harness` are included in the current repository patch. The Pebrel Agents UI detects the installed Desktop, manages this bundle through the Desktop-owned `dsh.cmd`, and launches Harness with Pebrel's private hook-pipe/helper environment. Pane routing uses an explicit pane id where available and otherwise the session workspace path.
