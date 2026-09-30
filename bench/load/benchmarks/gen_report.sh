#!/usr/bin/env bash
#
# gen_report.sh — turn a directory of raw `oha` JSON into report.md + report.html.
#
# Usage (normally called by run_bench.sh):
#   gen_report.sh <out_dir> <base_url> <duration> <connections> <stamp>
# Reads workload metadata on stdin, one per line:
#   key|group|label|method|path
#
# Can also be re-run by hand against an existing results dir to regenerate
# the reports without re-running the load test.

set -euo pipefail
export LC_ALL=C   # force '.' decimal separator for printf / awk / jq

OUT_DIR="$1"; BASE_URL="$2"; DURATION="$3"; CONNECTIONS="$4"; STAMP="$5"
RAW_DIR="$OUT_DIR/raw"
MD="$OUT_DIR/report.md"
HTML="$OUT_DIR/report.html"
SUMMARY="$OUT_DIR/summary.json"
SUMMARY_NDJSON="$(mktemp)"
trap 'rm -f "$SUMMARY_NDJSON"' EXIT

# round a float to N decimals
r() { printf "%.*f" "$2" "$1"; }

# ---- Markdown header ------------------------------------------------------
{
    echo "# Soli benchmark report"
    echo
    echo "- **Target:** \`$BASE_URL\`"
    echo "- **Run:** $STAMP"
    echo "- **Load:** $DURATION per workload, $CONNECTIONS concurrent connections"
    echo "- **Tool:** \`oha\`"
    echo
    echo "| Workload | Group | Method | Req/s | Success | Requests | p50 (ms) | p90 (ms) | p99 (ms) | Status codes |"
    echo "|----------|-------|--------|------:|--------:|---------:|---------:|---------:|---------:|--------------|"
} > "$MD"

# ---- HTML header ----------------------------------------------------------
GROUP_COLORS_json="#6366f1"
GROUP_COLORS_view="#06b6d4"
GROUP_COLORS_page="#f59e0b"
GROUP_COLORS_db="#a855f7"
GROUP_COLORS_crud="#ec4899"

cat > "$HTML" <<HEAD
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Soli benchmark report — $STAMP</title>
<style>
  :root { color-scheme: dark; }
  * { box-sizing: border-box; }
  body { margin:0; font-family: ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,sans-serif;
         background:#0b1020; color:#e6e9f2; padding:2rem clamp(1rem,4vw,4rem); }
  h1 { font-size:1.6rem; margin:0 0 .25rem; }
  .meta { color:#9aa3bd; font-size:.9rem; margin-bottom:1.75rem; }
  .meta code { background:#1a2138; padding:.1rem .4rem; border-radius:.3rem; }
  .cards { display:grid; grid-template-columns:repeat(auto-fit,minmax(220px,1fr)); gap:1rem; margin-bottom:2rem; }
  .card { background:#141b30; border:1px solid #232c47; border-radius:.75rem; padding:1rem 1.1rem; }
  .card .label { font-size:.8rem; color:#9aa3bd; }
  .card .rps { font-size:1.7rem; font-weight:700; margin:.15rem 0; }
  .card .sub { font-size:.78rem; color:#7e88a6; }
  .tag { display:inline-block; font-size:.68rem; font-weight:600; text-transform:uppercase;
         letter-spacing:.04em; padding:.12rem .5rem; border-radius:1rem; color:#0b1020; }
  .bar-wrap { overflow-x:auto; margin-bottom:2rem; }
  table { border-collapse:collapse; width:100%; font-size:.88rem; min-width:760px; }
  th,td { padding:.55rem .7rem; text-align:right; border-bottom:1px solid #232c47; white-space:nowrap; }
  th:first-child,td:first-child,th:nth-child(2),td:nth-child(2),th:nth-child(3),td:nth-child(3) { text-align:left; }
  thead th { color:#9aa3bd; font-weight:600; border-bottom:1px solid #38436b; }
  tbody tr:hover { background:#10172a; }
  .barcell { position:relative; }
  .bar { display:inline-block; height:.7rem; border-radius:.2rem; vertical-align:middle; margin-right:.4rem; }
  .ok { color:#34d399; } .warn { color:#fbbf24; } .bad { color:#f87171; }
  footer { color:#6b7493; font-size:.8rem; margin-top:1.5rem; }
</style>
</head>
<body>
<h1>Soli benchmark report</h1>
<div class="meta">
  Target <code>$BASE_URL</code> &nbsp;·&nbsp; $DURATION per workload &nbsp;·&nbsp;
  $CONNECTIONS connections &nbsp;·&nbsp; run $STAMP &nbsp;·&nbsp; tool <code>oha</code>
</div>
<div class="cards">
HEAD

# ---- find max rps for bar scaling ----------------------------------------
MAX_RPS=0
declare -A RPS
META=$(cat)   # slurp stdin metadata
while IFS='|' read -r key group label method path; do
    [ -z "$key" ] && continue
    f="$RAW_DIR/$key.json"
    [ -f "$f" ] || continue
    rps=$(jq -r '.summary.requestsPerSec // 0' "$f")
    RPS["$key"]=$rps
    awk "BEGIN{exit !($rps>$MAX_RPS)}" && MAX_RPS=$rps
done <<< "$META"
[ "$(awk "BEGIN{print ($MAX_RPS>0)}")" = "1" ] || MAX_RPS=1

# ---- per-workload rows (cards + md + html table rows) ---------------------
TABLE_ROWS=""
while IFS='|' read -r key group label method path; do
    [ -z "$key" ] && continue
    f="$RAW_DIR/$key.json"
    [ -f "$f" ] || continue

    rps=$(jq -r '.summary.requestsPerSec // 0' "$f")
    succ=$(jq -r '.summary.successRate // 0' "$f")
    p50=$(jq -r '(.latencyPercentiles.p50 // 0)*1000' "$f")
    p90=$(jq -r '(.latencyPercentiles.p90 // 0)*1000' "$f")
    p99=$(jq -r '(.latencyPercentiles.p99 // 0)*1000' "$f")
    reqs=$(jq -r '[.statusCodeDistribution[]?] | add // 0' "$f")
    codes=$(jq -r '.statusCodeDistribution | to_entries | map("\(.key):\(.value)") | join(" ")' "$f")

    succ_pct=$(r "$(awk "BEGIN{print $succ*100}")" 1)
    rps_fmt=$(r "$rps" 0)
    color_var="GROUP_COLORS_$group"; color="${!color_var:-#64748b}"

    # Markdown row
    echo "| $label | $group | $method | $rps_fmt | ${succ_pct}% | $reqs | $(r "$p50" 3) | $(r "$p90" 3) | $(r "$p99" 3) | $codes |" >> "$MD"

    # HTML card
    succ_class="ok"; [ "$(awk "BEGIN{print ($succ<1)}")" = "1" ] && succ_class="warn"
    [ "$(awk "BEGIN{print ($succ<0.99)}")" = "1" ] && succ_class="bad"
    cat >> "$HTML" <<CARD
  <div class="card">
    <span class="tag" style="background:$color">$group</span>
    <div class="label">$label</div>
    <div class="rps">$rps_fmt <span style="font-size:.9rem;font-weight:400;color:#9aa3bd">req/s</span></div>
    <div class="sub">p50 $(r "$p50" 2)ms · p99 $(r "$p99" 2)ms · <span class="$succ_class">${succ_pct}% ok</span></div>
  </div>
CARD

    # machine-readable summary row (consumed by the home dashboard)
    jq -n \
        --arg key "$key" --arg group "$group" --arg label "$label" \
        --arg method "$method" --arg path "$path" --arg color "$color" \
        --arg rps "$rps_fmt" --arg success "$succ_pct" --arg success_class "$succ_class" \
        --arg requests "$reqs" --arg p50 "$(r "$p50" 3)" --arg p90 "$(r "$p90" 3)" \
        --arg p99 "$(r "$p99" 3)" --argjson rps_pct "$(awk "BEGIN{printf \"%.0f\", ($rps/$MAX_RPS)*100}")" \
        --arg codes "$codes" \
        '{key:$key,group:$group,label:$label,method:$method,path:$path,color:$color,
          rps:$rps,success:$success,success_class:$success_class,requests:$requests,
          p50:$p50,p90:$p90,p99:$p99,rps_pct:$rps_pct,codes:$codes}' >> "$SUMMARY_NDJSON"

    # HTML table row (bar width relative to max rps)
    barpct=$(awk "BEGIN{printf \"%.1f\", ($rps/$MAX_RPS)*100}")
    TABLE_ROWS+="<tr><td>$label</td><td><span class=\"tag\" style=\"background:$color\">$group</span></td><td>$method <code>$path</code></td>"
    TABLE_ROWS+="<td class=\"barcell\"><span class=\"bar\" style=\"width:${barpct}px;max-width:120px;background:$color\"></span>$rps_fmt</td>"
    TABLE_ROWS+="<td class=\"$succ_class\">${succ_pct}%</td><td>$reqs</td><td>$(r "$p50" 3)</td><td>$(r "$p90" 3)</td><td>$(r "$p99" 3)</td><td>$codes</td></tr>"
done <<< "$META"

# ---- close HTML -----------------------------------------------------------
cat >> "$HTML" <<MID
</div>
<div class="bar-wrap">
<table>
<thead><tr>
  <th>Workload</th><th>Group</th><th>Endpoint</th><th>Req/s</th><th>Success</th>
  <th>Requests</th><th>p50 ms</th><th>p90 ms</th><th>p99 ms</th><th>Status</th>
</tr></thead>
<tbody>
$TABLE_ROWS
</tbody>
</table>
</div>
<footer>Generated by benchmarks/gen_report.sh · latency percentiles in milliseconds · req/s bars scaled to the fastest workload.</footer>
</body>
</html>
MID

echo "" >> "$MD"
echo "_Latency percentiles in milliseconds. Generated by \`benchmarks/gen_report.sh\`._" >> "$MD"

# ---- machine-readable summary (posted to the home dashboard) --------------
jq -s \
    --arg stamp "$STAMP" --arg base_url "$BASE_URL" \
    --arg duration "$DURATION" --arg connections "$CONNECTIONS" \
    '{stamp:$stamp, base_url:$base_url, duration:$duration, connections:$connections,
      peak_rps: (map(.rps|tonumber)|max|tostring),
      total_requests: (map(.requests|tonumber)|add|tostring),
      workloads:.}' \
    "$SUMMARY_NDJSON" > "$SUMMARY"
