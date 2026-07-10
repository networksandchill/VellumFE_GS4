#!/bin/bash
# Continuous Lich CPU/queue trace + native-sample auto-capture on peg. No sudo.
#
# Context: see ../../STALL.md. Companion to lich_rbspy_auto.sh that needs no root.
# It logs a continuous trace of Lich's CPU and the game-server socket receive
# queue (so even a no-peg hunt leaves a profile), and fires macOS `sample` on a
# peg as a fallback for rbspy. `sample` shows native frames — enough to classify
# the hang (onig_*/regex = backtracking, gc_* = GC thrash, vm_exec_core = Ruby loop).
#
# Run:  ./lich_watch.sh        (writes /tmp/lich_watch.out, /tmp/lich_sample_<HHMMSS>.txt)
#
# Game-server endpoint: matched by RESOLVING this DNS name at runtime, because
# since the AWS migration the IPs rotate (storm.gs4.game.play.net ->
# chimera.simutronics.com -> several A records) and a hardcoded IP goes stale.
# We re-resolve periodically so the receive-queue match follows IP changes.
GS_HOST='storm.gs4.game.play.net'
GS_PORT=10024
RESOLVE_EVERY=2400     # re-resolve DNS every N ticks (~10min at 4Hz)

OUT=/tmp/lich_watch.out
DURATION_TICKS=86400   # ~6h at 4Hz (0.25s sleep)
LOG_PCT=15             # log a trace line whenever CPU exceeds this
PEG_PCT=75             # %CPU that triggers a native `sample` capture

# Resolve GS_HOST -> an ERE matching "<ip>.<port>" for any current A-record.
# Falls back to dscacheutil, then (if DNS is down) to any IP on the game port.
resolve_gs_match() {
  local ips alt
  ips=$(dig +short "$GS_HOST" 2>/dev/null | grep -E '^[0-9]+(\.[0-9]+){3}$')
  [ -z "$ips" ] && ips=$(dscacheutil -q host -a name "$GS_HOST" 2>/dev/null | awk '/ip_address/{print $2}')
  if [ -z "$ips" ]; then printf '[0-9.]+\\.%s' "$GS_PORT"; return; fi
  alt=$(echo "$ips" | sed 's/\./\\./g' | paste -sd'|' -)
  printf '(%s)\\.%s' "$alt" "$GS_PORT"
}
GS_MATCH=$(resolve_gs_match)

: > "$OUT"
echo "lich-watch START $(date +%H:%M:%S)" >> "$OUT"
last=0
for i in $(seq 1 $DURATION_TICKS); do
  sleep 0.25
  [ $((i % RESOLVE_EVERY)) -eq 0 ] && GS_MATCH=$(resolve_gs_match)
  LPID=$(pgrep -f 'lich.rbw' | head -1); [ -z "$LPID" ] && continue
  cpu=$(ps -o %cpu= -p "$LPID" 2>/dev/null | tr -d ' '); [ -z "$cpu" ] && cpu=0
  now=$(date +%s)
  # continuous: log any moment CPU is elevated so each hunt's profile is visible
  if awk -v c="$cpu" -v p="$LOG_PCT" 'BEGIN{exit !(c+0>p)}'; then
    gsrq=$(netstat -an 2>/dev/null | grep -E "$GS_MATCH" | awk '{print $2; exit}'); [ -z "$gsrq" ] && gsrq=0
    printf '%s cpu=%-6s gsRecvQ=%s\n' "$(date +%H:%M:%S)" "$cpu" "$gsrq" >> "$OUT"
    # peg: native-sample capture (throttled 30s)
    if awk -v c="$cpu" -v p="$PEG_PCT" 'BEGIN{exit !(c+0>p)}' && [ $((now-last)) -ge 30 ]; then
      last=$now; ts=$(date +%H%M%S)
      echo "$(date +%H:%M:%S) >>> PEG $cpu% sampling -> /tmp/lich_sample_$ts.txt" >> "$OUT"
      sample "$LPID" 5 -file "/tmp/lich_sample_$ts.txt" >/dev/null 2>&1
    fi
  fi
  [ $((i % 480)) -eq 0 ] && printf '%s [hb] cpu=%s\n' "$(date +%H:%M:%S)" "$cpu" >> "$OUT"
done
echo "lich-watch DONE $(date +%H:%M:%S)" >> "$OUT"
