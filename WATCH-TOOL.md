# Watch tool — patched fork notes

This branch (`feat/watch-monitor-<version>`) is a private patch of
[openai/codex](https://github.com/openai/codex) that adds a reactive
**`watch` tool** to codex's agent toolbox — the equivalent of Claude
Code's Monitor tool. The agent can register a watch in natural
language ("watch roadmap.md and reply when it changes"), and when the
watched source changes, codex receives a fresh turn carrying the diff
and a standing instruction so it can react on its own.

## What's in this branch (vs upstream)

- `codex-rs/core/src/tools/handlers/watch.rs` — the watch tool
  handlers (`watch`, `watch_list`, `watch_stop`).
- `codex-rs/core/src/tools/handlers/watch_spec.rs` — tool specs.
- `codex-rs/core/src/session/session.rs` — `Session.watches` registry,
  `submit_self` self-wake primitive, allocate/register/deregister/stop
  methods.
- `codex-rs/core/src/session/mod.rs` — wires `self_submit_tx` once at
  Session construction so background tasks can start fresh turns.
- `codex-rs/core/src/tools/spec_plan.rs` — registers the three tools
  always-on in the runtime.
- `codex-rs/tui/src/bottom_pane/footer.rs` (+ chat_composer + mod) —
  statusline chip rendering: `· watch: <cmd>` when 1 watch is active,
  `· N watches` when 2+.
- `codex-rs/tui/src/chatwidget.rs` — `core_watches_mirror` map,
  `parse_watch_lifecycle` helper, `add_core_watches_output` overlay
  method, `/watch` (no args) dispatch.
- `codex-rs/tui/src/chatwidget/turn_runtime.rs` — `on_warning` intercept
  that parses lifecycle Warning text and updates the chip + mirror.

## How to use it

1. Check out this branch:
   ```bash
   git clone https://github.com/wolverin0/codex.git
   cd codex
   git checkout feat/watch-monitor-0133
   ```
2. Build (Linux/macOS — ~25 min on the final lto=fat link):
   ```bash
   cd codex-rs
   cargo build --release -p codex-cli
   ```
3. Use the resulting `target/release/codex` directly, or swap it into
   your local codex npm install (see the codex-watcher project below).
4. From codex: *"Register a watch with command `cat roadmap.md`,
   instruction `reply CHANGED with the diff`, interval 5 seconds."*
5. Observe: footer chip shows `· watch: cat roadmap.md`; `/watch`
   opens the active list.

## Maintenance across new codex releases

Upstream codex releases roughly weekly. This branch goes stale every
time. The maintenance kit that automates re-porting the patch to each
new release lives in a separate repo:

**→ [github.com/wolverin0/codex-watcher](https://github.com/wolverin0/codex-watcher)**

That repo contains:
- `scripts/update-to-latest.sh` — fetches new upstream tag,
  cherry-picks our patch commits onto it, builds, swaps the WSL
  vendored binary, runs a smoke test.
- `AGENTS.md` — agent-facing instructions so a Claude Code or codex
  session can run the maintenance loop without prior context.
- `conflict-hints/` — per-sticky-spot guidance for the predictable
  rebase conflicts (FooterProps literal sites, Session struct,
  Warning text format, on_warning intercept).
- `RUNBOOK.md` — manual fallback procedure.

## Design notes / RFC

Two architectural decisions that should land before a PR upstream:

1. **self-wake primitive** — exposing `tx_sub` on the inner
   `Session` (via `self_submit_tx: OnceLock<...>`) so background
   tasks can start a fresh turn even when idle. Today the mailbox
   can't reach an idle agent.
2. **sandboxing background polls** — the PoC's `sh -c <command>`
   runs outside the approval+sandbox path the `shell` tool uses.
   Approve-at-registration vs allowlist vs capability-flag.

Full RFC in [`RFC-watch-tool.md`](./RFC-watch-tool.md) at the root of
this branch. Will be filed as an issue on
[openai/codex](https://github.com/openai/codex) once the design
questions are answered.

## License

This branch inherits openai/codex's license (Apache-2.0). The
maintenance kit at codex-watcher is the same.
