//! The `solidb` realtime bus: one row per envelope in the engine-owned
//! `_rt_bus` collection, read back by every `soli serve` process through the
//! collection's changefeed. A sibling of `_live_bus`, sharing its subscription
//! loop ([`crate::live::changefeed`]) and, like it, pinned to the application's
//! base database.

use super::{Envelope, CONNECTED};
use crate::live::changefeed;
use crate::serve::tenant::TenantValue;

/// The bus collection. Underscore-prefixed like `_jobs`: engine-owned.
pub const BUS_COLLECTION: &str = "_rt_bus";

/// How long a row is kept. Subscribers read the envelope out of the
/// changefeed event itself; the row only matters to a debugger.
const PRUNE_AFTER_SECS: i64 = 120;

/// Write one envelope to `_rt_bus`. Runs on the publisher thread, under the
/// publishing application, against its base database.
pub fn publish(envelope: &Envelope) -> Result<(), String> {
    if !crate::db::is_solidb() {
        return Err("SOLI_REALTIME_BUS=solidb needs a SoliDB database".to_string());
    }
    let document = serde_json::to_value(envelope).map_err(|e| e.to_string())?;
    changefeed::base_database(|| {
        ensure_collection_once()?;
        crate::interpreter::builtins::model::crud::exec_insert(BUS_COLLECTION, None, document)
            .map(|_| ())
    })
}

/// Create the bus collection once per application.
fn ensure_collection_once() -> Result<(), String> {
    static DONE: TenantValue<bool> = TenantValue::new(|| false);
    if DONE.read(|done| *done) {
        return Ok(());
    }
    crate::interpreter::builtins::model::crud::ensure_collection(BUS_COLLECTION)?;
    DONE.write(|done| *done = true);
    Ok(())
}

/// Deliver one changefeed frame, if it is another process's envelope.
pub fn handle_frame(text: &str) {
    if let Some(document) = changefeed::inserted_document(text) {
        super::receive(document);
    }
}

/// Subscribe to `_rt_bus` on the server runtime, for the application this
/// thread serves.
pub fn start(runtime_handle: &tokio::runtime::Handle) {
    let _guard = runtime_handle.enter();
    crate::serve::tenant::spawn(async move {
        changefeed::run(changefeed::Feed {
            collection: BUS_COLLECTION,
            log_prefix: "[realtime]",
            paused: "broadcasts from other processes paused",
            connected: &CONNECTED,
            ensure: ensure_collection_once,
            on_frame: handle_frame,
            prune_after_secs: PRUNE_AFTER_SECS,
        })
        .await;
    });
}

#[cfg(test)]
mod tests {
    use super::super::Target;
    use super::*;
    use serde_json::json;

    #[test]
    fn a_foreign_insert_frame_is_delivered_and_our_own_is_not() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        crate::interpreter::builtins::streaming::register_subscriber("rt_solidb_spec", tx);

        let mut envelope = Envelope::new(
            Target::Broadcast {
                channel: "rt_solidb_spec".into(),
            },
            "from-b".into(),
        );
        let own = json!({"operation": "insert", "data": envelope}).to_string();
        handle_frame(&own);
        assert!(
            rx.try_recv().is_err(),
            "our own echo is not delivered twice"
        );

        envelope.origin = "node-b:42".into();
        let foreign = json!({"operation": "insert", "data": envelope}).to_string();
        handle_frame(&foreign);
        let frame = String::from_utf8(rx.try_recv().unwrap()).unwrap();
        assert!(frame.contains("data: from-b"), "{frame}");

        let delete = json!({"operation": "delete", "data": envelope}).to_string();
        handle_frame(&delete);
        assert!(rx.try_recv().is_err(), "a delete is not a message");
    }
}
