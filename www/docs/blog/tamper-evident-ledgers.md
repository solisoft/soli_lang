# Tamper-Evident Audit Logs in Soli, from Two Primitives

Most "audit logs" are a lie waiting to happen. They're a table, and a table can be
`UPDATE`d. If someone with database access edits a row — a transaction amount, an
approval timestamp, who accessed a record — nothing about the log itself reveals
that it happened. For compliance, finance, healthcare, or anything with a legal
retention requirement, a log you can silently rewrite is worth exactly nothing.

The fix is old and well understood: **hash-chain the records**, the way a blockchain
does, minus the distributed-consensus circus you almost never need. Each record
commits to the one before it, so changing any record after the fact breaks every
hash that follows — and that break is detectable by anyone, forever.

Soli ships the two primitives that make this a few lines of code rather than a
subtle crypto project: `Crypto.canonical_json` and `Crypto.merkle_root` (alongside
the `Crypto.sha256` you already know). Let's build a real, verifiable ledger with
them.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/tamper-evident-ledgers.svg" width="1024" height="576" alt="A hash-chained ledger of three records. Record 0 is the genesis; record 1 has been edited after the fact so its hash no longer matches, and the link to record 2 is shown broken. A Merkle root chip commits to the whole set. verify() returns broken_at: 1." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Every record commits to the previous one's hash — so editing record 1 invalidates its own hash and every link after it, and <code>verify()</code> pins the break to the exact record.</figcaption>
</figure>

## The one hard part: stable bytes

A hash is only as good as the bytes you feed it. Hash the same record twice and get
two different digests, and your chain is broken before it starts. This is the trap
that sinks most hand-rolled attempts, because **ordinary JSON serialization is not
stable**: object key order follows insertion order, so `{ "a": 1, "b": 2 }` and
`{ "b": 2, "a": 1 }` — the same record, loaded two different ways — serialize to
different strings and hash to different values.

`Crypto.canonical_json` fixes exactly this. It sorts object keys lexicographically,
recursively, so the same logical content always produces the same bytes:

```soli
Crypto.canonical_json({ "amount": 100, "to": "alice" })   # {"amount":100,"to":"alice"}
Crypto.canonical_json({ "to": "alice", "amount": 100 })   # {"amount":100,"to":"alice"}
```

Two dictionaries, same bytes. Now a hash of that is a stable content fingerprint you
can recompute anywhere, any time.

## Chaining the records

A ledger record carries its business data plus three chain fields: a monotonic
`seq`, the `prev_hash` of the record before it, and its own `hash`. The hash commits
to the previous hash, the sequence number, and the canonical content:

```soli
const GENESIS = "0000000000000000000000000000000000000000000000000000000000000000"

def append(chain, data)
  let prev = chain.length() == 0 ? GENESIS : chain[chain.length() - 1]["hash"]
  let seq  = chain.length()
  let hash = Crypto.sha256(prev + ":" + str(seq) + ":" + Crypto.canonical_json(data))
  chain.push({ "seq": seq, "prev_hash": prev, "hash": hash, "data": data })
  return chain
end
```

Because each `hash` folds in the previous record's `hash`, the records form a chain:
edit record #3's amount, and record #3's hash no longer matches — and so does not
match the `prev_hash` that record #4 committed to, and #5, and every record after.
One tampered row invalidates the entire tail.

Let's write a few:

```soli
let ledger = []
ledger = append(ledger, { "actor": "alice", "action": "transfer", "amount": 100 })
ledger = append(ledger, { "actor": "bob",   "action": "approve",  "ref": 1 })
ledger = append(ledger, { "actor": "alice", "action": "transfer", "amount": 250 })
```

## Verifying integrity

Verification is just re-deriving the chain and checking that every stored hash matches
what the content actually produces. The first mismatch is your tamper point:

```soli
def verify(chain)
  let prev = GENESIS
  for rec in chain
    let expected = Crypto.sha256(prev + ":" + str(rec["seq"]) + ":" + Crypto.canonical_json(rec["data"]))
    if expected != rec["hash"]
      return { "ok": false, "broken_at": rec["seq"] }
    end
    prev = rec["hash"]
  end
  return { "ok": true, "count": chain.length() }
end

print(verify(ledger))   # {ok => true, count => 3}
```

Now play the adversary. Someone with write access quietly bumps a historical amount:

```soli
ledger[1]["data"]["amount"] = 5000   # rewrite an approved record after the fact
print(verify(ledger))                # {ok => false, broken_at => 1}
```

The edit is caught, and pinned to the exact record. No matter how the row was
changed — through the app, through a raw SQL console, through a restored backup — the
chain math doesn't care. If the bytes changed, the hash changed, and verification
fails.

## One hash to prove the whole set: the Merkle root

Re-verifying a million-row ledger on every check is wasteful, and handing a regulator
a million hashes to compare is absurd. A **Merkle root** collapses the entire set into
a single hash. Publish that one value — in a report, a signed email, even a public
blockchain for third-party notarization — and anyone can later re-derive it from the
records and confirm nothing changed.

```soli
let leaves = ledger.map(fn(r) r["hash"])
let root   = Crypto.merkle_root(leaves)
# e.g. "ce947049b20eac714a134d9bcdfd9e72eeb1522bbd8a4fa2deb7a255ea22ee45"
```

`Crypto.merkle_root` pairs leaves as `sha256(left ‖ right)` up the tree until one hash
remains. Change any leaf — or even reorder them — and the root changes. It's the
compact commitment you archive daily: a 64-character string that proves the integrity
of everything underneath it.

## Wiring it into a Model

In a real app you don't hand-roll the array — you persist records and compute the
chain against the tail. The pattern drops straight into a model's `before_create`
callback, where the record is about to be written:

```soli
class LedgerEntry < Model
  before_create("chain")

  def chain
    let tail = LedgerEntry.order("seq", "desc").first()
    this.prev_hash = tail.nil? ? GENESIS : tail.hash
    this.seq       = tail.nil? ? 0 : tail.seq + 1
    this.hash      = Crypto.ledger_hash(this.prev_hash, this.seq, this.to_h())
  end
end
```

Two conveniences show up here. `instance.to_h()` returns the record's user fields as a
hash — exactly what you want to hash. And `Crypto.ledger_hash(prev, seq, data)` is a
one-call shorthand for the `sha256(prev ":" seq ":" canonical_json(data))` formula
above, so your write path and your verifier can share a single definition and never
drift apart.

## What this is, and what it isn't

Be honest with yourself about the threat model. A hash chain is **tamper-evident**,
not **tamper-proof**: an attacker who can rewrite records *and* recompute the whole
chain from their edit forward can produce a self-consistent forgery. What defeats that
is anchoring — periodically publishing the Merkle root somewhere you don't control (a
counter-signed report, a notary, a public chain), so the attacker can't also rewrite
history's published fingerprints. The chain makes silent edits impossible; anchoring
the root makes *undetectable* edits impossible.

That's the whole point: not a cryptocurrency, not consensus, not a P2P network — just
an append-only log whose integrity anyone can verify with two builtins and a loop.
For audit trails, financial ledgers, provenance records, and compliance logs, that's
exactly the guarantee you actually need.
