//! DogStatsD: metrics as UDP (or Unix datagram) lines to a Datadog agent.
//!
//! A worker formats one line (`name:value|type|#tag:v,…`) and hands it to a
//! bounded channel; it never touches a socket. One background thread drains
//! the channel and packs the lines into datagrams of at most
//! [`MAX_UDP_PAYLOAD`] bytes (8 KB on a Unix socket), newline-separated, as the
//! agent expects. A full channel drops the line and counts it — a slow or
//! absent agent must cost a request nothing.

#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::sync_channel;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::OnceLock;

#[cfg(any(test, not(target_arch = "wasm32")))]
use super::config::StatsdTarget;
use super::config::{sanitize_tag, StatsdConfig};

/// The largest UDP payload that fits an Ethernet MTU without fragmenting
/// (the Datadog clients' default).
pub const MAX_UDP_PAYLOAD: usize = 1432;
/// The Datadog clients' default for a Unix datagram socket.
pub const MAX_UDS_PAYLOAD: usize = 8192;
/// Lines waiting for the sender before new ones are dropped.
#[cfg(not(target_arch = "wasm32"))]
const QUEUE_CAP: usize = 16_384;

/// The DogStatsD type of a sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatsdType {
    Count,
    Gauge,
    Histogram,
    Distribution,
}

impl StatsdType {
    pub fn code(self) -> &'static str {
        match self {
            StatsdType::Count => "c",
            StatsdType::Gauge => "g",
            StatsdType::Histogram => "h",
            StatsdType::Distribution => "d",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "c" => Some(StatsdType::Count),
            "g" => Some(StatsdType::Gauge),
            "h" => Some(StatsdType::Histogram),
            "d" => Some(StatsdType::Distribution),
            _ => None,
        }
    }
}

/// A number as DogStatsD reads it: integers without a fraction.
fn format_value(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// One DogStatsD line. `tags` are the sample's own (key as given, value made
/// safe), then `global_tags`.
pub fn format_line(
    name: &str,
    value: f64,
    kind: StatsdType,
    tags: &[(String, String)],
    global_tags: &[String],
) -> String {
    let mut line = format!("{name}:{}|{}", format_value(value), kind.code());
    let mut first = true;
    let mut push = |line: &mut String, tag: &str| {
        line.push_str(if first { "|#" } else { "," });
        first = false;
        line.push_str(tag);
    };
    for (k, v) in tags {
        let tag = if v.is_empty() {
            sanitize_tag(k)
        } else {
            sanitize_tag(&format!("{k}:{v}"))
        };
        push(&mut line, &tag);
    }
    for tag in global_tags {
        push(&mut line, tag);
    }
    line
}

/// Pack lines into datagrams of at most `max` bytes. A line longer than
/// `max` goes alone (the agent may still take it; splitting it would not).
pub fn pack(lines: impl IntoIterator<Item = String>, max: usize) -> Vec<String> {
    let mut packets = Vec::new();
    let mut current = String::new();
    for line in lines {
        if !current.is_empty() && current.len() + 1 + line.len() > max {
            packets.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(&line);
    }
    if !current.is_empty() {
        packets.push(current);
    }
    packets
}

/// The sending half, shared by every worker.
pub struct Sink {
    tx: SyncSender<String>,
    pub global_tags: Vec<String>,
    pub dropped: AtomicU64,
}

impl Sink {
    pub fn send(&self, line: String) {
        match self.tx.try_send(line) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// The process' DogStatsD sink, started on first use; `None` when DogStatsD
/// is not configured or the socket could not be opened.
pub fn sink() -> Option<&'static Sink> {
    static SINK: OnceLock<Option<Sink>> = OnceLock::new();
    SINK.get_or_init(|| {
        let cfg = super::config::config().statsd.as_ref()?;
        start(cfg)
    })
    .as_ref()
}

/// Open the socket and start the sender thread.
#[cfg(not(target_arch = "wasm32"))]
pub fn start(cfg: &StatsdConfig) -> Option<Sink> {
    let transport = match Transport::open(&cfg.target) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "[WARN] DogStatsD: cannot open {:?}: {e}; metrics are not sent",
                cfg.target
            );
            return None;
        }
    };
    let (tx, rx) = sync_channel::<String>(QUEUE_CAP);
    let spawned = std::thread::Builder::new()
        .name("soli-dogstatsd".into())
        .spawn(move || run_sender(rx, transport));
    if let Err(e) = spawned {
        eprintln!("[WARN] DogStatsD: cannot start the sender thread: {e}");
        return None;
    }
    Some(Sink {
        tx,
        global_tags: cfg.global_tags.clone(),
        dropped: AtomicU64::new(0),
    })
}

/// No sockets and no threads on the edge.
#[cfg(target_arch = "wasm32")]
pub fn start(_cfg: &StatsdConfig) -> Option<Sink> {
    None
}

/// Block for one line, take whatever else is already queued, send it all.
#[cfg(not(target_arch = "wasm32"))]
fn run_sender(rx: std::sync::mpsc::Receiver<String>, transport: Transport) {
    let max = transport.max_payload();
    while let Ok(first) = rx.recv() {
        let mut lines = vec![first];
        while lines.len() < 4096 {
            match rx.try_recv() {
                Ok(line) => lines.push(line),
                Err(_) => break,
            }
        }
        for packet in pack(lines, max) {
            if let Err(e) = transport.send(packet.as_bytes()) {
                static WARNED: AtomicBool = AtomicBool::new(false);
                if !WARNED.swap(true, Ordering::Relaxed) {
                    eprintln!("[WARN] DogStatsD send failed (further failures suppressed): {e}");
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
enum Transport {
    Udp(std::net::UdpSocket),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixDatagram, String),
}

#[cfg(not(target_arch = "wasm32"))]
impl Transport {
    fn open(target: &StatsdTarget) -> std::io::Result<Self> {
        match target {
            StatsdTarget::Udp(addr) => {
                use std::net::ToSocketAddrs;
                let resolved = addr.to_socket_addrs()?.next().ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "no address")
                })?;
                let local = if resolved.is_ipv6() {
                    "[::]:0"
                } else {
                    "0.0.0.0:0"
                };
                let socket = std::net::UdpSocket::bind(local)?;
                socket.connect(resolved)?;
                Ok(Transport::Udp(socket))
            }
            #[cfg(unix)]
            StatsdTarget::Unix(path) => {
                let socket = std::os::unix::net::UnixDatagram::unbound()?;
                Ok(Transport::Unix(socket, path.clone()))
            }
            #[cfg(not(unix))]
            StatsdTarget::Unix(_) => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "unix:// sockets need a Unix system",
            )),
        }
    }

    fn max_payload(&self) -> usize {
        match self {
            Transport::Udp(_) => MAX_UDP_PAYLOAD,
            #[cfg(unix)]
            Transport::Unix(..) => MAX_UDS_PAYLOAD,
        }
    }

    fn send(&self, bytes: &[u8]) -> std::io::Result<()> {
        match self {
            Transport::Udp(socket) => socket.send(bytes).map(|_| ()),
            // Unconnected, so an agent restarted under the same path is found
            // again on the next packet.
            #[cfg(unix)]
            Transport::Unix(socket, path) => socket.send_to(bytes, path).map(|_| ()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn lines_follow_the_dogstatsd_format() {
        assert_eq!(
            format_line("orders.created", 1.0, StatsdType::Count, &[], &[]),
            "orders.created:1|c"
        );
        assert_eq!(
            format_line(
                "queue.depth",
                2.5,
                StatsdType::Gauge,
                &tags(&[("queue", "mail"), ("region", "eu|west")]),
                &["env:prod".to_string()]
            ),
            "queue.depth:2.5|g|#queue:mail,region:eu_west,env:prod"
        );
        assert_eq!(
            format_line(
                "checkout.time",
                0.25,
                StatsdType::Distribution,
                &[],
                &["service:shop".into()]
            ),
            "checkout.time:0.25|d|#service:shop"
        );
        assert_eq!(
            format_line("x", 3.0, StatsdType::Histogram, &tags(&[("flag", "")]), &[]),
            "x:3|h|#flag"
        );
    }

    #[test]
    fn packing_respects_the_payload_limit() {
        let lines: Vec<String> = (0..100).map(|i| format!("metric.{i:03}:1|c")).collect();
        let packets = pack(lines.clone(), 64);
        assert!(packets.iter().all(|p| p.len() <= 64), "{packets:?}");
        let rejoined: Vec<String> = packets
            .iter()
            .flat_map(|p| p.split('\n').map(str::to_string))
            .collect();
        assert_eq!(rejoined, lines, "every line, in order, none split");
        // An oversized line goes alone.
        let big = "x".repeat(100);
        let packets = pack(
            vec!["a:1|c".to_string(), big.clone(), "b:1|c".to_string()],
            64,
        );
        assert_eq!(packets, vec!["a:1|c".to_string(), big, "b:1|c".to_string()]);
        assert!(pack(Vec::<String>::new(), 64).is_empty());
    }

    #[test]
    fn a_loopback_agent_receives_batched_lines() {
        let agent = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind agent");
        agent
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("timeout");
        let port = agent.local_addr().expect("addr").port();
        let sink = start(&StatsdConfig {
            target: StatsdTarget::Udp(format!("127.0.0.1:{port}")),
            global_tags: vec!["env:test".to_string()],
        })
        .expect("sink");
        let mut sent = Vec::new();
        for i in 0..200 {
            let line = format_line(
                "loop.hits",
                i as f64,
                StatsdType::Count,
                &tags(&[("n", &i.to_string())]),
                &sink.global_tags,
            );
            sent.push(line.clone());
            sink.send(line);
        }
        let mut received = Vec::new();
        let mut buf = [0u8; 65536];
        while received.len() < sent.len() {
            let n = agent.recv(&mut buf).expect("a datagram before the timeout");
            assert!(n <= MAX_UDP_PAYLOAD, "datagram of {n} bytes");
            let text = std::str::from_utf8(&buf[..n]).expect("utf8");
            received.extend(text.split('\n').map(str::to_string));
        }
        assert_eq!(received, sent);
        assert_eq!(sink.dropped.load(Ordering::Relaxed), 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_unix_socket_agent_receives_lines() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("dsd.socket");
        let agent = std::os::unix::net::UnixDatagram::bind(&path).expect("bind");
        agent
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("timeout");
        let sink = start(&StatsdConfig {
            target: StatsdTarget::Unix(path.to_string_lossy().into_owned()),
            global_tags: Vec::new(),
        })
        .expect("sink");
        sink.send("uds.hits:1|c".to_string());
        let mut buf = [0u8; 8192];
        let n = agent.recv(&mut buf).expect("datagram");
        assert_eq!(&buf[..n], b"uds.hits:1|c");
    }
}
