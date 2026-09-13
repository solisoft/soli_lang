#!/usr/bin/env bash
# scripts/mem-probe.sh — what one `soli serve` process actually costs in RAM.
#
# Every worker thread builds its own `Interpreter` and re-parses the whole app
# (src/serve/mod.rs, the `for i in 0..num_workers` loop), so baseline RSS grows
# with SOLI_WORKERS rather than staying flat. That is why
# PRODUCTION_DEFAULT_WORKERS is 2 on a 16-core box. Any change that claims to
# share memory between workers — or between apps in one host process — has to
# show that slope getting flatter, and `ps -o rss=` is too coarse to show it:
# it counts file-backed pages the kernel shares with every other soli process.
#
# So this reads /proc/<pid>/smaps_rollup instead and reports Pss_Anon + SwapPss,
# the anonymous heap that is genuinely this process's own. Both halves matter:
# Pss_Anon counts only *resident* pages, so on a machine that is swapping, part
# of the heap silently drops out of the figure and a process looks smaller the
# more pressure the box is under.
#
# Two more things this had to learn the hard way, both of which produced
# double-digit swings that had nothing to do with the code being measured:
#
#   * mimalloc's default purge delay leaves freed pages mapped, so the RSS left
#     over from the one-time boot-parse churn is still counted. Every sample is
#     taken with MIMALLOC_PURGE_DELAY=0 so the allocator returns them first.
#   * Transparent huge pages get collapsed and split by khugepaged on its own
#     schedule, which moved anonymous RSS by tens of MiB between otherwise
#     identical runs. Nothing here can pin that down, so each point is sampled
#     REPEATS times and reported as a median with its range.
#
# Read the range, not just the median. And do not compare a sweep taken now
# against one taken an hour ago: machine state drifts, and that drift lands
# entirely on whichever side of the comparison ran later. For an A/B, alternate
# the two binaries within one session — `--ab OLD NEW` does exactly that.
#
# Usage:
#   ./scripts/mem-probe.sh www                       # sweep 1 2 4 8 workers
#   WORKER_SWEEP="1 16" REPEATS=5 ./scripts/mem-probe.sh www
#   ./scripts/mem-probe.sh --ab ./old-soli ./new-soli www 16
#   ./scripts/mem-probe.sh --diff before.tsv after.tsv
#
# Env vars (all optional):
#   SOLI_BIN     binary to measure       (default: target/release/soli, else PATH)
#   WORKER_SWEEP worker counts to try    (default: "1 2 4 8")
#   REPEATS      samples per point       (default: 3)
#   LABEL        tag for the output file (default: <timestamp>)
#   OUT_DIR      where to write results  (default: /tmp/soli_mem_probe)
#   PORT         first port to try       (default: 5311)
#   SETTLE       seconds to wait after warmup before sampling (default: 3)
#   PATH_TO_HIT  URL path used to warm   (default: /)
#
# Requires: Linux (smaps_rollup), curl, ss (iproute2).

set -euo pipefail

log()  { printf '[mem-probe] %s\n' "$*" >&2; }
fail() { printf '[mem-probe] ERROR: %s\n' "$*" >&2; exit 1; }

# ---------------------------------------------------------------- diff mode --
# Two sweeps of the same app, one before a change and one after, lined up so
# the slope against worker count is visible rather than just the endpoints.
if [[ "${1:-}" == "--diff" ]]; then
  BEFORE="${2:?usage: mem-probe.sh --diff BEFORE.tsv AFTER.tsv}"
  AFTER="${3:?usage: mem-probe.sh --diff BEFORE.tsv AFTER.tsv}"
  [[ -r "$BEFORE" ]] || fail "cannot read $BEFORE"
  [[ -r "$AFTER"  ]] || fail "cannot read $AFTER"
  log "Two sweeps taken at different times also measure whatever the machine did"
  log "in between. Prefer --ab, which alternates the binaries in one session."
  awk -F'\t' '
    FNR==1 { next }                         # skip header
    NR==FNR { b_med[$1]=$3; b_lo[$1]=$4; b_hi[$1]=$5; next }
    {
      w=$1
      if (!(w in b_med)) next
      d = $3 - b_med[w]
      pct = b_med[w] > 0 ? (d * 100.0 / b_med[w]) : 0
      # A delta smaller than either sample range is not a result.
      noise = (b_hi[w] - b_lo[w] > $5 - $4) ? b_hi[w] - b_lo[w] : $5 - $4
      verdict = (d < 0 ? -d : d) > noise ? "" : "  (within noise)"
      printf "%-9s %9.1f %9.1f %+9.1f %+7.1f%%   %9.1f%s\n", \
             w, b_med[w], $3, d, pct, noise, verdict
    }
  ' "$BEFORE" "$AFTER" | {
    printf '%-9s %9s %9s %9s %8s   %9s\n' \
           workers before after delta "" noise
    printf '%-9s %9s %9s %9s %8s   %9s\n' \
           ""     "MiB"  "MiB"  "MiB" ""  "MiB"
    cat
  }
  exit 0
fi

# ------------------------------------------------------------------- setup --
AB_OLD=""; AB_NEW=""; AB_WORKERS=""
if [[ "${1:-}" == "--ab" ]]; then
  AB_OLD="${2:?usage: mem-probe.sh --ab OLD_BIN NEW_BIN <app-dir> [workers]}"
  AB_NEW="${3:?usage: mem-probe.sh --ab OLD_BIN NEW_BIN <app-dir> [workers]}"
  [[ -x "$AB_OLD" ]] || fail "not executable: $AB_OLD"
  [[ -x "$AB_NEW" ]] || fail "not executable: $AB_NEW"
  AB_WORKERS="${5:-8}"
  set -- "${4:?usage: mem-probe.sh --ab OLD_BIN NEW_BIN <app-dir> [workers]}"
fi

APP="${1:-}"
[[ -n "$APP" ]] || fail "usage: mem-probe.sh <app-dir>   (e.g. ./scripts/mem-probe.sh www)"
[[ -d "$APP" ]] || fail "app directory not found: $APP"
APP="$(cd "$APP" && pwd)"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKER_SWEEP="${WORKER_SWEEP:-1 2 4 8}"
LABEL="${LABEL:-$(date +%Y%m%d-%H%M%S)}"
OUT_DIR="${OUT_DIR:-/tmp/soli_mem_probe}"
PORT="${PORT:-5311}"
SETTLE="${SETTLE:-3}"
REPEATS="${REPEATS:-3}"
PATH_TO_HIT="${PATH_TO_HIT:-/}"

[[ -r /proc/self/smaps_rollup ]] || fail "no /proc/self/smaps_rollup — this probe is Linux-only"
command -v curl >/dev/null || fail "curl not found in PATH"
command -v ss   >/dev/null || fail "ss not found in PATH (install iproute2)"

# Prefer the tree's own release build; fall back to whatever `soli` resolves to.
if [[ -z "${SOLI_BIN:-}" ]]; then
  if   [[ -x "$REPO_ROOT/target/remote/release/soli" ]]; then SOLI_BIN="$REPO_ROOT/target/remote/release/soli"
  elif [[ -x "$REPO_ROOT/target/release/soli"        ]]; then SOLI_BIN="$REPO_ROOT/target/release/soli"
  else SOLI_BIN="$(command -v soli || true)"
  fi
fi
[[ -n "$SOLI_BIN" && -x "$SOLI_BIN" ]] || fail "no soli binary; build one (rbuild lang) or set SOLI_BIN"

# A debug build's numbers are not comparable to a release build's — they are
# several times larger and dominated by unoptimised frames. Say so rather than
# letting a debug sweep be quoted as a baseline.
if [[ "$SOLI_BIN" == *"/debug/"* ]]; then
  log "WARNING: $SOLI_BIN is a debug build; figures are not comparable to release"
fi

mkdir -p "$OUT_DIR"
TSV="$OUT_DIR/${LABEL}.tsv"

log "binary:  $SOLI_BIN"
log "app:     $APP"
log "sweep:   SOLI_WORKERS = $WORKER_SWEEP  (x$REPEATS samples, median reported)"
log "output:  $TSV"

printf 'workers\trss_mib\tanon_median_mib\tanon_min_mib\tanon_max_mib\tboot_ms\n' > "$TSV"

# smaps_rollup field, in MiB with one decimal. Absent fields read as 0 rather
# than aborting the sweep: the set varies with kernel version.
rollup_mib() {
  local pid="$1" field="$2"
  awk -v f="$field:" '$1==f { printf "%.1f", $2/1024; found=1 }
                      END   { if (!found) printf "0.0" }' \
      "/proc/$pid/smaps_rollup" 2>/dev/null || printf '0.0'
}

# The process's own anonymous memory, swapped-out pages included.
#
# Pss_Anon alone counts residency, not footprint: under memory pressure the
# kernel pages part of the heap out and the process reads as *smaller*, which
# is exactly backwards for a measurement whose job is to say how much memory
# the process needs.
anon_mib() {
  local pid="$1"
  awk '$1=="Pss_Anon:" { a=$2 } $1=="SwapPss:" { s=$2 }
       END { printf "%.1f", (a+s)/1024 }' \
      "/proc/$pid/smaps_rollup" 2>/dev/null || printf '0.0'
}

# Median of the values on stdin, one per line.
median() {
  sort -n | awk '{ v[NR]=$1 }
                 END { if (NR==0) { print "0.0"; exit }
                       print (NR%2) ? v[(NR+1)/2] : (v[NR/2]+v[NR/2+1])/2 }'
}

port_is_listening() {
  ss -ltn "sport = :$1" 2>/dev/null | tail -n +2 | grep -q LISTEN
}

# One measurement: boot a server, warm it, sample it, kill it.
# Echoes "<anon_mib> <rss_mib> <boot_ms>".
sample_once() {
  local bin="$1" app="$2" workers="$3"
  while port_is_listening "$PORT"; do PORT=$((PORT + 1)); done
  local port="$PORT"
  local log="$OUT_DIR/${LABEL}_w${workers}.log"
  local start_ns
  start_ns=$(date +%s%N)

  MIMALLOC_PURGE_DELAY=0 SOLI_WORKERS="$workers" \
    setsid nohup "$bin" serve "$app" --port "$port" > "$log" 2>&1 < /dev/null &

  local booted=0
  for _ in $(seq 1 300); do          # up to 60s
    if port_is_listening "$port"; then booted=1; break; fi
    sleep 0.2
  done
  local pid
  pid=$(pgrep -f "serve $app --port $port" | head -1)
  if [[ "$booted" != "1" || -z "$pid" ]]; then
    tail -30 "$log" >&2
    fail "server did not listen on :$port within 60s at workers=$workers"
  fi
  local boot_ms=$(( ($(date +%s%N) - start_ns) / 1000000 ))

  # The first requests page in lazily-initialised statics, the template cache
  # and the JIT, none of which are resident at the listening socket.
  for _ in $(seq 1 15); do
    curl -sf -o /dev/null --max-time 10 "http://127.0.0.1:$port$PATH_TO_HIT" >/dev/null 2>&1 || true
  done
  sleep "$SETTLE"

  local anon rss
  anon=$(anon_mib "$pid")
  rss=$(rollup_mib "$pid" Rss)

  kill "$pid" 2>/dev/null || true
  for _ in $(seq 1 15); do kill -0 "$pid" 2>/dev/null || break; sleep 0.2; done
  kill -9 "$pid" 2>/dev/null || true
  PORT=$((port + 1))
  sleep 1

  echo "$anon $rss $boot_ms"
}

# ------------------------------------------------------------------ A/B mode --
# Alternates two binaries within one session, so machine drift lands on both
# sides instead of on whichever ran later. This is the only comparison worth
# trusting; two sweeps taken minutes apart are not one.
if [[ "$AB_OLD" != "" ]]; then
  workers="${AB_WORKERS:-8}"
  log "A/B at $workers workers, $REPEATS alternating pairs"
  old_vals=""; new_vals=""
  for _ in $(seq 1 "$REPEATS"); do
    old_vals="$old_vals $(sample_once "$AB_OLD" "$APP" "$workers" | cut -d' ' -f1)"
    new_vals="$new_vals $(sample_once "$AB_NEW" "$APP" "$workers" | cut -d' ' -f1)"
  done
  old_med=$(printf '%s\n' $old_vals | median)
  new_med=$(printf '%s\n' $new_vals | median)
  echo
  printf 'old  %s\n' "$(printf '%s\n' $old_vals | tr '\n' ' ')"
  printf 'new  %s\n' "$(printf '%s\n' $new_vals | tr '\n' ' ')"
  printf '\nmedian anon: old %s MiB, new %s MiB (delta %s MiB)\n' \
    "$old_med" "$new_med" \
    "$(awk -v a="$old_med" -v b="$new_med" 'BEGIN { printf "%+.1f", b-a }')"
  log "Ranges overlap more often than not — read them before quoting the median."
  exit 0
fi

# -------------------------------------------------------------------- sweep --
for workers in $WORKER_SWEEP; do
  anon_vals=""; rss_vals=""; boot_vals=""
  for _ in $(seq 1 "$REPEATS"); do
    read -r a r b <<< "$(sample_once "$SOLI_BIN" "$APP" "$workers")"
    anon_vals="$anon_vals $a"; rss_vals="$rss_vals $r"; boot_vals="$boot_vals $b"
  done
  anon_med=$(printf '%s\n' $anon_vals | median)
  rss_med=$(printf '%s\n' $rss_vals | median)
  boot_med=$(printf '%s\n' $boot_vals | median)
  anon_min=$(printf '%s\n' $anon_vals | sort -n | head -1)
  anon_max=$(printf '%s\n' $anon_vals | sort -n | tail -1)

  printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$workers" "$rss_med" "$anon_med" "$anon_min" "$anon_max" "$boot_med" >> "$TSV"
  log "workers=$workers  anon median=${anon_med}MiB [${anon_min}-${anon_max}]  rss=${rss_med}MiB  boot=${boot_med}ms"
done

echo
printf '%-8s %9s %11s %19s %9s\n' workers RSS ANON_MEDIAN ANON_RANGE BOOT
printf '%-8s %9s %11s %19s %9s\n' ""      MiB MiB         MiB        ms
awk -F'\t' 'FNR>1 { printf "%-8s %9s %11s %19s %9s\n", $1, $2, $3, "["$4" - "$5"]", $6 }' "$TSV"
echo
log "TSV: $TSV"
log "A/B two binaries in one session with: $0 --ab OLD NEW <app> [workers]"
