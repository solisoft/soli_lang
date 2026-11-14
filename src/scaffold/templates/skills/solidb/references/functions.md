# SDBQL functions

**How this list was built:** every upper-case name in a match arm or route
table of `src/sdbql/executor/` (SoliDB source) was sent to a live server as
`RETURN NAME()`. A name is listed below when the server did **not** answer
`Unknown function: NAME` (it answered with a result or an arity error).
Aggregates and lambda functions were re-probed with arguments. The examples
in the tables were each run and returned exactly the value shown.

An unknown name raises `Unknown function: X` in `RETURN`/`LET`. Inside a
`FILTER` it silently matched nothing through 2.9.4; after it, the query fails
there too. Function names are case-insensitive.

## Verified examples

### Strings

| Call | Result |
|------|--------|
| `CONCAT("a", 1, null, "b")` | `"a1b"` (null skipped) |
| `CONCAT_SEPARATOR(",", ["A","B"])` / `JOIN(["a","b"], ",")` | `"A,B"` / `"a,b"` |
| `LOWER("Hi")`, `UPPER("Hi")` | `"hi"`, `"HI"` |
| `TRIM("  hi  ")`, `LTRIM`, `RTRIM` | `"hi"` |
| `SUBSTRING("Hello", 0, 2)`, `SUBSTRING("Hello", 1)` | `"He"`, `"ello"` |
| `LEFT("Hello", 2)`, `RIGHT("Hello", 2)` | `"He"`, `"lo"` |
| `LENGTH("café")`, `BYTE_LENGTH("café")` | `4`, `5` |
| `LENGTH(null)`, `LENGTH("")` | `0`, `0` |
| `SPLIT("a,b", ",")` | `["a","b"]` |
| `SPLIT_PART("a-b-c", "-", 2)` | `"b"` (1-based) |
| `SUBSTITUTE("aba", "a", "c")` | `"cbc"` |
| `CONTAINS("Hello", "ell")`, `CONTAINS([1,2], 2)` | `true`, `true` |
| `FIND_FIRST("hello", "l")`, `FIND_LAST("hello", "l")` | `2`, `3` |
| `LIKE("Hi", "h%")`, `LIKE("Hi", "h%", true)` | `false`, `true` (3rd arg = case-insensitive) |
| `STARTS_WITH("Hi", "H")`, `ENDS_WITH("Hi", "i")` | `true` |
| `PAD_LEFT("1", 3, "0")`, `PAD_RIGHT("1", 3, "0")` | `"001"`, `"100"` |
| `REPEAT("a", 3)`, `CAPITALIZE("hi")`, `TITLE_CASE("hello world")` | `"aaa"`, `"Hi"`, `"Hello World"` |
| `REGEX_TEST("abc", "^a")`, `REGEX_REPLACE("abc", "b", "d")` | `true`, `"adc"` |
| `REGEX_MATCHES("a1b2", "\\d")` | `["1","2"]` |
| `SLUGIFY("Hello World!")`, `UNACCENT("Hélène")` | `"hello-world"`, `"Helene"` |
| `LEVENSHTEIN("kitten", "sitting")` | `3` |
| `JSON_PARSE("{\"a\":1}")`, `JSON_STRINGIFY({a:1})` | `{a:1}`, `"{\"a\":1}"` |
| `MD5("a")`, `SHA256("a")` | hex digests |
| `BASE64_ENCODE("hi")`, `BASE64_DECODE("aGk=")` | `"aGk="`, `"hi"` |
| `UUID()` / `UUIDV7()` | UUID strings (`IS_UUID(UUID())` → `true`) |

### Numbers and aggregates over arrays

Math and aggregate functions return JSON floats (`SUM([1,2,3])` → `6.0`,
`FLOOR(4.8)` → `4.0`); compare numerically, not by string.

| Call | Result |
|------|--------|
| `ABS(-5)`, `CEIL(4.2)`, `FLOOR(4.8)` | `5.0`, `5.0`, `4.0` |
| `ROUND(2.567, 2)`, `ROUND(2.5)` | `2.57`, `3.0` |
| `SQRT(16)`, `POW(2, 3)`, `MOD(7, 3)`, `CLAMP(10, 0, 5)` | `4.0`, `8.0`, `1.0`, `5.0` |
| `SUM([1,2,3])`, `AVG([1,2,3])` | `6.0`, `2.0` (floats) through 2.9.4; after it `SUM` of integers is an integer: `6`, and `SUM([1, 2.5])` → `3.5`, `SUM([1.0, 2.0])` → `3.0` |
| `MIN([3,1,2])`, `MAX([3,1,2])`, `MEDIAN([1,5,10])` | `1.0`, `3.0`, `5.0` |
| `SUM([])`, `AVG([])`, `MAX([])` | `-0.0`, `null`, `null` (since 2.9.5 `SUM([])` → `0`) |
| `SUM` of ten `0.1`, `AVG([0.1,0.2,0.3])` | `1.0`, `0.19999999999999998` after 2.7.3 (exact sum, rounded once; through 2.7.3 a running f64 sum: `0.9999999999999999`, `0.20000000000000004`). Also in `COLLECT … AGGREGATE`. `+` still rounds each step: `0.1 + 0.2` → `0.30000000000000004` |
| `SUM(FOR … RETURN …)` over zero rows | `-0.0` (since 2.9.5: `0`) |
| `PERCENTILE([1,2,3,4,5,6,7,8,9,10], 90)` | `9.0` (write the list or `1..10`; `[1..10]` is a nested array) |
| `STDDEV([3,1,2,2])` / `STDDEV_SAMP` / `VARIANCE` / `VAR_SAMP` | `0.707…` / `0.816…` / `0.5` / `0.666…` |
| `COUNT_DISTINCT([3,1,2,2])`, `PRODUCT([3,1,2,2])` | `3`, `12.0` |
| `COUNT([1,2])` / `LENGTH([1,2])` | `2` |

### Arrays

| Call | Result |
|------|--------|
| `FIRST([1,2])`, `LAST([1,2])`, `NTH([1,2], 1)` | `1`, `2`, `2` |
| `SLICE([1,2,3], 1, 1)` | `[2]` |
| `PUSH([1], 2)`, `APPEND([1], [2])` | `[1,2]` |
| `UNION([1,2], [2,3])` | `[1,2,3]` — **deduplicates** (AQL's UNION keeps duplicates) |
| `UNION_DISTINCT([1,2], [2,3])`, `INTERSECTION(…)`, `MINUS([1,2], [2])` | `[1,2,3]`, `[2]`, `[1]` |
| `UNIQUE([1,1,2])`, `SORTED([2,1])`, `REVERSE([1,2])` | `[1,2]`, `[1,2]`, `[2,1]` |
| `FLATTEN([[1],[2,[3]]])`, `FLATTEN(…, 2)` | `[1,2,[3]]`, `[1,2,3]` |
| `RANGE(1, 3)` | `[1,2,3]` |
| `POSITION([1,2], 2)`, `INDEX_OF([5,6], 6)`, `CONTAINS_ARRAY([1], 1)` | `true`, `1`, `true` |
| `REMOVE_VALUE([1,2,1], 1)` | `[2]` |
| `TAKE([1,2,3], 2)`, `DROP([1,2,3], 1)`, `CHUNK([1,2,3,4], 2)` | `[1,2]`, `[2,3]`, `[[1,2],[3,4]]` |
| `ZIP(["a"], [1])` | `{a: 1}` |

### Lambdas (`x -> expr`, `(acc, x) -> expr`)

| Call | Result |
|------|--------|
| `MAP([1,2], x -> x * 2)` | `[2,4]` |
| `FILTER([1,2,3], x -> x > 1)` | `[2,3]` |
| `REDUCE([1,2,3], (acc, x) -> acc + x, 10)` | `16` |
| `FIND([5,6,7], x -> x > 5)` | `6` |
| `EVERY([1,2], x -> x > 0)` / `ALL(…)` | `true` |
| `SOME([1,2], x -> x > 1)` | `true` (`ANY(` is taken by the quantifier syntax — parse error) |
| `NONE([1,2], x -> x > 5)` | `true` |
| `GROUP_BY([3,1,2], x -> x)` | `[{key, items}, …]` |
| `SORT_BY([3,1,2], x -> x)` | `[1,2,3]` |
| `FILTER_KEYS({a:1, _b:2}, k -> !STARTS_WITH(k, "_"))` | `{a: 1}` |
| `MAP_VALUES(obj, v -> …)`, `MAP_KEYS(obj, k -> …)` | object in, object out (array → error) |

### Objects / documents

| Call | Result |
|------|--------|
| `MERGE({a:1}, {b:2})` | `{a:1, b:2}` |
| `MERGE({o:{a:1}}, {o:{b:2}})` | `{o:{b:2}}` (shallow) |
| `MERGE_RECURSIVE({o:{a:1}}, {o:{b:2}})` | `{o:{a:1, b:2}}` |
| `KEEP({a:1,b:2}, "a")`, `KEEP(doc, ["a","b"])` | `{a:1}` |
| `UNSET({a:1,b:2}, "a")` | `{b:2}` |
| `ATTRIBUTES({a:1,b:2})`, `VALUES({a:1})`, `ENTRIES({a:1})` | `["a","b"]`, `[1]`, `[["a",1]]` |
| `FROM_ENTRIES([["a",1]])` | `{a:1}` |
| `HAS({a:1}, "a")`, `GET({a:{b:5}}, "a.b", 0)` | `true`, `5` |
| `DOCUMENT("emp/ann")`, `DOCUMENT("emp", "ann")`, `DOCUMENT("emp", ["ann","bob"])` | doc / doc / array; missing → `null` |
| `PARSE_KEY("users/ada")`, `PARSE_COLLECTION("users/ada")` | `"ada"`, `"users"` |
| `COLLECTION_COUNT("emp")` | `4` |

### Types and casts

| Call | Result |
|------|--------|
| `IS_NULL`, `IS_STRING`, `IS_NUMBER`, `IS_ARRAY`, `IS_OBJECT`, `IS_BOOLEAN`, `IS_EMPTY([])` | `true` for the matching type |
| `TYPEOF([])`, `TYPEOF(null)`, `TYPEOF(NOW())` | `"array"`, `"null"`, `"number"` |
| `TO_NUMBER("12")`, `TO_NUMBER("x")` | `12`, `0` |
| `TO_STRING(12)`, `TO_BOOL(0)`, `TO_ARRAY(1)` | `"12"`, `false`, `[1]` |
| `COALESCE(null, 1)`, `NOT_NULL(null, 2)`, `NULLIF(1, 1)` | `1`, `2`, `null` |
| `IF(1 > 0, "y", "n")` | `"y"` |

### Dates

Timestamps are **epoch milliseconds** (`DATE_NOW()`, `NOW()` return numbers);
string results are ISO 8601 with milliseconds and `Z`.

| Call | Result |
|------|--------|
| `DATE_ISO8601(0)` | `"1970-01-01T00:00:00.000Z"` |
| `DATE_TIMESTAMP("2025-01-01T00:00:00Z")` | `1735689600000` |
| `DATE_YEAR("2025-01-01")`, `DATE_MONTH(…)`, `DATE_DAYOFWEEK("2025-01-05")` | `2025`, `1`, `0` (Sunday) |
| `DATE_ADD("2025-01-31", 1, "month")` | `"2025-02-28T00:00:00.000Z"` (clamped) |
| `DATE_SUBTRACT("2025-03-01", 1, "day")` | `"2025-02-28T00:00:00.000Z"` |
| `DATE_DIFF("2025-01-01", "2025-03-01", "days")` | `59` |
| `DATE_FORMAT("2025-03-04T05:06:07Z", "%Y-%m-%d")` | `"2025-03-04"` (strftime) |
| `DATE_TRUNC("2025-03-04T05:06:07Z", "month")` | `"2025-03-01T00:00:00.000Z"` |
| `TIME_BUCKET("2025-01-01T10:07:00Z", "5m")` | `"2025-01-01T10:05:00.000Z"` |

### Geo, vector, fulltext

See indexes-search.md: `DISTANCE(lat1, lon1, lat2, lon2)` (meters),
`GEO_DISTANCE(p1, p2)`, `GEO_POINT(lat, lon)` (→ GeoJSON `[lon, lat]`),
`NEAR(coll, center, radius, field)`, `FULLTEXT(coll, field, q, …)`,
`VECTOR_SEARCH(coll, index, vec, k, opts?)`, `VECTOR_SIMILARITY`,
`VECTOR_DISTANCE(a, b, "euclidean")`.

### Window functions

`ROW_NUMBER()`, `RANK()` and aggregates with `OVER (PARTITION BY … ORDER BY …)`
inside `RETURN` (see sdbql.md). Not callable as plain functions.

## Every accepted name (probed on solidb 2.4.0)

Only the examples above have verified semantics; for the rest only the name
is confirmed.

- **String:** BOOST BYTE_LENGTH CAPITALIZE CHAR_LENGTH CHARACTER_LENGTH CONCAT
  CONCAT_SEPARATOR CONCAT_WS CONTAINS DECODE_URI DECODE_URI_COMPONENT ELLIPSIS
  ENCODE_URI ENCODE_URI_COMPONENT ENDS_WITH FIND FIND_FIRST FIND_LAST
  HUMAN_BYTES INITCAP JOIN LEFT LIKE LOWER LPAD LTRIM MASK NGRAM_SIMILARITY
  NGRAM_POSITIONAL_SIMILARITY NUMBER_FORMAT PAD_LEFT PAD_RIGHT PHRASE
  RANDOM_TOKEN REGEX_MATCH REGEX_MATCHES REGEX_REPLACE REGEX_SPLIT REGEX_TEST
  REPEAT REPLACE REPLACE_NTH REVERSE RFIND RIGHT RPAD RTRIM SPLIT SPLIT_PART
  STARTS_WITH SUBSTITUTE SUBSTR SUBSTRING SUBSTRING_BYTES TITLE_CASE TO_LOWER
  TO_UPPER TOKENS TOLOWER TOUPPER TRANSLATE TRIM TRUNCATE_TEXT UNACCENT UPPER
  URL_DECODE URL_ENCODE WORD_COUNT SLUGIFY SANITIZE HIGHLIGHT SNIPPET IS_BLANK
- **Phonetic / fuzzy:** SOUNDEX SOUNDEX_FR SOUNDEX_ES SOUNDEX_IT SOUNDEX_PT
  SOUNDEX_NL SOUNDEX_EL SOUNDEX_JA METAPHONE DOUBLE_METAPHONE COLOGNE
  COLOGNE_PHONETIC CAVERPHONE NYSIIS LEVENSHTEIN LEVENSHTEIN_DISTANCE
  LEVENSHTEIN_MATCH SIMILARITY FUZZY_MATCH NGRAM_MATCH JACCARD SEMANTIC
- **Math / aggregate:** ABS ACOS ASIN ATAN ATAN2 AVERAGE AVG CBRT CEIL CEILING
  CLAMP COS COUNT_DISTINCT COUNT_UNIQUE UNIQUE_COUNT DEG DEGREES EXP EXP2 FLOOR
  GCD HYPOT LCM LN LOG LOG10 LOG2 MAX MAXIMUM MEDIAN MIN MINIMUM MOD PERCENTILE
  QUANTILE PI POW POWER PRODUCT RAD RADIANS RAND RAND_INT RANDOM RANDOM_INT
  ROUND SIGN SIN SQRT STDDEV STDDEV_POP STDDEV_POPULATION STDDEV_SAMP
  STDDEV_SAMPLE SUM TAN TRUNC TRUNCATE VARIANCE VAR_POP VARIANCE_POPULATION
  VAR_SAMP VARIANCE_SAMPLE BIT_AND BIT_OR BIT_XOR BIT_NOT BIT_NEGATE
  BIT_SHIFT_LEFT BIT_SHIFT_RIGHT DECAY_EXP DECAY_GAUSS DECAY_LINEAR
- **Array:** APPEND CHUNK CONTAINS_ARRAY COUNT COUNT_BY DIFFERENCE DROP FIRST
  FLATTEN INDEX_OF INTERLEAVE INTERSECTION KEY_BY LAST LENGTH MAX_BY MIN_BY
  MINUS MODE NTH OUTERSECTION PAIRWISE POP POSITION PUSH RANGE REMOVE_NTH
  REMOVE_VALUE REMOVE_VALUES SHIFT SHUFFLE SKIP SLICE SORTED SORTED_DESC
  SORTED_UNIQUE SYMDIFF TAKE TRANSPOSE UNION UNION_DISTINCT UNIQUE UNSHIFT ZIP
  ZIP_OBJECT
- **Lambda:** MAP FILTER FLAT_MAP REDUCE FIND GROUP_BY SORT_BY KEY_BY COUNT_BY
  MIN_BY MAX_BY WINDOW_BY EVERY ALL SOME NONE MAP_KEYS MAP_VALUES FILTER_KEYS
- **Object / misc:** ATTRIBUTES KEYS VALUES ENTRIES FROM_ENTRIES MERGE
  MERGE_RECURSIVE DEEP_MERGE DIFF GET HAS KEEP KEEP_RECURSIVE UNSET
  UNSET_RECURSIVE SET_PATH UNSET_PATH REDACT MATCHES VALUE PARSE_URL
  QUERY_STRING PARSE_IDENTIFIER PARSE_COLLECTION PARSE_KEY COALESCE NOT_NULL
  NULLIF IF ASSERT TRY APPLY CALL IN_RANGE EXISTS HASH CRC32 FNV64
  IPV4_FROM_NUMBER IPV4_TO_NUMBER
- **Types / casts:** IS_ARRAY IS_LIST IS_BOOL IS_BOOLEAN IS_DATE IS_DATESTRING
  IS_DATETIME IS_DOCUMENT IS_EMPTY IS_INT IS_INTEGER IS_KEY IS_NULL IS_NUMBER
  IS_NUMERIC IS_OBJECT IS_STRING IS_SAME_COLLECTION IS_EMAIL IS_URL IS_UUID
  IS_IPV4 IS_IBAN IS_SIREN IS_SIRET LUHN TO_ARRAY TO_LIST TO_BOOL TO_BOOLEAN
  TO_NUMBER TO_NUM TO_STRING TO_STR TYPEOF TYPE_OF TYPENAME
- **JSON / encoding / crypto:** JSON_PARSE PARSE_JSON JSON_STRINGIFY
  JSON_STRINGIFY_PRETTY TO_JSON JSON_POINTER BASE64_ENCODE BASE64_DECODE
  TO_BASE64 FROM_BASE64 HEX_ENCODE HEX_DECODE TO_HEX FROM_HEX MD5 SHA1 SHA256
  SHA512 HMAC_SHA256 ARGON2_HASH ARGON2_VERIFY
- **IDs:** UUID UUID_V4 UUIDV4 UUID_V7 UUIDV7 ULID NANOID
- **Date / time series:** NOW NOW_ISO DATE_NOW DATE_NOW_ISO DATE_ADD DATE_SUB
  DATE_SUBTRACT DATE_COMPARE DATE_DIFF DATE_DAY DATE_DAYOFWEEK DATE_DAYOFYEAR
  DATE_DAYS_IN_MONTH DATE_END_OF DATE_FORMAT DATE_HOUR DATE_ISO8601
  DATE_ISOWEEK DATE_ISOWEEKYEAR DATE_LEAPYEAR DATE_MILLISECOND DATE_MINUTE
  DATE_MONTH DATE_PARSE DATE_QUARTER DATE_ROUND DATE_SECOND DATE_SERIES
  DATE_TIMESTAMP DATE_TRUNC DATE_WEEK DATE_YEAR DATE_TIMEZONE DATE_TIMEZONES
  DATE_LOCALTOUTC DATE_UTCTOLOCAL HUMAN_TIME TIME_BUCKET DELTA FILL RATE
  RESAMPLE
- **Geo:** DISTANCE GEO_DISTANCE GEO_WITHIN GEO_CONTAINS GEO_INTERSECTS
  GEO_EQUALS GEO_IN_RANGE GEO_AREA GEO_POINT GEO_LINESTRING GEO_POLYGON
  GEO_MULTIPOINT GEO_MULTILINESTRING GEO_MULTIPOLYGON NEAR
- **Search / vector / graph / AI (need collections, indexes or an LLM
  provider):** FULLTEXT BM25 SEARCH_INDEX SEARCH_SCORE HYBRID_SEARCH SAMPLE
  VECTOR_SEARCH VECTOR_SIMILARITY VECTOR_DISTANCE VECTOR_NORMALIZE
  VECTOR_INDEX_STATS NEIGHBORS GRAPH_RAG COMMUNITY_SEARCH PAGERANK
  DEGREE_CENTRALITY RERANK RAG_PIPELINE EMBED EXTRACT CITE GROUNDED
- **Sketches / catalog / context:** APPROX_COUNT_DISTINCT APPROX_PERCENTILE
  APPROX_TOP_K SKETCH_MERGE MINHASH MINHASH_COUNT MINHASH_ERROR MATCH_SEQ
  DOCUMENT COLLECTION_COUNT CURRENT_DATABASE CURRENT_USER CURRENT_ROLES CAN
  ROW_POLICY DOC_AS_OF DOC_HISTORY SNAPSHOT_DIFF CREATE_GRAPH DROP_GRAPH
  GRAPH_INFO CREATE_VIEW DROP_VIEW

**Not functions** (despite appearing in SoliDB's own reference): `SLEEP`
(`Unknown function: SLEEP`). `COLLECTION(@name)` does not exist either.
