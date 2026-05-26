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

## Install

This is **not** an alternative `npm install` — it's a Rust source
fork. You install official codex first, then **replace its bundled
binary** with one you built from this branch. The npm shim, version
checks, telemetry path, and local SQLite state DB all stay the same.

> ⚠️ Version-pin caveat: this branch is based on upstream codex
> `rust-v0.133.0`. The bundled binary you swap in MUST match the
> version of the `@openai/codex` npm package you installed; otherwise
> the SQLite state DB schema will desync (see
> [the WSL/Windows schema mismatch story](https://github.com/wolverin0/codex/issues/) — `no such table: thread_goals` etc.). Either install the matching npm
> version OR use the [codex-watcher](https://github.com/wolverin0/codex-watcher)
> kit to rebase this branch onto whichever upstream tag matches your
> installed version.

### Linux / WSL x86_64

```bash
# 1. Install the official codex npm package (we keep its npm shim +
#    state DB; we only replace the binary).
npm install -g @openai/codex@0.133.0    # version must match this branch

# 2. Clone the patched fork
cd ~
git clone https://github.com/wolverin0/codex.git codex-patched
cd codex-patched
git checkout feat/watch-monitor-0133

# 3. Build (release, ~20-30 min on the final lto=fat link)
cd codex-rs
cargo build --release -p codex-cli

# 4. Swap the patched binary into the npm vendored slot.
#    First close any running `codex` process (the file gets locked).
TARGET="$HOME/.local/lib/node_modules/@openai/codex/node_modules/@openai/codex-linux-x64/vendor/x86_64-unknown-linux-musl/codex/codex"
cp -p "$TARGET" "${TARGET}.orig-$(date +%s)"   # back up stock binary
cp target/release/codex "$TARGET"

# 5. Verify
codex --version    # should show 0.133.0 (patched)
strings "$TARGET" | grep -c "watch auto-stopped after"   # should print 1
```

### macOS arm64

Same flow, different vendor path:

```bash
TARGET="$HOME/.local/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/codex/codex"
```

(For Intel macOS, use `@openai/codex-darwin-x64` / `x86_64-apple-darwin`.)

### Windows (native, not WSL)

Currently **untested** — the build works under Linux/WSL only because
some build deps assume POSIX. The recommended setup on Windows hosts
is to install official codex in WSL Ubuntu and use that as your daily
codex from Windows Terminal / WezTerm. Native Windows support is a
follow-up if there's demand.

### Reverting to stock codex

```bash
TARGET="$HOME/.local/lib/node_modules/@openai/codex/node_modules/@openai/codex-linux-x64/vendor/x86_64-unknown-linux-musl/codex/codex"
ls -la "${TARGET}".orig-*   # find your backup
cp "${TARGET}.orig-XXXX" "$TARGET"
codex --version    # back to stock 0.133.0
```

## Try it

After installing, in any codex session:

> *"Register a watch with command `cat roadmap.md`, instruction `reply
> CHANGED with the diff`, interval 5 seconds. Then wait silently."*

Expected:
- Lifecycle Warning: `⚠ 👁 watch [0] active: \`cat roadmap.md\` → reply CHANGED with the diff — 1 watch(es) running`
- Footer chip: `· watch: cat roadmap.md`
- Type `/watch` (no args) → renders the active-watch list overlay.
- Modify `roadmap.md` → codex receives a fresh turn with the diff and
  the instruction, reacts on its own.

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
