# RFC: A reactive `watch` tool for codex — Claude Code Monitor parity

> Draft for an `openai/codex` issue. Working PoC fork on `feat/watch-monitor-0133`
> (codex 0.133.0), verified live with gpt-5.5. This is a request for **design
> guidance** before opening a PR — the architectural choices below need
> maintainer blessing, and (independently) clarity on whether `openai/codex`
> currently accepts external feature contributions of this shape.

## TL;DR

Codex today has no way for the agent to **react to background events on its own**.
Claude Code has this (its `Monitor` tool + `/loop`) — the user says
*"watch roadmap.md and do task X when it changes"* in natural language, Claude
sets it up, and the agent acts later, without further prompting.

We have a working pure-core implementation of the same behavior in codex
(plain-English request → the model invokes a `watch` tool → background watcher →
agent receives a fresh turn when the watched source changes). It exists today
but needs two architectural decisions maintainers should weigh in on **before**
a PR is opened.

## Problem / motivation

Real workflows where codex currently can't help without constant re-prompting:

- *"Watch the build log and tell me when it fails, with the failing test name."*
- *"Watch `roadmap.md` and when I add a checked-off item, archive it."*
- *"Watch this PR's CI status and notify me when it goes green."*
- *"Watch the bot's heartbeat file; if it stops updating, run the recovery script."*

All of these reduce to *"observe a source → when it changes, run an instruction
on what changed."* In Claude this is one natural-language sentence; in codex
today it requires the user to manually paste status into the chat on every
change.

## Proposed surface — model-callable tools

Three small tools, registered always-on in `spec_plan.rs::add_runtime`:

```text
watch{ command: string, instruction: string, interval_seconds?: number }
    → register a watch. The runtime polls `command` in the background; when its
      output changes (and has settled), a fresh turn fires with `instruction`
      and a diff of what changed.

watch_list{}
    → list active watches: id, command, instruction.

watch_stop{ id: number }
    → abort and remove an active watch.
```

The model chooses to call `watch` when the user describes a reactive intent.
That's the whole UX delta.

A small **`Session.watches`** registry (`Vec<WatchRecord>`) holds the
in-flight tasks (id, command, instruction, `tokio::task::AbortHandle`).

## The two architectural decisions that need a maintainer call

### 1. The self-wake primitive (the load-bearing one)

To make the agent **react** when idle, *something* has to start a new turn from
a background task. Today the inner `Session` (which a tool handler receives via
`ToolInvocation.session: Arc<Session>`) holds `tx_event` but **not** the
submission channel — `tx_sub` lives only on the outer `Codex`. We confirmed
empirically that the mailbox (`InputQueue::enqueue_mailbox_communication`
with `trigger_turn: true`) is consumed only **during an active turn**, so it
can't wake an idle session — codex really has no native event-driven re-entry.

The PoC introduces a small primitive on the inner `Session`:

```rust
pub(crate) self_submit_tx: OnceLock<async_channel::Sender<Submission>>,

pub(crate) async fn submit_self(&self, op: Op) -> bool {
    /* sends Op::UserInput on the session's own submission loop;
       the loop starts a fresh turn regardless of idle/busy state */
}
```

Wired once at session creation (`mod.rs::Session::new` site) with
`session.self_submit_tx.set(tx_sub.clone())`. This is **the** capability that
unlocks any "background task → agent reacts" feature, not just `watch`.

**Decision needed:** is this primitive (a tool / background task injecting
synthetic user turns) acceptable upstream? If yes, should it be gated by a
capability/feature flag, restricted to specific tool exposures, or have an
explicit per-watch acknowledgement on registration?

### 2. Sandboxing background polls

`run_command` in the PoC spawns `sh -c <command>` in the watch's working
directory (captured `turn.cwd` at registration), repeatedly, **outside** the
approval+sandbox path the `shell` tool goes through
(`process_exec_tool_call` → `crate::sandboxing::execute_env`).

The fundamental tension: codex's exec path is **approval-gated and
turn-scoped** (`create_exec_approval_requirement_for_command`, `ShellRequest`,
event emitters). A detached background poll can't do interactive approval
per-poll. Options:

- **(a) Approve-at-registration:** the watch command goes through normal
  exec approval **once** at registration. Subsequent polls run under the
  same captured sandbox policy via `process_exec_tool_call` with the
  permission_profile + sandbox_cwd captured from the turn. Approval handled
  by the existing path; sandbox enforced per-poll.
- **(b) Allowlist:** only register watches whose command matches a safe
  pattern (e.g., `cat <file>`, `tail -n …`, `git status --porcelain`).
  Limits expressiveness; safer surface.
- **(c) Capability flag:** new permission like
  `permissions.allow_background_watch` (default false), unlocking
  unrestricted background polls when on.

The PoC is honest about this — it's just cwd-confined today; full sandbox is a
design choice that needs to come from maintainers.

## Working PoC — what's already verified live

On `feat/watch-monitor-0133` (codex 0.133.0), gpt-5.5, swapped binary:

- **Plain-English invocation:** *"Watch `roadmap.md` and reply WATCH_TOOL_FIRED
  when it changes."* → model calls `watch{command,instruction,interval_seconds}`
  → registers and replies *"Watching."*
- **Autonomous reaction while idle:** edit the file → background poll detects
  change → `submit_self(Op::UserInput{ items:[Text(instruction + [watch fired])] })`
  starts a fresh turn → agent responds with `WATCH_TOOL_FIRED`, **no user input**
  between.
- **Cross-source watching:** demonstrated codex (in WSL) watching another
  WezTerm pane's text via `wezterm.exe cli get-text --pane-id 0`.
- **Burst collapse:** 5 rapid file changes within one interval → **1** reaction
  on the final settled state, not 5. (Settle-detection: react only when output
  is stable across two consecutive polls.)
- **Management:** `watch_list` shows active watches; `watch_stop <id>` aborts.
- **Visibility:** each register/stop emits an `EventMsg::Warning` notice with
  the active count (`"👁 watch [0] active: \`cat roadmap.md\` — 1 watch(es) running"`);
  every reaction shows `[watch fired — …]` inline.

## Trigger sources

- **Command poll (exec)** — implemented; the most general (covers files via
  `cat`, panes via wezterm cli, logs via tail, CI via curl). Settle-detection
  collapses bursts.
- **Native file watch (`notify`)** — designed, not in the current PoC; would
  give lower latency and no polling for the common file case. The crate is
  already a codex workspace dep.
- **Interval timer** — the `/loop` analog; trivial extension of the same
  spawn (no command, fire every N).

A clean upstream design probably exposes one `kind: "exec" | "file" | "timer"`
parameter.

## Robustness in the PoC

- **Settle-detection** (collapses bursts to one reaction).
- **Cooldown** after each fire (≈ `interval * 3`) to avoid stacking reactions.
- **Auto-stop** after 5 consecutive command failures (non-zero exit counts).
- **Cap** of 8 concurrent watches per session.
- **Cwd-confinement** of the poll.
- **Diff-on-change** — reaction text carries the `+`/`-` line diff, not the
  full capture.

## Limitations / known follow-ups

- Auto-stop's notice runs from a background task and must be delivered via the
  same `submit_self` path (one synthetic ack turn) — pure event emission while
  idle doesn't reach the TUI.
- The PoC `tools/handlers/watch.rs` runs `sh -c` directly — **decision #2
  above** decides whether this gets routed through the sandboxed exec path.
- No persistence across session restart (intentional v1 scope).
- The PoC's `Session.watches` registry doesn't surface to the TUI's footer; a
  persistent "N watches" footer would require a new `EventMsg` variant + the
  app-server-protocol mapping + a tui footer widget (≥3 layers of churn we
  preferred to avoid until the core design is blessed).

## Asks

1. **Is the self-wake primitive (`submit_self` / a session-scoped `tx_sub`
   clone exposed to tools) acceptable in principle?** If yes, with what
   constraints (capability flag, exposure limits, acknowledgement at
   registration)?
2. **Which sandboxing model do you prefer for background polls** — approve-at-
   registration + per-poll sandboxed exec; allowlist; capability flag;
   something else?
3. **Process question:** does `openai/codex` currently accept external feature
   contributions of this scope? CLA / contribution requirements?

If the answers are *"yes, here's how,"* we'll clean up (remove the WSL build
hack, drop the redundant `/watch` slash-command commit, base on `main`, add
tests, fold the sandbox decision in) and open a PR.

## Pointers (PoC fork)

- Branch: `feat/watch-monitor-0133` (this fork) — base `rust-v0.133.0`
- Commits (chronological):
  - `0d06246` `feat(core): model-callable watch tool with idle-wake (Claude Monitor parity)`
  - `fadcefdf34` `feat(core): watch_list + watch_stop tools + session watch registry`
  - `a7dc3c70db` `feat(core): visible watch lifecycle notices with active count`
  - `66ecbfd171` `fix(core): watch settle-detection to stop reaction floods`
  - `0a37a4b169` `feat(core): watch robustness — cwd, cooldown, auto-stop, diff, cap`

Happy to demo the live PoC or split commits further once design questions land.
