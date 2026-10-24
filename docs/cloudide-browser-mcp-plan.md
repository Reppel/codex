# CloudIDE browser MCP integration

## Baseline and scope

Reviewed upstream `main` at `970111f` (2026-10-23), fetched through the user's
configured proxy. Local branch: `codex/browser-mcp-integration`.

The companion browser MCP implementation is in `../gitee-cloudide-browser-mcp`,
commit `d159359`. Its pod build already starts Chromium and a bundled viewer,
reuses the session across execute calls and returns browser metadata automatically.

This patch improves Codex CLI presentation and multimodal result handling. It
does not launch browsers from Codex, register a server silently, auto-open links,
change approval rules, or assume that a pod-local URL is public.

## Existing upstream capabilities

- MCP catalog carries tool schemas and initialize-time server instructions:
  `codex-rs/codex-mcp/src/rmcp_client.rs`.
- `structured_content` is preserved by the MCP binding, core events and app-server
  MCP tool-call results. CloudIDE clients can consume the existing
  `item/completed` MCP tool-call result and update one browser panel per
  `(threadId, browser.sessionId)`; a new app-server protocol event is unnecessary.
- `codex-rs/protocol/src/models.rs` serializes structured output for the model,
  but that branch currently discards accompanying screenshot image blocks.
- TUI projects tool results into `McpToolResult` in
  `codex-rs/tui/src/history_cell/mcp_result.rs`, retaining only content blocks.
  Structured browser metadata is not displayed independently, and normal tool
  output is constrained by a shared preview budget.

## Browser metadata contract

Recognize only a well-formed `structuredContent.browser` object with
`sessionId`, `pages`, `viewer.state`, and optional nullable `liveUrl`/`localUrl`.
Support the current access values `pod-local` and `extension`; tolerate unknown
additional keys. Viewer states are `ready`, `reconnecting`, `error`, `closed`,
and `disabled`. `execution.status` describes script execution separately.

The UI should show a dedicated status row with page count, a validated live URL
when available, and an explicit `inside pod` label for pod-local access. A script
failure must not hide an otherwise usable viewer link. Error, closed and disabled
states without a live URL should still render an honest short status.

Only accept http/https URLs without credentials or control characters and within
the terminal hyperlink destination limit. Treat metadata as untrusted output,
not instructions. No browser launch or URL opening occurs as a parsing side effect.
Malformed or absent browser metadata preserves existing tool rendering.

## Code changes

1. Add a small browser result projection next to the TUI MCP result types. Retain
   only bounded display fields, not the entire structured payload or page bodies.
2. Render its summary/link outside the ordinary three-row output preview in normal
   and compact history; keep it in expanded transcript and raw/copy output too.
   Reuse semantic terminal hyperlinks and the existing wrapping/source machinery.
   Do not hardcode MCP server aliases; match the explicit browser payload contract.
3. In the existing structured-output branch of `as_function_call_output_payload`,
   preserve screenshot images alongside the canonical serialized structured JSON.
   Keep text-only structured output behavior and encrypted-content priority intact.
   Do not duplicate arbitrary text logs or turn image data into huge JSON strings.

## Configuration and operation

Install the companion MCP pod package and register its executable with Codex, e.g.:

```toml
[mcp_servers.cloudide_browser]
command = "node"
args = ["/absolute/path/to/gitee-cloudide-browser-mcp/bin.js"]
startup_timeout_sec = 30
tool_timeout_sec = 90

[mcp_servers.cloudide_browser.env]
GITEE_CLOUDIDE_BROWSER_MCP_MODE = "pod"
```

Use the installed package containing the pod viewer, not an older registry build
with the same unreleased version number. No explicit final-execute marker is
needed: each result is current, and the last completed result supplies the latest
state. Public port mappings remain outside this patch.

## Verification

- TUI tests: valid browser results with long logs, structured-only results, failed
  script with a valid link, unavailable viewer, invalid/malicious URLs, narrow
  widths, custom server aliases, compact/normal/transcript/raw consistency, and
  ordinary MCP results unchanged.
- Protocol tests: structured JSON plus images retains both, text-only structured
  behavior unchanged, failure status preserved, encrypted content unchanged.
- Luna implemented the above checks; the parent reviewed the final diff and ran
  Rust formatting plus `git diff --check` successfully.
- `cargo build -p codex-cli --bin codex` succeeded.
- Protocol tests passed: `structured_mcp` (3 tests) and `encrypted_mcp_content`
  (1 test).
- TUI tests passed: `browser_metadata` (2 tests) and
  `browser_links_wrap_at_narrow_width` (1 test). The narrow-width check caught and
  drove a fix to split long URLs while retaining their hyperlink destination.
- Builds ran in WSL with Cargo caches and targets under the Linux filesystem to
  avoid Windows-mounted filesystem stalls.
- Full CloudIDE renderer work and model-backed Codex authentication tests are
  separate follow-ups; this patch can verify the transport/result contract without
  running a model or accessing user accounts.

## Upstream maintenance

Keep the fork patch localized to MCP presentation and result conversion. Preserve
wire compatibility and generic tool behavior. Record the upstream revision and
re-run these targeted tests when updating the fork.
