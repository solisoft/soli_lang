# Cross-language benchmark: Ruby side. Output: category|name|best_ms
require 'time'
require 'date'
def best(cat, name, reps = 7)
  yield
  t = reps.times.map { t0=Process.clock_gettime(Process::CLOCK_MONOTONIC); yield; (Process.clock_gettime(Process::CLOCK_MONOTONIC)-t0)*1000.0 }
  puts "#{cat}|#{name}|#{'%.5f' % t.min}"
end
N = 20_000
# Deterministic LCG so Soli and Ruby get byte-identical inputs.
def lcg(n)
  out = []; x = 12_345
  n.times { x = (x * 1_103_515_245 + 12_345) % 2_147_483_648; out << x % 100_000 }
  out
end
A  = lcg(N)                       # shuffled: sort must actually sort
B  = (N/2...(N + N/2)).to_a       # 50% overlap with 0..N for set ops
DUPS = (0...N).map { |i| i % (N/4) }  # 4x duplicates: realistic uniq input
NESTED = (0...(N/10)).map { |i| (0...10).map { |j| i*10+j } }  # real nesting for flatten
SA = (0...N).map { |i| "item-#{i}" }
H  = (0...N).to_h { |i| ["key-#{i}", i] }
G  = (0...N).to_h { |i| ["key-#{i}", i] }
S  = (0...8000).map { |i| "word#{i}" }.join(" ")

# --- Array ---
best("Array","build")        { r=[]; i=0; while i<N; r.push(i); i+=1; end; r }
best("Array","map")          { A.map { |x| x*2 } }
best("Array","filter")       { A.select { |x| x>10 } }
best("Array","reduce")       { A.reduce(0) { |a,x| a+x } }
best("Array","each")         { A.each { |x| x } }
best("Array","sort")         { A.sort }
best("Array","reverse")      { A.reverse }
best("Array","uniq")         { DUPS.uniq }
best("Array","union")        { A | B }
best("Array","intersection") { A & B }
best("Array","difference")   { A - B }
best("Array","flatten")      { NESTED.flatten }
best("Array","sum")          { A.sum }
best("Array","join")         { SA.join(",") }
best("Array","includes")     { A.include?(N-1) }
best("Array","index_of")     { A.index(N-1) }
# --- Hash ---
best("Hash","build")         { h={}; i=0; while i<N; h["key-#{i}"]=i; i+=1; end; h }
best("Hash","get")           { h=H; i=0; while i<N; h["key-500"]; i+=1; end }
best("Hash","set")           { h=H; i=0; while i<N; h["zz"]=i; i+=1; end }
best("Hash","has_key")       { h=H; i=0; while i<N; h.key?("key-500"); i+=1; end }
best("Hash","keys")          { H.keys }
best("Hash","values")        { H.values }
best("Hash","merge")         { H.merge(G) }
best("Hash","each")          { H.each { |_k,v| v } }
best("Hash","select")        { H.select { |_k,v| v>10 } }
best("Hash","transform_values") { H.transform_values { |v| v } }
best("Hash","invert")        { H.invert }
# --- String ---
best("String","upcase")      { S.upcase }
best("String","downcase")    { S.downcase }
best("String","split")       { S.split(" ") }
best("String","chars")       { S.chars }
best("String","bytes")       { S.bytes }
best("String","replace_all") { S.gsub("word","W") }
best("String","sub")         { S.sub("word","W") }
best("String","contains")    { S.include?("word7999") }
best("String","index_of")    { S.index("word7999") }
best("String","reverse")     { S.reverse }
best("String","capitalize")  { S.capitalize }
best("String","concat_plus") { r=+""; i=0; while i<10_000; r = r + "x"; i+=1; end; r }
best("String","interpolate") { i=0; r=nil; while i<N; r="a#{i}b"; i+=1; end; r }
# --- Numeric / control flow ---
best("Numeric","int_loop")   { s=0; i=0; while i<N; s+=i; i+=1; end; s }
best("Numeric","float_math") { s=0.0; i=0; while i<N; s+=i*1.5; i+=1; end; s }
best("Numeric","modulo")     { s=0; i=0; while i<N; s+=(i%7); i+=1; end; s }
best("Control","closure_call"){ f=->(x){x+1}; s=0; i=0; while i<N; s=f.call(s); i+=1; end; s }
best("Control","fn_call")    { def _inc(x)=x+1; s=0; i=0; while i<N; s=_inc(s); i+=1; end; s }

# --- DateTime / Duration ---
# NOTE: Ruby has no Duration type in core. The closest idiomatic equivalents are
# Time arithmetic and Float second differences, which is what these compare against.
DT_ISO = "2026-01-01T00:00:00Z"
T1 = Time.parse(DT_ISO)
T2 = Time.parse("2026-03-15T12:00:00Z")
M = 20_000
best("DateTime","now")         { i=0; while i<M; Time.now; i+=1; end }
best("DateTime","parse")       { i=0; while i<M; Time.parse(DT_ISO); i+=1; end }
best("DateTime","format")      { i=0; while i<M; T1.strftime("%Y-%m-%d"); i+=1; end }
best("DateTime","add_hours")   { i=0; while i<M; T1 + 5*3600; i+=1; end }
best("DateTime","subtract_days"){ i=0; while i<M; T1 - 10*86_400; i+=1; end }
best("DateTime","year")        { i=0; while i<M; T1.year; i+=1; end }
best("DateTime","end_of_month"){ i=0; while i<M; Date.new(T1.year, T1.month, -1); i+=1; end }
best("DateTime","from_unix")   { i=0; while i<M; Time.at(1_700_000_000); i+=1; end }
best("DateTime","to_unix")     { i=0; while i<M; T1.to_i; i+=1; end }
best("Duration","of_days")     { i=0; while i<M; 3*86_400; i+=1; end }
best("Duration","between")     { i=0; while i<M; (T2 - T1); i+=1; end }

# --- Field-keyed aggregates ---
# Soli expresses these with a field *name* (`rows.sum_by("n")`); Ruby's
# equivalents take a block. The block call per element is the point of the
# comparison, not an unfair handicap — it is how the operation is written in
# idiomatic Ruby, and Ruby has no non-block form.
ROWS = (0...N).map { |i| { "t" => "type#{i % 7}", "n" => i } }
FLAT = (0...N).map { |i| i % 7 }
best("Aggregate","sum_by")   { ROWS.sum { |r| r["n"] } }
best("Aggregate","group_by") { ROWS.group_by { |r| r["t"] } }
best("Aggregate","index_by") { ROWS.to_h { |r| [r["t"], r] } }
best("Aggregate","count_by") { ROWS.each_with_object(Hash.new(0)) { |r, h| h[r["t"]] += 1 } }
best("Aggregate","tally")    { FLAT.tally }

