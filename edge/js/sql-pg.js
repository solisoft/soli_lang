// Postgres for Soli's `postgres` adapter in a Worker: node-postgres over the
// Worker's TCP sockets (`nodejs_compat`), usually through Hyperdrive. Copied
// into the Worker by `soli edge build` when the app uses Postgres.
import pg from "pg";

// What each type reads as. Numbers and booleans become JSON numbers and
// booleans, json/jsonb are parsed; everything else (timestamps, uuid…) stays
// the text the native adapter reads.
const PARSERS = {
  16: (v) => v === "t",
  20: integer,
  21: Number,
  23: Number,
  26: Number,
  700: Number,
  701: Number,
  1700: Number,
  114: JSON.parse,
  3802: JSON.parse,
};
const TEXT = (v) => v;

// int8 within 2^53 becomes a number; beyond it stays exact, as text.
function integer(v) {
  const n = Number(v);
  return Number.isSafeInteger(n) ? n : v;
}

export async function connect(connectionString) {
  const client = new pg.Client({
    connectionString,
    types: { getTypeParser: (oid) => PARSERS[oid] ?? TEXT },
  });
  await client.connect();
  return {
    async query(sql, params) {
      // Without parameters the simple protocol runs several statements at
      // once (a migration script); its result is then one per statement.
      const config = params.length
        ? { text: sql, values: params, rowMode: "array" }
        : { text: sql, rowMode: "array" };
      const result = await client.query(config);
      const last = Array.isArray(result) ? result[result.length - 1] : result;
      return {
        columns: (last?.fields ?? []).map((field) => field.name),
        rows: last?.rows ?? [],
        changes: last?.rowCount ?? 0,
      };
    },
    end: () => client.end(),
  };
}
