# Stall profiling tools

Helper scripts for the "main window slow to print" investigation. Full write-up:
[`../../STALL.md`](../../STALL.md).

The stall is a ~20s **100%-single-core compute hang inside Lich** (not VellumFE). These
scripts catch the Ruby stack at the moment Lich pegs a core, so we can see exactly which
script/line is burning CPU.

## Scripts

| Script | Needs root? | What it does | Output |
|---|---|---|---|
| `lich_rbspy_auto.sh` | **yes** (`sudo`) | On Lich CPU peak, records a 7s **rbspy** profile (Ruby method + line) | `/tmp/lich_rbspy_<HHMMSS>.txt`, log `/tmp/lich_rbspy.out` |
| `lich_watch.sh` | no | Continuous CPU + game-server recv-queue trace; native `sample` capture on peak | `/tmp/lich_watch.out`, `/tmp/lich_sample_<HHMMSS>.txt` |

## Usage

```bash
# one-time: rbspy (needs root on macOS to read another process's memory)
brew install rbspy

# arm both before a hunting session (leave running ~6h):
sudo tools/stall-profiling/lich_rbspy_auto.sh   # line-level capture
tools/stall-profiling/lich_watch.sh &           # no-sudo trace + native sample
```

When Lich next pegs, the profile auto-records. Read the newest `/tmp/lich_rbspy_*.txt` —
the top entries in the `summary_by_line` output are the lines burning the core.

## Notes

- These profile **Lich** (the Ruby process), not VellumFE — the stall lives upstream of
  the frontend. See STALL.md for why.
- In VellumFE, press **F12** for the live perf HUD; during a stall watch **Net In** — it
  drops to ~0.5–1.2 KB/s while the server is actually sending ~8 KB/s, which is the tell
  that data is backing up in Lich.
- `lich_watch.sh` resolves the game server by DNS name (`GS_HOST=storm.gs4.game.play.net`,
  port `GS_PORT=10024`) and re-resolves periodically, so it follows the rotating
  AWS IPs automatically — no hardcoded IP to go stale. If the hostname/port ever
  changes, edit those two vars. (Sanity-check the live socket with
  `lsof -nP -p "$(pgrep -f lich.rbw)" | grep ESTABLISHED`.)
- Tunables live at the top of each script (duration, peak %, capture length, resolve interval).
