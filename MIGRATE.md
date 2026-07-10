# MIGRATE.md — Adopting upstream mainline into this fork

Plan for moving from this fork (`local-improvements`, v0.1.9) onto upstream
`Nisugi/VellumFE` mainline (v0.3.0), **without** losing the fork's custom work or
the local profile customizations.

> Baseline for this document: `upstream/main` @ **v0.3.0-beta.5**. All file:line
> references were verified against that commit and `local-improvements`
> (fork tip `1db2232`). Re-verify after fetching newer upstream — upstream moves
> fast (was 398 commits ahead at time of writing).
>
> Revised 2026-07-08 after a verification pass: Phase 4 no longer ports the
> AF_UNIX socket. Upstream's embedded web server provides the same command
> injection over a localhost WebSocket, so the primary path is **script
> changes only — zero Rust**. The AF_UNIX port survives as fallback (4B).

---

## TL;DR

- Upstream did a **ground-up rewrite** into a multi-frontend app. The fork's 23
  commits **cannot be rebased/cherry-picked** — the files they patch (`src/app.rs`,
  `src/ui/*`) no longer exist.
- **~15 of the 23 fork commits are already in mainline** (perf + bugfixes, reimplemented independently) — drop them.
- The control socket's job (`sendgs` command injection) is covered by
  upstream's **embedded web server** (localhost WebSocket + pairing token +
  on-disk session registry). Primary plan: **rewrite `sendgs`/`gsread` against
  that, port no Rust at all** — which means no fork branch to maintain; track
  `upstream/main` directly. AF_UNIX port kept as fallback (Phase 4B).
- Fullscreen is **not** ported — replaced by named layouts + a keybind.
- Split-pane / multi-stream tabs are **abandoned** (unused).
- Both the fork branch and the on-disk profiles are safe by default, but
  profiles need a **manual copy + layout migration** to be picked up by v0.3.0.
- The `gs` launcher **survives** (it's Lich orchestration, which upstream does
  not do) but needs edits at switch time — see Phase 4 step 5.

---

## 1. Current state

| | Fork (`local-improvements`) | Upstream `main` |
|---|---|---|
| Version | 0.1.9-beta.1 | 0.3.0-beta.5 |
| Structure | `src/app.rs` + `src/ui/*` | `src/core/*` + `src/frontend/{tui,gui,web,headless}` |
| Remote access | AF_UNIX control socket (`sendgs`/`gsread`/`feed.log`) | WebSocket + PWA (`core/remote.rs`, `frontend/web/`) |
| Profile dir | `~/.vellum-fe/<Char>/` | `~/.vellum-fe/profiles/<Char>/` |

- Remotes: `origin` = `networksandchill/VellumFE_GS4`, `upstream` = `Nisugi/VellumFE`.
- Merge-base: `b75e3bc`. Divergence: **23 ahead / 398 behind**.
- **`origin/local-improvements` is at `c3e05c7`; the local tip `1db2232` is not
  pushed** — push it first (Phase 0).

### File relocation map (for finding old code in the new tree)

| Fork path | Mainline equivalent |
|---|---|
| `src/app.rs` (event loop) | `src/frontend/tui/runtime.rs` |
| `src/app.rs` (command/dot handling, state) | `src/core/app_core/{commands,state,layout,keybinds}.rs` |
| `src/app.rs` (message processing) | `src/core/messages.rs` |
| `src/ui/*` | `src/frontend/tui/*` |
| `src/config.rs` (path helpers) | `src/config/paths.rs` |
| `src/control.rs` | *(not ported — replaced by `src/frontend/web/server.rs`; see Phase 4)* |
| `defaults/*.toml` | `defaults/globals/*.toml` |

---

## 2. Scope decisions

### Drop — already in mainline (do NOT port)
`5a84fff` time-budget msg processing · `a97a746` regex/UTF-8 hot-path · `b75e3bc`/`1898728`
network UTF-8 · `6edffd4` RUST_LOG-gated logging · `207a50d` `<indicator>` parsing ·
`3196bfc` hand/hands highlight + focused-border color · `55251af` ctrl+n/p search cycling ·
`b8ba6bf` countdown scaling · `4442ebf` layout capacity · `f7426d0` Cargo.lock · doc commits.

### Abandon
- **Multi-stream / split-pane tabs** (`57d0f38`, `f3b1ed9`) — unused.

### Replace (config, not code)
- **Fullscreen** (`869fab9`, `5159a21`, `27d742b`, `1e45f64`'s `--fullscreen`) →
  named layouts + keybind. See Phase 3.

### Replace (scripts, not Rust)
- **AF_UNIX control socket** (`8092cc4`, `933c09a`, `c3e05c7`, `1db2232`) →
  upstream's embedded web server already exposes the identical injection path
  (`RemoteEvent::Command`); rewrite `sendgs`/`gsread` against it. See Phase 4.
  AF_UNIX port only if the web route disappoints (Phase 4B).

> Verification note (2026-07-08): the drop list was spot-checked. Search
> cycling (`55251af`) is confirmed in mainline (`NextSearchMatch`/
> `PrevSearchMatch`, `src/config/keybinds.rs`). The time-budget drain
> (`5a84fff`) has **no** mainline equivalent — the drain loop at
> `src/frontend/tui/runtime.rs:610` is unbounded — but mainline's parser
> rewrite (~20 `perf:` commits, benchmarked ~68k lines/sec against a
> 505-pattern highlight set) makes the budget moot: a worst-case Lich-flood
> backlog drains in tens of ms. Confirm via the perf HUD during Phase 1.

---

## 3. Safety model (read before touching anything)

**Fork branch:** trying mainline never modifies `local-improvements`. Only real
gap is the unpushed tip commit (fixed in Phase 0). Work happens on a new branch.

**Profiles:** the v0.3.0 build reads/writes `~/.vellum-fe/profiles/<Char>/`, a
**different path** from the current `~/.vellum-fe/<Char>/`. So it will **not
clobber** existing configs — but it also **won't pick them up**; it seeds fresh
defaults. Real profiles to preserve: **Asoma, Bonsord, Irim** (+ shared
`layouts/`, `sounds/`, `cmdlist1.xml`). The layout TOML format changed, so
`layout.toml` needs upstream's `MigrateLayout` tool, and `config.toml` /
highlights / keybinds moved and may need hand-reconciling.

**Isolation lever:** upstream honors **`VELLUM_FE_DIR`** — point the test build
at a throwaway dir so `~/.vellum-fe` is never touched during evaluation
(`src/config/paths.rs:44`).

---

## 4. Migration plan

### Phase 0 — Back up
```bash
cd ~/gemstone/VellumFE
git push origin local-improvements          # save the unpushed tip 1db2232
git fetch upstream                           # refresh mainline
cp -a ~/.vellum-fe ~/.vellum-fe.bak-$(date +%Y%m%d)   # snapshot profiles
```

### Phase 1 — Build mainline against a sandbox
```bash
git switch -c try-mainline upstream/main
cargo build --release                        # confirm it compiles clean
# NOTE: bare `vellum-fe` opens the graphical launcher on mainline — pass args
# to get the TUI. Sandbox dir keeps real profiles untouched:
VELLUM_FE_DIR=/tmp/vellum-sandbox ./target/release/vellum-fe --port 8000 --character Sandbox
```
Goal: confirm the untouched mainline runs and connects before adding anything.
While connected during a busy stretch, check the perf HUD's parser throughput —
this confirms the "time-budget drop is safe" call from §2 against real load.

### Phase 2 — Migrate ONE profile (start with Asoma)
```bash
# New layout is per-profile under profiles/<Char>/
mkdir -p ~/.vellum-fe/profiles/Asoma
# Convert the old layout format with upstream's tool:
./target/release/vellum-fe migrate-layout --src ~/.vellum-fe/Asoma   # writes ~/.vellum-fe/Asoma/migrated/
# Review, then place the migrated layout.toml into ~/.vellum-fe/profiles/Asoma/
# Hand-reconcile config.toml / highlights / keybinds (formats moved to defaults/globals/*)
```
> Verify the exact `migrate-layout` subcommand flags with `--help` — see
> `src/main.rs` `MigrateLayout` (`src/main.rs:168`, `:255`).

Then run mainline normally (`gs Asoma ...`) and fix up windows/highlights until
the profile feels right. Repeat for Bonsord/Irim once the flow is proven.

### Phase 3 — Fullscreen → layouts (no code)
Named layouts are first-class in mainline: `.savelayout <name>` / `.loadlayout
<name>` (`src/core/app_core/commands.rs:276,287`), and any `.`-prefixed macro
routes to the dot-command handler (`commands.rs:11`).

1. Build a "zoomed" layout (main window maximized, others hidden) and save it:
   `.savelayout zoomed`. Save the normal one too: `.savelayout normal`.
2. Bind two keys via `Macro` keybinds:
   - `F1` → `.loadlayout normal`
   - `F2` → `.loadlayout zoomed`

**Optional single-key toggle (tiny code):** add one `KeyAction::ToggleLayout`
variant in `src/config/keybinds.rs` (enum at `:94`) + a handler that alternates
between two configured layout names. Much smaller than the old fullscreen port.

### Phase 4 — Point `sendgs`/`gsread` at the embedded web server (no Rust)

**What the fork feature did:** a per-character Unix socket letting external
scripts (`sendgs`) inject a command into the running client without opening a
second Lich connection; `gsread` tailed output via `feed.log` (a tee of the
post-hook stream).

**Why no port is needed:** upstream's embedded web server already provides the
identical injection path. A WebSocket client sends a command, the server emits
`RemoteEvent::Command(text)` (`src/frontend/web/server.rs:599`), and the TUI
runtime injects it exactly like locally typed input — echo, dot-commands,
shared history (`src/frontend/tui/runtime.rs:455`). Everything the fork built
in Rust maps to something upstream ships:

| Fork piece | Upstream equivalent |
|---|---|
| `control.sock` per character | WebSocket `/ws` on a per-instance localhost port |
| socket glob for discovery | `~/.vellum-fe/web-sessions/<pid>.json` registry (`{character, port, pid, started_at}`) |
| stale-socket pruning (`c3e05c7`) | registry GC: entries for dead PIDs auto-removed (`server.rs` `registry::list_and_gc`) |
| `--control` / `--no-control` flag | `[web] enabled` in config.toml, or `--web-port <PORT>` CLI flag |
| feed.log tee (`1db2232`) | WS broadcast stream (every client receives finalized styled lines) |
| filesystem-permission auth | pairing token, plain hex in `~/.vellum-fe/web-token` |

Security posture: default bind is `127.0.0.1` (`src/config/settings.rs:516`) —
localhost-only unless deliberately set to `0.0.0.0` for the phone PWA.

**Wire protocol** (newline-delimited JSON text frames; unknown types ignored):
1. First frame must be auth: `{"t":"auth","d":{"token":"<hex from ~/.vellum-fe/web-token>"}}`
2. Then: `{"t":"cmd","d":{"text":"look"}}`
(Parser: `src/frontend/web/protocol.rs:466` `parse_client_message`.)

Steps:
1. **Enable the web server per profile** — `[web] enabled = true` in each
   `~/.vellum-fe/profiles/<Char>/config.toml`. Unpinned instances treat the
   configured port as a base and walk upward when taken, so multiple
   characters need no per-profile port juggling (`settings.rs:511`).
2. **Rewrite `sendgs`** (needs `websocat`; `brew install websocat`):
   - Resolve character → port from `~/.vellum-fe/web-sessions/*.json`
     (same shape as today's socket glob + `VELLUM_CHAR` disambiguation;
     liveness is already guaranteed by the registry's PID GC).
   - Read the token from `~/.vellum-fe/web-token`.
   - `printf '%s\n' "$AUTH_JSON" "$CMD_JSON" | websocat -n1 ws://127.0.0.1:$PORT/ws`
     (fire-and-forget inject).
3. **Rewrite or retire `gsread`** — `feed.log` was fork code and does not
   exist on mainline. Options:
   - Subscribe to the same WebSocket: after auth, the server pushes the
     snapshot + live text stream as JSON; filter/format with `jq`.
   - Or lean on Lich's `;logxml` (raw pre-parse feed, already in the stack).
4. **Output capture (`sendgs look` returning text):** because the WS connection
   that injects the command *also* receives the text stream, per-command
   capture is now easy in-script — send the cmd, print incoming lines until
   the next prompt/quiet period, then close. Start fire-and-forget; add this
   only if missed.
5. **Trim the `gs` launcher** (must happen at switch time — mainline's CLI
   rejects unknown flags):
   - Drop `--links` from the `vellum-fe` launch lines (flag no longer exists).
   - Drop `--no-control` / `NO_CONTROL_ARG` (socket is gone; web toggle lives
     in config.toml).
   - Drop `--fs` / `--fullscreen` (upstream has no such flag — Phase 3's
     layout keybind replaces it).
   - Keep everything else: Lich start + session-file wait, keychain unlock,
     `--steal`, `--rbspy`, `--testbuild`, `--xml` are all Lich orchestration
     upstream does not do. `--character` still works (`--profile` falls back
     to it for the config dir).

**Payoff:** zero fork Rust → no `try-mainline` divergence to maintain; future
upstream updates are `git pull` + rebuild.

### Phase 4B — Fallback: port the AF_UNIX control socket (only if 4 disappoints)

If the WS route proves flaky for scripting (latency, websocat dependency,
token friction), port the fork's socket:

1. **Add `src/control.rs`** — port the `UnixListener` accept loop from
   `local-improvements:src/control.rs`, emitting
   `crate::core::remote::RemoteEvent::Command(line)`.
2. ⚠️ **Channel caveat (verified):** the runtime's `remote_rx` exists **only
   when `web.enabled`** (`runtime.rs:197–208`); with web off there is no
   channel to borrow. The port must create its own
   `mpsc::unbounded_channel::<RemoteEvent>()` and have the runtime drain it
   unconditionally (or merge it into `remote_rx`).
3. Socket path: pin to the old flat `~/.vellum-fe/<Char>/control.sock` so
   `sendgs`/`gsread` globs work unchanged; remove the file on shutdown.
4. feed.log tee: hook `src/core/messages.rs` where finalized lines are
   written (`RemoteSink::push_text` is the model).

### Phase 5 — Deploy / promote
Follow the existing testbuild→promote flow:
```bash
cargo build --release
cp target/release/vellum-fe ~/gemstone/vellum-fe-test    # test with: gs <login> --testbuild
# once verified live:
cp target/release/vellum-fe ~/gemstone/vellum-fe         # promote
```
Deploy the **release** build (~9–10 MB); a debug binary caused UI stalls.

---

## 5. Rollback

- **Binary:** `cp ~/gemstone/vellum-fe.bak ~/gemstone/vellum-fe` (keep a copy of
  the current known-good 9.6 MB binary before promoting).
- **Profiles:** `rm -rf ~/.vellum-fe && mv ~/.vellum-fe.bak-YYYYMMDD ~/.vellum-fe`
  (use `/bin/rm` for the flagged form on this machine).
- **Branch:** `local-improvements` is untouched throughout; `git switch
  local-improvements` returns to the old fork at any time.

---

## 6. Definition of done

- [ ] `try-mainline` builds and runs clean against a real profile.
- [ ] Asoma/Bonsord/Irim migrated (windows, highlights, keybinds, layout).
- [ ] Normal ↔ zoomed layout toggle bound and working.
- [ ] `[web] enabled` in each profile; `~/.vellum-fe/web-sessions/` shows the
      running character(s).
- [ ] Rewritten `sendgs <cmd>` injects into the running client over WS;
      output need covered (`gsread`-over-WS or `;logxml`).
- [ ] `gs` launcher trimmed (`--links`, `--no-control`, `--fs` removed) and
      launches mainline cleanly, including `--steal` and `--testbuild`.
- [ ] Release binary promoted to `~/gemstone/vellum-fe`.
- [ ] If Phase 4 sufficed (no Rust changes): retire `try-mainline`, track
      `upstream/main` directly. If 4B code was written: push `try-mainline`
      to `origin`.
