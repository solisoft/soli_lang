//! The `solikv` realtime bus: Redis-style `PUBLISH` / `SUBSCRIBE` on one
//! channel per application, against the SoliKV (or Redis, or Valkey) the app
//! already names with `SOLIKV_RESP_HOST` / `SOLIKV_RESP_PORT` /
//! `SOLIKV_TOKEN`.
//!
//! Publishing goes through the application's RESP pool, on the bus's
//! publisher thread. Each `soli serve` process holds one extra connection in
//! subscribe mode, read on the server runtime; it pings every
//! [`PING_EVERY`] and gives the connection up when nothing — not even the
//! pong — has come back for [`SILENCE_LIMIT`], then reconnects with backoff.
//!
//! Unlike the `solidb` bus, nothing is stored: a message published while a
//! subscriber is reconnecting is lost to it. The same fire-and-forget contract.

use super::{Envelope, CONNECTED};
use crate::interpreter::builtins::resp::RespValue;
use crate::serve::tenant::{self, TenantId};

/// How often the subscriber pings an idle connection.
pub const PING_EVERY: std::time::Duration = std::time::Duration::from_secs(15);

/// How long a subscriber waits for any frame before it reconnects.
pub const SILENCE_LIMIT: std::time::Duration = std::time::Duration::from_secs(45);

/// The RESP channel this application publishes and subscribes on.
///
/// `SOLI_REALTIME_CHANNEL`, or `soli:rt`. Two apps sharing one SoliKV must
/// set distinct channels; in a multi-app (vhost) server, every app but the
/// first adds its directory name, so they never hear each other.
pub fn channel() -> String {
    let base = crate::platform::env::var("SOLI_REALTIME_CHANNEL")
        .ok()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "soli:rt".to_string());
    channel_for(&base, tenant::current_id())
}

fn channel_for(base: &str, id: TenantId) -> String {
    if id == TenantId::PRIMARY {
        return base.to_string();
    }
    let root = tenant::get(id).map(|t| t.root()).unwrap_or_default();
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| id.to_string());
    format!("{base}:{name}")
}

/// `PUBLISH` one envelope. Runs on the publisher thread, under the
/// publishing application.
pub fn publish(envelope: &Envelope) -> Result<(), String> {
    let json = serde_json::to_string(envelope).map_err(|e| e.to_string())?;
    let channel = channel();
    let command = ["PUBLISH", channel.as_str(), json.as_str()];
    let send = || crate::interpreter::builtins::solikv::resp_cmd_unlogged(&command);
    // A pooled connection to a SoliKV that has since restarted accepts the
    // write and fails the read. The pool has dropped it by then, so one retry
    // goes out on a fresh connection instead of losing the message.
    send().or_else(|_| send()).map(|_| ())
}

/// Deliver the payload of one pub/sub frame, if it is a `message` on our
/// channel carrying another process's envelope.
pub(crate) fn handle_frame(frame: &RespValue, channel: &str) -> bool {
    let RespValue::Array(items) = frame else {
        return false;
    };
    let [kind, on, payload] = items.as_slice() else {
        return false;
    };
    if kind.as_str() != Some("message") || on.as_str() != Some(channel) {
        return false;
    }
    let Some(document) = payload
        .as_str()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
    else {
        return false;
    };
    super::receive(document)
}

/// Subscribe on the server runtime, for the application this thread serves.
pub fn start(runtime_handle: &tokio::runtime::Handle) {
    let _guard = runtime_handle.enter();
    tenant::spawn(async move { run().await });
}

const BACKOFF_START: std::time::Duration = std::time::Duration::from_millis(200);
const BACKOFF_MAX: std::time::Duration = std::time::Duration::from_secs(30);

async fn run() {
    let mut backoff = BACKOFF_START;
    let mut announced = false;
    loop {
        let result = subscribe_and_pump().await;
        let had_socket = CONNECTED.write(|connected| std::mem::replace(connected, false));
        if let Err(e) = result {
            if had_socket || !announced {
                eprintln!(
                    "[realtime] bus unavailable: {e} (broadcasts from other processes paused)"
                );
                announced = true;
            }
        }
        if had_socket {
            backoff = BACKOFF_START;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

#[cfg(target_arch = "wasm32")]
async fn subscribe_and_pump() -> Result<(), String> {
    Err(crate::platform::unsupported_on_edge(
        "a SoliKV SUBSCRIBE connection",
    ))
}

#[cfg(not(target_arch = "wasm32"))]
async fn subscribe_and_pump() -> Result<(), String> {
    use crate::interpreter::builtins::resp::encode_command;
    use tokio::io::AsyncWriteExt;

    let (host, port, token) = crate::interpreter::builtins::solikv::resp_endpoint();
    let channel = channel();
    let address = format!("{host}:{port}");
    let stream = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::TcpStream::connect(&address),
    )
    .await
    .map_err(|_| format!("connect to {address} timed out"))?
    .map_err(|e| format!("connect to {address} failed: {e}"))?;
    let _ = stream.set_nodelay(true);
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(read_half);

    let mut hello = Vec::new();
    if let Some(token) = &token {
        hello.extend(encode_command(&["AUTH", token]));
    }
    hello.extend(encode_command(&["SUBSCRIBE", &channel]));
    write_half
        .write_all(&hello)
        .await
        .map_err(|e| format!("SUBSCRIBE failed: {e}"))?;
    if token.is_some() {
        let reply = read_within(&mut reader).await?;
        if let Err(e) = reply {
            return Err(format!("AUTH refused: {e}"));
        }
    }

    let pump = async {
        loop {
            match read_within(&mut reader).await? {
                Err(e) => return Err(format!("SoliKV error: {e}")),
                Ok(frame) => {
                    if is_subscribed(&frame, &channel) {
                        CONNECTED.write(|connected| *connected = true);
                    } else {
                        handle_frame(&frame, &channel);
                    }
                }
            }
        }
    };
    let ping = async {
        let mut every = tokio::time::interval(PING_EVERY);
        every.tick().await;
        loop {
            every.tick().await;
            if let Err(e) = write_half.write_all(&encode_command(&["PING"])).await {
                return Err::<(), String>(format!("PING failed: {e}"));
            }
        }
    };
    tokio::select! {
        result = pump => result,
        result = ping => result,
    }
}

/// One frame, or an error once the connection has been silent for
/// [`SILENCE_LIMIT`]. The outer `Result` is the connection, the inner one a
/// RESP error reply.
#[cfg(not(target_arch = "wasm32"))]
async fn read_within<R>(reader: &mut R) -> Result<Result<RespValue, String>, String>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    tokio::time::timeout(SILENCE_LIMIT, read_frame(reader))
        .await
        .map_err(|_| "no reply from SoliKV".to_string())?
}

/// The `subscribe` confirmation for `channel`.
fn is_subscribed(frame: &RespValue, channel: &str) -> bool {
    matches!(frame, RespValue::Array(items)
        if items.first().and_then(RespValue::as_str) == Some("subscribe")
            && items.get(1).and_then(RespValue::as_str) == Some(channel))
}

/// The largest bulk string the subscriber accepts: an envelope's message plus
/// its JSON wrapping.
const MAX_BULK: usize = super::MAX_MESSAGE_BYTES * 2 + 4096;

/// Read one RESP2 frame. Pub/sub frames are arrays of scalars, so a nested
/// array is refused rather than recursed into. The outer `Result` is a broken
/// stream, the inner one an error reply (`-ERR …`).
pub(crate) async fn read_frame<R>(reader: &mut R) -> Result<Result<RespValue, String>, String>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let line = read_line(reader).await?;
    if let Some(count) = line.strip_prefix('*') {
        let count: i64 = count
            .parse()
            .map_err(|_| format!("bad RESP array header {line:?}"))?;
        if count < 0 {
            return Ok(Ok(RespValue::Null));
        }
        if count > 16 {
            return Err(format!("unexpected {count}-element RESP array"));
        }
        let mut items = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let line = read_line(reader).await?;
            if line.starts_with('*') {
                return Err("nested RESP array on a pub/sub connection".to_string());
            }
            match scalar(reader, &line).await? {
                Ok(item) => items.push(item),
                Err(e) => return Ok(Err(e)),
            }
        }
        return Ok(Ok(RespValue::Array(items)));
    }
    scalar(reader, &line).await
}

async fn scalar<R>(reader: &mut R, line: &str) -> Result<Result<RespValue, String>, String>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let Some(prefix) = line.chars().next() else {
        return Err("empty RESP line".to_string());
    };
    let payload = &line[1..];
    match prefix {
        '+' => Ok(Ok(RespValue::SimpleString(payload.to_string()))),
        '-' => Ok(Err(payload.to_string())),
        ':' => payload
            .parse()
            .map(|i| Ok(RespValue::Integer(i)))
            .map_err(|_| format!("bad RESP integer {line:?}")),
        '$' => {
            let len: i64 = payload
                .parse()
                .map_err(|_| format!("bad RESP bulk header {line:?}"))?;
            if len < 0 {
                return Ok(Ok(RespValue::Null));
            }
            let len = len as usize;
            if len > MAX_BULK {
                return Err(format!("a {len} byte RESP bulk string is over the limit"));
            }
            let mut buf = vec![0u8; len + 2];
            reader
                .read_exact(&mut buf)
                .await
                .map_err(|e| format!("RESP read failed: {e}"))?;
            buf.truncate(len);
            String::from_utf8(buf)
                .map(|s| Ok(RespValue::BulkString(s)))
                .map_err(|_| "RESP bulk string is not UTF-8".to_string())
        }
        _ => Err(format!("unknown RESP type {prefix:?}")),
    }
}

async fn read_line<R>(reader: &mut R) -> Result<String, String>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    use tokio::io::AsyncBufReadExt;
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .await
        .map_err(|e| format!("RESP read failed: {e}"))?;
    if read == 0 {
        return Err("connection closed by SoliKV".to_string());
    }
    if !line.ends_with("\r\n") {
        return Err("truncated RESP line".to_string());
    }
    line.truncate(line.len() - 2);
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::super::Target;
    use super::*;

    fn decode(bytes: &[u8]) -> Result<Result<RespValue, String>, String> {
        let mut reader = bytes;
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(read_frame(&mut reader))
    }

    #[test]
    fn the_decoder_reads_pub_sub_frames() {
        let frame = decode(b"*3\r\n$7\r\nmessage\r\n$7\r\nsoli:rt\r\n$5\r\nhello\r\n")
            .unwrap()
            .unwrap();
        let RespValue::Array(items) = frame else {
            panic!("an array")
        };
        assert_eq!(items[0].as_str(), Some("message"));
        assert_eq!(items[2].as_str(), Some("hello"));

        let subscribed = decode(b"*3\r\n$9\r\nsubscribe\r\n$7\r\nsoli:rt\r\n:1\r\n")
            .unwrap()
            .unwrap();
        assert!(is_subscribed(&subscribed, "soli:rt"));
        assert!(!is_subscribed(&subscribed, "other"));

        assert_eq!(decode(b"+OK\r\n").unwrap().unwrap().as_str(), Some("OK"));
        assert_eq!(
            decode(b"-NOAUTH Authentication required\r\n").unwrap(),
            Err("NOAUTH Authentication required".to_string())
        );
    }

    #[test]
    fn the_decoder_refuses_what_a_pub_sub_connection_never_sends() {
        assert!(decode(b"*1\r\n*1\r\n:1\r\n").is_err(), "nested array");
        assert!(decode(b"*100\r\n").is_err(), "huge array");
        assert!(decode(b"$999999999\r\n").is_err(), "huge bulk");
        assert!(decode(b"").is_err(), "closed");
        assert!(decode(b"+OK").is_err(), "truncated");
        assert!(decode(b"?x\r\n").is_err(), "unknown type");
    }

    #[test]
    fn a_message_frame_delivers_a_foreign_envelope_and_not_our_own() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        crate::interpreter::builtins::streaming::register_subscriber("rt_solikv_spec", tx);
        let mut envelope = Envelope::new(
            Target::Broadcast {
                channel: "rt_solikv_spec".into(),
            },
            "from-b".into(),
        );
        let frame = |envelope: &Envelope, on: &str| {
            RespValue::Array(vec![
                RespValue::BulkString("message".into()),
                RespValue::BulkString(on.into()),
                RespValue::BulkString(serde_json::to_string(envelope).unwrap()),
            ])
        };
        assert!(!handle_frame(&frame(&envelope, "soli:rt"), "soli:rt"));
        assert!(rx.try_recv().is_err(), "our own echo");

        envelope.origin = "node-b:7/abc".into();
        assert!(
            !handle_frame(&frame(&envelope, "elsewhere"), "soli:rt"),
            "another channel"
        );
        assert!(handle_frame(&frame(&envelope, "soli:rt"), "soli:rt"));
        let sse = String::from_utf8(rx.try_recv().unwrap()).unwrap();
        assert!(sse.contains("data: from-b"), "{sse}");
    }

    #[test]
    fn the_first_app_uses_the_base_channel_and_the_others_add_their_name() {
        assert_eq!(channel_for("soli:rt", TenantId::PRIMARY), "soli:rt");
        let id = tenant::register(std::path::PathBuf::from("/srv/sites/shop"));
        assert_eq!(channel_for("soli:rt", id), "soli:rt:shop");
    }
}
