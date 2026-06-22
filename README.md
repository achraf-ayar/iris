# iris

> A live terminal supervisor for every active Claude Code session. **Alpha — working build, not yet published to crates.io.**

[![License: MIT](https://img.shields.io/github/license/itzenata/iris?color=blue)](LICENSE)
[![Status: alpha](https://img.shields.io/badge/status-alpha-yellow)](#whats-working-today)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-dea584?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Made for Claude Code](https://img.shields.io/badge/made%20for-Claude%20Code-c678dd)](https://claude.com/claude-code)
[![Stars](https://img.shields.io/github/stars/itzenata/iris?style=social)](https://github.com/itzenata/iris/stargazers)
[![Last commit](https://img.shields.io/github/last-commit/itzenata/iris?color=green)](https://github.com/itzenata/iris/commits/main)

🌐 **Landing page:** [itzenata.github.io/iris](https://itzenata.github.io/iris/)

## What it does

A fast terminal dashboard that watches **all your running Claude Code sessions at once** — what each one is doing right now, its model, tokens and estimated cost, an AI "doing / done / next" summary, and one-key approval of pending tool calls routed from any session into a single pane.

It reads the transcripts Claude Code already writes under `~/.claude/projects/` — **no daemon, no config, nothing to set up** beyond an optional approval hook.

**Hard rules:** local-first (the only network call is the AI summary you opt into), read-only over your transcripts, and a heartbeat so sessions never hang waiting on iris.

```
 iris  approved Bash   4 active    pending 1    · 3m · 14:21:07

┌ sessions ───────────────┐ ┌ detail ────────────────────────────┐
│ ⚠ Build CLI to superv…   │ │ ⚠ PENDING APPROVAL — Bash in iris   │
│   APPROVE Bash — a/d      │ │ git push --force                    │
│   iris · opus-4-8 · 7.2M  │ │ a allow   d deny                    │
│ ● Slack triage           │ │ model opus-4-8  turns 31  ~cost $24  │
│   running · Bash          │ │ tool calls                          │
│   slack · haiku-4-5       │ │ Bash    ████████░░ 18               │
│ ✓ Configure CloudSQL…    │ │ Edit    █████░░░░░ 9                │
│   done · awaiting you     │ │ ── activity ──                      │
│ ○ Complete five actions   │ │ ▸ you   add the gitignore           │
│   idle · lanoria-club     │ │ ⚒ Bash  cargo build --release       │
└─────────────────────────┘ └─────────────────────────────────────┘
 j/k move  a/d allow/deny  ⏎ details  s summary  i approvals  q quit
```

> See it rendered in color on the [landing page](https://itzenata.github.io/iris/).

## What's working today

A single live pane, refreshed every second:

| Panel | What it shows |
|---|---|
| **Sessions list** | Every session active in the last N minutes, grouped and sorted, color-coded by state |
| **Status glyphs** | `⚠` pending approval · `●` running · `✓` done / awaiting you · `○` idle |
| **Per-session meta** | Model (`opus-4-8`, `sonnet`, `haiku`, `fable`), token total, estimated USD cost |
| **Activity feed** | The latest prompt, thinking, tool call, and result of the entered session, tailed live |
| **Tool timeline** | A histogram of which tools a session leans on — spot the one stuck in a build loop |
| **AI summary** | `s` for a Haiku-generated "doing / done / next" briefing of any session |
| **Approval modal** | `⏎` opens the full tool input with an `x` AI risk read; `a`/`d` allow or deny |

**Views & navigation:** vim motions (`j`/`k`, `g`/`G`, `Ctrl-d`/`Ctrl-u`) on both the session list and the activity feed, foldable groups (`space`/`z`), and an `ls` subcommand that prints a one-shot table with no TUI.

**Remote approvals:** `iris install-hook` registers a `PreToolUse` hook in `settings.json`. With gating armed (`A`), any session's permission prompt routes into iris — approve or deny it, for one session or a whole group, from one place.

**Cost model:** per-model pricing (input / output / cache-write / cache-read) kept as editable constants in [`src/cost.rs`](./src/cost.rs). Figures are estimates — adjust them to your plan.

## Hard rules

- **Read-only over your data.** iris tails the transcript files Claude Code writes; it never edits them.
- **Local-first.** The only outbound request is the AI summary / risk read, and only when you press `s` / `x`. No telemetry, no remote config.
- **Never hangs a session.** iris touches a heartbeat file while running. If it's stale (iris not up) or gating is disarmed, the hook instantly defers to Claude Code's normal permission flow — your sessions are never blocked on a dashboard that isn't there.
- **Opt-in interception.** Approval gating is off until you arm it with `A`, and it disarms automatically when iris exits.
- **Your key, your machine.** The Anthropic API key for summaries is entered in-app (`K`) and saved `0600` in your home directory.

## Install

Not yet on crates.io. To build the alpha:

```bash
git clone https://github.com/itzenata/iris.git
cd iris
cargo install --path .   # drops `iris` in ~/.cargo/bin
```

Then:

```bash
iris                     # live dashboard
iris ls                  # one-shot table, no TUI
iris install-hook        # route approvals through iris (--project for ./.claude)
iris uninstall-hook      # remove the hook
```

Single static binary, built with Rust + [ratatui](https://ratatui.rs). Reads `~/.claude/projects/` — override with `-d <path>`.

## Keys

| Key | Action |
|---|---|
| `j` `k` | move between sessions |
| `g` `G` | jump to first / last · `Ctrl-d` `Ctrl-u` half-page |
| `space` `z` | fold a group / fold all |
| `⏎` | open the approval detail (full input + AI risk read), or enter a session's feed |
| `a` `d` | allow / deny the pending tool call (whole group when a header is selected) |
| `s` | AI summary of the selected session (`g` to regenerate) |
| `x` | AI risk read of the pending tool call |
| `i` | open the approval-interception proposal |
| `A` | arm / disarm approval gating |
| `K` | set your Anthropic API key (saved `0600`) |
| `r` | force refresh |
| `q` | quit |

## Progress

- [x] MIT-licensed, single static Rust binary
- [x] [Landing page](https://itzenata.github.io/iris/) on GitHub Pages
- [x] [Issue templates](.github/ISSUE_TEMPLATE) for bugs, features, integration ideas
- [x] Live session discovery + tailing from `~/.claude/projects/`
- [x] Dashboard: status glyphs, model, tokens, estimated cost
- [x] Activity feed with vim navigation and foldable groups
- [x] Tool-usage histogram per session
- [x] AI "doing / done / next" summaries (Haiku)
- [x] `PreToolUse` hook bridge — remote approve / deny from one pane
- [x] AI risk read on a pending tool call
- [x] Heartbeat fallback so sessions never block on iris
- [x] `ls` one-shot table mode
- [ ] Configurable pricing via file instead of source constants
- [ ] 60s demo video + validation post
- [ ] Publish to crates.io / prebuilt binaries

## Get involved

- ⭐ Star to follow progress
- 💡 [Suggest an integration or signal](https://github.com/itzenata/iris/issues/new?template=integration_suggestion.md)
- 💬 [Open an issue](https://github.com/itzenata/iris/issues/new/choose) for any session-supervision problem you'd want solved

## Development

```bash
cargo run                 # live TUI
cargo run -- ls           # one-shot table
cargo build --release     # optimized binary (LTO, stripped)
```

Code layout: `main.rs` is the CLI entry and event loop; modules under [`src/`](./src) split as `app` (state), `ui` (rendering), `session` (transcript parsing), `bridge` (the hook + heartbeat), `anthropic` (summaries / risk reads), and `cost` (estimation).

License: [MIT](./LICENSE)
