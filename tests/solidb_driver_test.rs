//! The native SoliDB driver against a real server.
//!
//! Plain ORM reads on the driver decode their rows straight from MessagePack
//! into Soli values (`solidb_driver::try_query_values`) instead of going
//! through a `serde_json::Value` tree first. That fast path has to produce the
//! same values as the JSON path, row for row, for every type a document can
//! carry — and the ORM has to actually take it.
//!
//! Needs a server: `SOLIDB_DRIVER_TEST_HOST=127.0.0.1:6745` (credentials from
//! `SOLIDB_USERNAME` / `SOLIDB_PASSWORD`, `admin`/`admin` by default). Without
//! one the test skips — or fails under `SOLI_REQUIRE_DB=1`, which CI sets next
//! to its SoliDB service so a missing server cannot pass as a green run.
#![cfg(feature = "solidb-driver")]

use solilang::interpreter::value::{json_to_value, Value};

fn server() -> Option<String> {
    match std::env::var("SOLIDB_DRIVER_TEST_HOST") {
        Ok(host) if !host.is_empty() => Some(host),
        _ => {
            if std::env::var("SOLI_REQUIRE_DB").is_ok_and(|flag| flag == "1") {
                panic!(
                    "SOLI_REQUIRE_DB=1 but SOLIDB_DRIVER_TEST_HOST is unset: \
                     the native driver test would have been skipped"
                );
            }
            eprintln!("skip: SOLIDB_DRIVER_TEST_HOST unset (no SoliDB to test the driver against)");
            None
        }
    }
}

fn credentials() -> (String, String) {
    (
        std::env::var("SOLIDB_USERNAME").unwrap_or_else(|_| "admin".into()),
        std::env::var("SOLIDB_PASSWORD").unwrap_or_else(|_| "admin".into()),
    )
}

/// Documents covering what a row can hold: integers, floats, strings, arrays,
/// nested objects and nulls.
fn fixture(i: i64) -> serde_json::Value {
    serde_json::json!({
        "id": i,
        "title": format!("Post title {i}"),
        "views": i * 7,
        "ratio": i as f64 / 4.0,
        "tags": ["a", format!("t{i}")],
        "meta": { "published": i % 2 == 0, "note": serde_json::Value::Null },
    })
}

#[test]
fn driver_rows_decode_to_the_same_values_as_the_json_path() {
    let Some(host) = server() else { return };
    let database = format!("soli_driver_test_{}", std::process::id());
    let (user, pass) = credentials();

    // The driver and the model layer read these once, on first use.
    std::env::set_var("SOLI_DB_DRIVER", "1");
    std::env::set_var("SOLIDB_HOST", format!("http://{host}"));
    std::env::set_var("SOLIDB_DATABASE", &database);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let mut client = solidb_client::SoliDBClient::connect(&host)
            .await
            .expect("connect to SoliDB");
        client
            .auth("_system", &user, &pass)
            .await
            .expect("authenticate");
        client
            .create_database(&database)
            .await
            .expect("create test database");
        client
            .create_collection(&database, "items", None)
            .await
            .expect("create collection");
        for i in 1..=5 {
            client
                .insert(&database, "items", Some(&format!("{i:02}")), fixture(i))
                .await
                .expect("insert fixture");
        }
    });

    let query = "FOR d IN items SORT d._key RETURN d";
    let json_rows = solilang::solidb_driver::try_query(query, None)
        .expect("driver is on")
        .expect("JSON-path query");
    let value_rows = solilang::solidb_driver::try_query_values(query, None)
        .expect("driver is on")
        .expect("value-path query");

    assert_eq!(json_rows.len(), 5, "every fixture row comes back");
    assert_eq!(value_rows.len(), json_rows.len());
    for (json, value) in json_rows.iter().cloned().zip(&value_rows) {
        let expected = json_to_value(json).expect("convert JSON row");
        assert_eq!(&expected, value, "the fast path decoded a row differently");
    }

    // The ORM entry point takes the fast path and returns the same rows.
    let orm = solilang::interpreter::builtins::model::crud::exec_auto_collection(
        query.to_string(),
        "items",
    );
    match orm {
        Value::Array(rows) => assert_eq!(*rows.borrow(), value_rows),
        other => panic!("exec_auto_collection returned {other:?}"),
    }

    rt.block_on(async {
        if let Ok(mut client) = solidb_client::SoliDBClient::connect(&host).await {
            let _ = client.auth("_system", &user, &pass).await;
            let _ = client.delete_database(&database).await;
        }
    });
}
