// ============================================================================
// Model.transaction — block form
//
// `Model.transaction(fn() { ... })` opens a DB transaction, runs the block,
// commits on success, and rolls back (re-raising the error) if the block
// throws. Document writes inside the block (`create` / `save` / `delete` /
// key reads via `find`) participate in the transaction automatically.
//
// These assertions hit the document/transaction endpoints, so they need a live
// SolidB. When no DB is configured they are skipped (the suite still passes),
// mirroring the other model specs.
//
// NOTE: cursor-based reads (`.where(...).all()`) are NOT part of the open
// transaction — they observe committed state. The spec relies on exactly that:
// after commit a query finds the row, after rollback it does not.
// ============================================================================

class TxAccount extends Model
end

// Detect DB availability the same way the other model specs do.
let __db_available = false;
try
    let __probe = TxAccount.create({ "name": "__tx_probe__", "balance": 0 });
    if __probe["valid"]
        __db_available = true;
        __probe["record"].delete();
    end
catch e
end

if __db_available

describe("Model.transaction block form", fn() {
    test("commits writes when the block completes normally", fn() {
        TxAccount.transaction(fn() {
            TxAccount.create({ "name": "tx_commit", "balance": 100 });
        });

        let found = TxAccount.where("name == @n", { "n": "tx_commit" }).all();
        assert(found.length() == 1);

        for record in found
            record.delete();
        end
    });

    test("rolls back writes when the block throws, and re-raises", fn() {
        let threw = false;
        try
            TxAccount.transaction(fn() {
                TxAccount.create({ "name": "tx_rollback", "balance": 50 });
                throw "boom";
            });
        catch error
            threw = true;
        end

        // The original error propagates out of the transaction...
        assert(threw);
        // ...and the write made before the throw was discarded.
        let found = TxAccount.where("name == @n", { "n": "tx_rollback" }).all();
        assert(found.length() == 0);
    });

    test("returns the block's value", fn() {
        let result = TxAccount.transaction(fn() {
            return "committed";
        });
        assert_eq(result, "committed");
    });

    test("nested transactions join the outer one (inner throw rolls back all)", fn() {
        let threw = false;
        try
            TxAccount.transaction(fn() {
                TxAccount.create({ "name": "tx_outer", "balance": 1 });
                TxAccount.transaction(fn() {
                    TxAccount.create({ "name": "tx_inner", "balance": 2 });
                    throw "boom";
                });
            });
        catch error
            threw = true;
        end

        assert(threw);
        // Both the outer and inner writes are gone — the inner transaction
        // joined the outer one, so the single rollback undid everything.
        let outer = TxAccount.where("name == @n", { "n": "tx_outer" }).all();
        let inner = TxAccount.where("name == @n", { "n": "tx_inner" }).all();
        assert(outer.length() == 0);
        assert(inner.length() == 0);
    });
});

end // if __db_available
