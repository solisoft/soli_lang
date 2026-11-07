use super::*;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufStream, DuplexStream};

async fn send(stream: &mut BufStream<DuplexStream>, message: Value) {
    let body = message.to_string();
    stream
        .write_all(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes())
        .await
        .unwrap();
    stream.flush().await.unwrap();
}

async fn receive(stream: &mut BufStream<DuplexStream>) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut header = String::new();
        stream.read_line(&mut header).await.unwrap();
        let length: usize = header
            .strip_prefix("Content-Length: ")
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut blank = String::new();
        stream.read_line(&mut blank).await.unwrap();
        assert_eq!(blank, "\r\n");
        let mut body = vec![0; length];
        stream.read_exact(&mut body).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    })
    .await
    .expect("LSP server did not respond")
}

#[tokio::test]
async fn native_async_backend_serves_the_document_lifecycle_over_lsp() {
    let (service, socket) = LspService::new(Backend::new);
    let (client, server) = tokio::io::duplex(8192);
    let (input, output) = tokio::io::split(server);
    let task = tokio::spawn(
        tower_lsp_server::Server::new(input, output, socket)
            .concurrency_level(1)
            .serve(service),
    );
    let mut client = BufStream::new(client);

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"capabilities": {}}
        }),
    )
    .await;
    let initialized = receive(&mut client).await;
    assert_eq!(initialized["id"], 1);
    assert_eq!(
        initialized["result"]["capabilities"]["definitionProvider"],
        true
    );

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "method": "initialized", "params": {}
        }),
    )
    .await;
    assert_eq!(receive(&mut client).await["method"], "window/logMessage");

    let uri = "file:///app/greet.sl";
    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": {"textDocument": {
                "uri": uri, "languageId": "soli", "version": 1,
                "text": "def greet(name)\n  name\nend\n\nprint(greet(\"x\"))\n"
            }}
        }),
    )
    .await;
    let diagnostics = receive(&mut client).await;
    assert_eq!(diagnostics["method"], "textDocument/publishDiagnostics");
    assert_eq!(diagnostics["params"]["uri"], uri);
    assert_eq!(diagnostics["params"]["diagnostics"], json!([]));

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "textDocument/definition",
            "params": {"textDocument": {"uri": uri}, "position": {"line": 4, "character": 7}}
        }),
    )
    .await;
    let definition = receive(&mut client).await;
    assert_eq!(definition["result"]["uri"], uri);
    assert_eq!(
        definition["result"]["range"]["start"],
        json!({"line": 0, "character": 4})
    );

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "textDocument/rename",
            "params": {
                "textDocument": {"uri": uri}, "position": {"line": 0, "character": 5},
                "newName": "welcome"
            }
        }),
    )
    .await;
    let renamed = receive(&mut client).await;
    let edits = renamed["result"]["changes"][uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert!(edits.iter().all(|edit| edit["newText"] == "welcome"));

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": {
                "textDocument": {"uri": uri, "version": 2},
                "contentChanges": [{"text": "def greet(\n"}]
            }
        }),
    )
    .await;
    let diagnostics = receive(&mut client).await;
    assert_eq!(diagnostics["method"], "textDocument/publishDiagnostics");
    assert_eq!(diagnostics["params"]["diagnostics"][0]["severity"], 1);

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didClose",
            "params": {"textDocument": {"uri": uri}}
        }),
    )
    .await;
    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 4, "method": "textDocument/definition",
            "params": {"textDocument": {"uri": uri}, "position": {"line": 0, "character": 5}}
        }),
    )
    .await;
    let closed = receive(&mut client).await;
    assert_eq!(closed["id"], 4);
    assert_eq!(closed["result"], Value::Null);
    assert!(closed.get("error").is_none());

    send(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 5, "method": "shutdown"
        }),
    )
    .await;
    let shutdown = receive(&mut client).await;
    assert_eq!(shutdown["id"], 5);
    assert_eq!(shutdown["result"], Value::Null);
    assert!(shutdown.get("error").is_none());
    send(&mut client, json!({"jsonrpc": "2.0", "method": "exit"})).await;
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("LSP server did not exit")
        .unwrap();
}

#[test]
fn a_syntax_error_diagnostic_carries_its_error_code() {
    let error = crate::lint("def broken(\n").unwrap_err();
    let diagnostic = syntax_error_diagnostic(&error);
    let code = match diagnostic.code {
        Some(lsp_types::NumberOrString::String(code)) => code,
        other => panic!("no code: {other:?}"),
    };
    assert!(code.starts_with("E02"), "{code}");
}
