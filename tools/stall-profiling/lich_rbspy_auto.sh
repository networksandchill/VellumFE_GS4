#!/bin/bash
# Auto-capture a Ruby-level profile whenever Lich pegs a CPU core.
#
# Context: see ../../STALL.md. The "main window slow to print" stall is a ~20s
# 100%-single-core compute hang inside Lich. This watches Lich's CPU and, the
# instant it pegs, records a 7s rbspy profile so the hot Ruby method/line is
# captured without anyone having to react in time.
#
# rbspy needs root on macOS (reads another process's memory):  sudo ./lich_rbspy_auto.sh
# Install rbspy:  brew install rbspy
#
# Output: per-capture profile at /tmp/lich_rbspy_<HHMMSS>.txt (summary_by_line:
# the top lines are the ones burning CPU). Run log at /tmp/lich_rbspy.out.

OUT=/tmp/lich_rbspy.out
DURATION_TICKS=86400   # ~6h at 5Hz (0.2s sleep). Lower for a shorter watch.
PEG_PCT=75             # %CPU of one core that counts as "pegged"
RECORD_SECS=7          # length of each rbspy capture

: > "$OUT"
echo "rbspy auto-profiler START $(date +%H:%M:%S) as $(whoami) — watching Lich, ~6h" | tee -a "$OUT"
last=0
for i in $(seq 1 $DURATION_TICKS); do
  sleep 0.2
  LPID=$(pgrep -f 'lich.rbw' | head -1)
  [ -z "$LPID" ] && continue
  cpu=$(ps -o %cpu= -p "$LPID" 2>/dev/null | tr -d ' '); [ -z "$cpu" ] && cpu=0
  now=$(date +%s)
  if awk -v c="$cpu" -v p="$PEG_PCT" 'BEGIN{exit !(c+0>p)}'; then
    if [ $((now-last)) -ge 30 ]; then          # one capture per stall (>=30s apart)
      last=$now; ts=$(date +%H%M%S)
      f="/tmp/lich_rbspy_$ts.txt"
      echo "$(date +%H:%M:%S)  PEG cpu=$cpu%  -> rbspy recording ${RECORD_SECS}s -> $f" | tee -a "$OUT"
      rbspy record --pid "$LPID" --duration "$RECORD_SECS" --format summary_by_line --file "$f" 2>>"$OUT"
      chmod 644 "$f" 2>/dev/null
      echo "$(date +%H:%M:%S)  capture done: $f" | tee -a "$OUT"
    fi
  fi
done
echo "DONE $(date +%H:%M:%S)" | tee -a "$OUT"
