//! A stub SurrealDB for the tests that do not need a real one: `POST /sql` (`INFO FOR DB;`) answers a table list, and
//! `POST /export` answers a line naming what was asked, like the Python's `_stub_surreal`. It records every request
//! so a test can say what the job asked for, with which headers.

#![allow(dead_code, missing_docs, clippy::pedantic)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// How `/export` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// An export-looking text, as long as `rows` says.
    Normal,
    /// Text that is not an export.
    NotAnExport,
    /// A chunked answer that stops halfway and drops the connection.
    Cut,
    /// This status and a one-line body.
    Status(u16),
}

/// One request seen.
#[derive(Debug, Clone)]
pub struct Seen {
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

struct State {
    tables: Vec<String>,
    mode: Mode,
    rows: usize,
    seen: Vec<Seen>,
    stop: AtomicBool,
}

pub struct SurrealStub {
    pub url: String,
    addr: std::net::SocketAddr,
    st: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
}

impl Drop for SurrealStub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
    }
}

impl SurrealStub {
    pub fn start(tables: &[&str], mode: Mode, rows: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let st = Arc::new(Mutex::new(State {
            tables: tables.iter().map(|t| (*t).to_string()).collect(),
            mode,
            rows,
            seen: Vec::new(),
            stop: AtomicBool::new(false),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (s2, stop2) = (st.clone(), stop.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(c) = conn else { continue };
                let s3 = s2.clone();
                std::thread::spawn(move || {
                    let _ = serve(&s3, c);
                });
            }
        });
        Self {
            url: format!("http://127.0.0.1:{}", addr.port()),
            addr,
            st,
            stop,
        }
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.st.lock().unwrap_or_else(PoisonError::into_inner).seen.clone()
    }

    pub fn set_mode(&self, mode: Mode) {
        self.st.lock().unwrap_or_else(PoisonError::into_inner).mode = mode;
    }
}

fn serve(st: &Arc<Mutex<State>>, tcp: TcpStream) -> std::io::Result<()> {
    let mut r = BufReader::new(tcp.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
    let mut headers = BTreeMap::new();
    loop {
        let mut h = String::new();
        r.read_line(&mut h)?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    let body = String::from_utf8_lossy(&body).into_owned();
    let (tables, mode, rows) = {
        let mut g = st.lock().unwrap_or_else(PoisonError::into_inner);
        g.seen.push(Seen {
            path: path.clone(),
            headers,
            body: body.clone(),
        });
        (g.tables.clone(), g.mode, g.rows)
    };
    let mut w = tcp;
    if path.ends_with("/sql") {
        let map: serde_json::Map<String, serde_json::Value> = tables
            .iter()
            .map(|t| (t.clone(), serde_json::Value::String(format!("DEFINE TABLE {t}"))))
            .collect();
        let out = serde_json::json!([{"status": "OK", "result": {"tables": map}}]).to_string();
        return reply(&mut w, 200, "application/json", out.as_bytes());
    }
    if path.ends_with("/export") {
        let cfg: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        let asked: Vec<String> = cfg["tables"]
            .as_array()
            .map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let records = cfg.get("records").and_then(|v| v.as_bool()).unwrap_or(true);
        let mut text = String::from(
            "-- ------------------------------\n-- OPTION\n-- ------------------------------\n\nOPTION IMPORT;\n\n",
        );
        for t in &asked {
            text.push_str(&format!("DEFINE TABLE {t} TYPE NORMAL SCHEMAFULL PERMISSIONS NONE;\n"));
        }
        if records {
            for t in &asked {
                for i in 0..rows {
                    text.push_str(&format!("INSERT [ {{ id: {t}:r{i}, text: 'row {i} of {t}' }} ];\n"));
                }
            }
        }
        match mode {
            Mode::Normal => return reply(&mut w, 200, "text/plain", text.as_bytes()),
            Mode::NotAnExport => return reply(&mut w, 200, "text/plain", "hello;\n".repeat(2000).as_bytes()),
            Mode::Status(code) => return reply(&mut w, code, "text/plain", b"not allowed"),
            Mode::Cut => {
                let half = &text.as_bytes()[..text.len() / 2];
                write!(
                    w,
                    "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n{:x}\r\n",
                    half.len()
                )?;
                w.write_all(half)?;
                w.write_all(b"\r\n")?;
                w.flush()?;
                return Ok(()); // no last chunk: the stream is dropped
            }
        }
    }
    reply(&mut w, 404, "text/plain", b"no")
}

fn reply(w: &mut TcpStream, status: u16, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        w,
        "HTTP/1.1 {status} X\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )?;
    w.write_all(body)?;
    w.flush()
}
