use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const IO_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum RespValue {
    SimpleString(String),
    Error(String),
    Integer(i64),
    BulkString(String),
    Null,
    Array(Vec<RespValue>),
}

#[allow(dead_code)]
impl RespValue {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            RespValue::Integer(i) => Some(*i),
            RespValue::BulkString(s) => s.parse().ok(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            RespValue::SimpleString(s) | RespValue::BulkString(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, RespValue::Null)
    }

    pub fn into_array(self) -> Option<Vec<RespValue>> {
        match self {
            RespValue::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            RespValue::SimpleString(s) => serde_json::Value::String(s.clone()),
            RespValue::Error(e) => serde_json::Value::String(e.clone()),
            RespValue::Integer(i) => serde_json::json!(*i),
            RespValue::BulkString(s) => serde_json::Value::String(s.clone()),
            RespValue::Null => serde_json::Value::Null,
            RespValue::Array(items) => {
                serde_json::Value::Array(items.iter().map(|v| v.to_json()).collect())
            }
        }
    }
}

pub(crate) fn encode_command(args: &[&str]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    buf.extend_from_slice(format!("*{}\r\n", args.len()).as_bytes());
    for arg in args {
        buf.extend_from_slice(format!("${}\r\n", arg.len()).as_bytes());
        buf.extend_from_slice(arg.as_bytes());
        buf.extend_from_slice(b"\r\n");
    }
    buf
}

fn read_line(reader: &mut BufReader<&TcpStream>) -> Result<String, String> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("RESP read error: {}", e))?;
    if line.ends_with("\r\n") {
        line.truncate(line.len() - 2);
    }
    Ok(line)
}

fn read_resp(reader: &mut BufReader<&TcpStream>) -> Result<RespValue, String> {
    let line = read_line(reader)?;
    if line.is_empty() {
        return Err("RESP: empty response".to_string());
    }

    let (prefix, payload) = line.split_at(1);
    match prefix {
        "+" => Ok(RespValue::SimpleString(payload.to_string())),
        "-" => Err(payload.to_string()),
        ":" => {
            let i = payload
                .parse::<i64>()
                .map_err(|e| format!("RESP integer parse error: {}", e))?;
            Ok(RespValue::Integer(i))
        }
        "$" => {
            let len = payload
                .parse::<i64>()
                .map_err(|e| format!("RESP bulk length error: {}", e))?;
            if len < 0 {
                return Ok(RespValue::Null);
            }
            let len = len as usize;
            let mut buf = vec![0u8; len + 2]; // +2 for \r\n
            reader
                .read_exact(&mut buf)
                .map_err(|e| format!("RESP bulk read error: {}", e))?;
            buf.truncate(len); // drop trailing \r\n
            let s = String::from_utf8(buf).map_err(|e| format!("RESP UTF-8 error: {}", e))?;
            Ok(RespValue::BulkString(s))
        }
        "*" => {
            let count = payload
                .parse::<i64>()
                .map_err(|e| format!("RESP array length error: {}", e))?;
            if count < 0 {
                return Ok(RespValue::Null);
            }
            let mut items = Vec::with_capacity(count as usize);
            for _ in 0..count {
                items.push(read_resp(reader)?);
            }
            Ok(RespValue::Array(items))
        }
        _ => Err(format!("RESP: unknown type byte '{}'", prefix)),
    }
}

pub(crate) struct RespPool {
    connections: Mutex<Vec<TcpStream>>,
    host: String,
    port: u16,
    auth_token: Option<String>,
}

impl RespPool {
    pub fn new(host: String, port: u16, auth_token: Option<String>) -> Self {
        Self {
            connections: Mutex::new(Vec::new()),
            host,
            port,
            auth_token,
        }
    }

    fn create_connection(&self) -> Result<TcpStream, String> {
        use std::net::ToSocketAddrs;
        let addr_str = format!("{}:{}", self.host, self.port);
        let sock_addr = addr_str
            .to_socket_addrs()
            .map_err(|e| format!("SoliKV resolve error '{}': {}", addr_str, e))?
            .next()
            .ok_or_else(|| format!("SoliKV: no addresses for '{}'", addr_str))?;
        let stream = TcpStream::connect_timeout(&sock_addr, CONNECT_TIMEOUT)
            .map_err(|e| format!("SoliKV connect error ({}): {}", addr_str, e))?;

        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|e| format!("SoliKV set_read_timeout: {}", e))?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|e| format!("SoliKV set_write_timeout: {}", e))?;
        stream
            .set_nodelay(true)
            .map_err(|e| format!("SoliKV set_nodelay: {}", e))?;

        // AUTH if token is configured
        if let Some(token) = &self.auth_token {
            let cmd = encode_command(&["AUTH", token]);
            (&stream)
                .write_all(&cmd)
                .map_err(|e| format!("SoliKV AUTH write: {}", e))?;
            let mut reader = BufReader::new(&stream);
            match read_resp(&mut reader) {
                Ok(_) => {}
                Err(e) => return Err(format!("SoliKV AUTH failed: {}", e)),
            }
        }

        Ok(stream)
    }

    fn get(&self) -> Result<TcpStream, String> {
        if let Ok(mut pool) = self.connections.lock() {
            if let Some(conn) = pool.pop() {
                return Ok(conn);
            }
        }
        self.create_connection()
    }

    fn put(&self, conn: TcpStream) {
        if let Ok(mut pool) = self.connections.lock() {
            if pool.len() < 32 {
                pool.push(conn);
            }
            // else drop the connection
        }
    }

    pub fn execute(&self, args: &[&str]) -> Result<RespValue, String> {
        let encoded = encode_command(args);

        let mut conn = self.get()?;

        // Try to write; on failure, reconnect once
        if (&conn).write_all(&encoded).is_err() {
            conn = self.create_connection()?;
            (&conn)
                .write_all(&encoded)
                .map_err(|e| format!("SoliKV write error: {}", e))?;
        }

        let mut reader = BufReader::new(&conn);
        match read_resp(&mut reader) {
            Ok(val) => {
                self.put(conn);
                Ok(val)
            }
            Err(e) => Err(e),
        }
    }
}
