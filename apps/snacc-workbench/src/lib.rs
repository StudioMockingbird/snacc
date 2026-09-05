use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const PAGE: &str = include_str!("../page.html");
const SNIPPETS: &str = include_str!(concat!(env!("OUT_DIR"), "/snippets.json"));
const SESSION_HEADER: &str = "x-snacc-session";
const MAX_REQUEST_BODY: usize = 320 * 1024;
const MAX_SOURCE: usize = 256 * 1024;
const MAX_STDIN: usize = 64 * 1024;
const MAX_STDOUT: usize = 1024 * 1024;
const MAX_STDERR: usize = 1024 * 1024;
const MAX_EXECUTION: Duration = Duration::from_secs(3);
const CSP: &str =
    "default-src 'none'; connect-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'";

#[derive(Clone)]
struct Shared {
    page: String,
    snippets: String,
    token: String,
    address: SocketAddr,
    active: Arc<AtomicBool>,
}

#[derive(Debug)]
struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

#[derive(Debug)]
struct HttpError {
    status: &'static str,
    message: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunRequest {
    source: String,
    #[serde(default)]
    stdin: String,
}

#[derive(Serialize)]
struct DiagnosticReport {
    phase: String,
    message: String,
    span: Option<SpanReport>,
}

#[derive(Serialize)]
struct SpanReport {
    #[serde(rename = "startByte")]
    start_byte: usize,
    #[serde(rename = "endByte")]
    end_byte: usize,
    start: PositionReport,
    end: PositionReport,
}

#[derive(Serialize)]
struct PositionReport {
    line: usize,
    column: usize,
}

#[derive(Serialize)]
struct CompileReport {
    status: &'static str,
    diagnostics: Vec<DiagnosticReport>,
    #[serde(rename = "durationMs")]
    duration_ms: f64,
}

#[derive(Serialize)]
struct ExecutionReport {
    status: &'static str,
    #[serde(rename = "exitCode")]
    exit_code: Option<i32>,
    #[serde(rename = "stdinBytes")]
    stdin_bytes: usize,
    stdout: String,
    stderr: String,
    #[serde(rename = "stdoutTruncated")]
    stdout_truncated: bool,
    #[serde(rename = "stderrTruncated")]
    stderr_truncated: bool,
    #[serde(rename = "durationMs")]
    duration_ms: f64,
}

#[derive(Serialize)]
struct RunResponse {
    compile: CompileReport,
    execution: Option<ExecutionReport>,
    #[serde(rename = "totalMs")]
    total_ms: f64,
}

struct Captured {
    bytes: Vec<u8>,
    truncated: bool,
}

struct BusyGuard(Arc<AtomicBool>);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub fn run() -> io::Result<()> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let address = listener.local_addr()?;
    let token = session_token()?;
    let page = PAGE.replace("__SNACC_SESSION_TOKEN__", &token);
    let shared = Arc::new(Shared {
        page,
        snippets: SNIPPETS.to_string(),
        token,
        address,
        active: Arc::new(AtomicBool::new(false)),
    });

    println!("snacc-workbench executes native programs; use trusted local snippets only.");
    println!("snacc-workbench listening at http://{address}/");

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let shared = Arc::clone(&shared);
                thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, &shared) {
                        eprintln!("snacc-workbench request failed: {error}");
                    }
                });
            }
            Err(error) => eprintln!("snacc-workbench accept failed: {error}"),
        }
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream, shared: &Shared) -> io::Result<()> {
    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err(error) => return write_error(&mut stream, error),
    };
    if request.headers.get("host") != Some(&format!("127.0.0.1:{}", shared.address.port())) {
        return write_error(
            &mut stream,
            HttpError {
                status: "400 Bad Request",
                message: "invalid Host header".into(),
            },
        );
    }

    match request.path.as_str() {
        "/" => {
            if request.method != "GET" {
                return write_method_error(&mut stream, "GET");
            }
            write_response(
                &mut stream,
                "200 OK",
                "text/html; charset=utf-8",
                shared.page.as_bytes(),
                &[
                    ("Cache-Control", "no-store"),
                    ("Content-Security-Policy", CSP),
                ],
            )
        }
        "/api/snippets" => {
            if request.method != "GET" {
                return write_method_error(&mut stream, "GET");
            }
            if !authorized(&request, shared) {
                return write_error(&mut stream, forbidden("missing or invalid session token"));
            }
            write_response(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                shared.snippets.as_bytes(),
                &[],
            )
        }
        "/api/run" => {
            if request.method != "POST" {
                return write_method_error(&mut stream, "POST");
            }
            if !authorized(&request, shared) {
                return write_error(&mut stream, forbidden("missing or invalid session token"));
            }
            let origin = format!("http://127.0.0.1:{}", shared.address.port());
            if request.headers.get("origin") != Some(&origin) {
                return write_error(&mut stream, forbidden("invalid Origin header"));
            }
            let content_type = request
                .headers
                .get("content-type")
                .map(String::as_str)
                .unwrap_or_default();
            if content_type.split(';').next().map(str::trim) != Some("application/json") {
                return write_error(
                    &mut stream,
                    HttpError {
                        status: "415 Unsupported Media Type",
                        message: "Content-Type must be application/json".into(),
                    },
                );
            }
            let run_request: RunRequest = match serde_json::from_slice(&request.body) {
                Ok(value) => value,
                Err(error) => {
                    return write_error(
                        &mut stream,
                        HttpError {
                            status: "400 Bad Request",
                            message: format!("invalid JSON request: {error}"),
                        },
                    );
                }
            };
            if run_request.source.trim().is_empty() {
                return write_error(
                    &mut stream,
                    HttpError {
                        status: "400 Bad Request",
                        message: "source must contain a non-whitespace character".into(),
                    },
                );
            }
            if run_request.source.len() > MAX_SOURCE {
                return write_error(
                    &mut stream,
                    HttpError {
                        status: "413 Payload Too Large",
                        message: "source exceeds the 256 KiB limit".into(),
                    },
                );
            }
            if run_request.stdin.len() > MAX_STDIN {
                return write_error(
                    &mut stream,
                    HttpError {
                        status: "413 Payload Too Large",
                        message: "stdin exceeds the 64 KiB limit".into(),
                    },
                );
            }
            if shared
                .active
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return write_error(
                    &mut stream,
                    HttpError {
                        status: "429 Too Many Requests",
                        message: "another run is already active".into(),
                    },
                );
            }
            let _busy = BusyGuard(Arc::clone(&shared.active));
            let response = run_program(&run_request.source, &run_request.stdin);
            let body = serde_json::to_vec(&response).map_err(io::Error::other)?;
            write_response(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                &body,
                &[],
            )
        }
        _ => write_error(
            &mut stream,
            HttpError {
                status: "404 Not Found",
                message: "not found".into(),
            },
        ),
    }
}

fn run_program(source: &str, stdin: &str) -> RunResponse {
    let total_start = Instant::now();
    let compile_start = Instant::now();
    let built = match snacc_driver::build(source) {
        Ok(built) => built,
        Err(snacc_driver::DriverError::Compile(diagnostics)) => {
            return RunResponse {
                compile: CompileReport {
                    status: "failed",
                    diagnostics: diagnostic_reports(source, &diagnostics),
                    duration_ms: milliseconds(compile_start.elapsed()),
                },
                execution: None,
                total_ms: milliseconds(total_start.elapsed()),
            };
        }
        Err(error) => {
            return RunResponse {
                compile: CompileReport {
                    status: "failed",
                    diagnostics: vec![DiagnosticReport {
                        phase: "build".into(),
                        message: error.to_string(),
                        span: None,
                    }],
                    duration_ms: milliseconds(compile_start.elapsed()),
                },
                execution: None,
                total_ms: milliseconds(total_start.elapsed()),
            };
        }
    };
    let compile_duration = milliseconds(compile_start.elapsed());
    let execution = execute(&built, stdin);
    RunResponse {
        compile: CompileReport {
            status: "succeeded",
            diagnostics: Vec::new(),
            duration_ms: compile_duration,
        },
        execution: Some(execution),
        total_ms: milliseconds(total_start.elapsed()),
    }
}

fn execute(executable: &snacc_driver::BuiltExecutable, stdin: &str) -> ExecutionReport {
    // Split so process/pipe behavior (stdin delivery, threaded draining, timeout
    // kill) is testable against any executable, not only one produced by the
    // Snacc compiler pipeline that `BuiltExecutable` requires.
    run_process(executable.path(), executable.directory(), stdin)
}

fn run_process(path: &Path, working_dir: &Path, stdin: &str) -> ExecutionReport {
    let start = Instant::now();
    let mut command = Command::new(path);
    command
        .current_dir(working_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ExecutionReport {
                status: "failed-to-start",
                exit_code: None,
                stdin_bytes: stdin.len(),
                stdout: String::new(),
                stderr: error.to_string(),
                stdout_truncated: false,
                stderr_truncated: false,
                duration_ms: milliseconds(start.elapsed()),
            };
        }
    };

    let stdout = child.stdout.take().expect("spawned child has stdout pipe");
    let stderr = child.stderr.take().expect("spawned child has stderr pipe");
    let child_stdin = child.stdin.take().expect("spawned child has stdin pipe");
    let stdout_limited = Arc::new(AtomicBool::new(false));
    let stderr_limited = Arc::new(AtomicBool::new(false));
    let stdout_limited_for_thread = Arc::clone(&stdout_limited);
    let stderr_limited_for_thread = Arc::clone(&stderr_limited);
    let stdout_thread =
        thread::spawn(move || capture_stream(stdout, MAX_STDOUT, stdout_limited_for_thread));
    let stderr_thread =
        thread::spawn(move || capture_stream(stderr, MAX_STDERR, stderr_limited_for_thread));
    let stdin_bytes = stdin.len();
    let stdin_data = stdin.as_bytes().to_vec();
    let stdin_thread = thread::spawn(move || {
        let mut writer = child_stdin;
        let _ = writer.write_all(&stdin_data);
    });

    let process_status;
    let mut final_status = "exited";
    loop {
        if stdout_limited.load(Ordering::Acquire) || stderr_limited.load(Ordering::Acquire) {
            final_status = "output-limit";
            let _ = child.kill();
            process_status = child.wait().ok();
            break;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                process_status = Some(status);
                break;
            }
            Ok(None) if start.elapsed() >= MAX_EXECUTION => {
                final_status = "timed-out";
                let _ = child.kill();
                process_status = child.wait().ok();
                break;
            }
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            Err(_) => {
                final_status = "failed-to-start";
                let _ = child.kill();
                process_status = child.wait().ok();
                break;
            }
        }
    }
    let _ = stdin_thread.join();
    let captured_stdout = stdout_thread.join().unwrap_or(Captured {
        bytes: Vec::new(),
        truncated: false,
    });
    let captured_stderr = stderr_thread.join().unwrap_or(Captured {
        bytes: Vec::new(),
        truncated: false,
    });
    if final_status == "exited" && (captured_stdout.truncated || captured_stderr.truncated) {
        final_status = "output-limit";
    }
    ExecutionReport {
        status: final_status,
        exit_code: process_status.and_then(|status| status.code()),
        stdin_bytes,
        stdout: String::from_utf8_lossy(&captured_stdout.bytes).into_owned(),
        stderr: String::from_utf8_lossy(&captured_stderr.bytes).into_owned(),
        stdout_truncated: captured_stdout.truncated,
        stderr_truncated: captured_stderr.truncated,
        duration_ms: milliseconds(start.elapsed()),
    }
}

fn capture_stream<R: Read>(mut reader: R, limit: usize, limited: Arc<AtomicBool>) -> Captured {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 16 * 1024];
    let mut truncated = false;
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        if bytes.len() < limit {
            let remaining = limit - bytes.len();
            let keep = remaining.min(count);
            bytes.extend_from_slice(&buffer[..keep]);
            if keep < count {
                truncated = true;
                limited.store(true, Ordering::Release);
            }
        } else {
            truncated = true;
            limited.store(true, Ordering::Release);
        }
    }
    Captured { bytes, truncated }
}

fn diagnostic_reports(
    source: &str,
    diagnostics: &snacc_compiler::Diagnostics,
) -> Vec<DiagnosticReport> {
    diagnostics
        .items
        .iter()
        .map(|diagnostic| {
            let span = diagnostic.span.as_ref().map(|span| SpanReport {
                start_byte: span.start,
                end_byte: span.end,
                start: position(source, span.start),
                end: position(source, span.end),
            });
            DiagnosticReport {
                phase: format!("{:?}", diagnostic.phase).to_ascii_lowercase(),
                message: diagnostic.message.clone(),
                span,
            }
        })
        .collect()
}

fn position(source: &str, offset: usize) -> PositionReport {
    let offset = offset.min(source.len());
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .map(|line| line.chars().count() + 1)
        .unwrap_or(1);
    PositionReport { line, column }
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn session_token() -> io::Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        token.push_str(&format!("{byte:02x}"));
    }
    Ok(token)
}

fn read_request(stream: &mut TcpStream) -> Result<Request, HttpError> {
    let mut raw = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).map_err(io_http_error)?;
        if count == 0 {
            return Err(HttpError {
                status: "400 Bad Request",
                message: "request ended before headers".into(),
            });
        }
        raw.extend_from_slice(&chunk[..count]);
        if let Some(index) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            break index;
        }
        if raw.len() > 64 * 1024 {
            return Err(HttpError {
                status: "413 Payload Too Large",
                message: "request headers are too large".into(),
            });
        }
    };
    let header_text = std::str::from_utf8(&raw[..header_end]).map_err(|_| HttpError {
        status: "400 Bad Request",
        message: "request headers are not valid UTF-8".into(),
    })?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or_else(|| HttpError {
        status: "400 Bad Request",
        message: "request line is missing".into(),
    })?;
    let parts = request_line.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 || parts[2] != "HTTP/1.1" {
        return Err(HttpError {
            status: "400 Bad Request",
            message: "only HTTP/1.1 requests are supported".into(),
        });
    }
    let mut headers = HashMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(HttpError {
                status: "400 Bad Request",
                message: "malformed request header".into(),
            });
        };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    let content_length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|_| HttpError {
            status: "400 Bad Request",
            message: "invalid Content-Length".into(),
        })?
        .unwrap_or(0);
    if content_length > MAX_REQUEST_BODY {
        return Err(HttpError {
            status: "413 Payload Too Large",
            message: "request body exceeds the 320 KiB limit".into(),
        });
    }
    let body_start = header_end + 4;
    let mut body = raw[body_start..].to_vec();
    if body.len() > content_length {
        body.truncate(content_length);
    }
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let mut rest = vec![0_u8; remaining];
        stream.read_exact(&mut rest).map_err(io_http_error)?;
        body.extend_from_slice(&rest);
    }
    body.truncate(content_length);
    Ok(Request {
        method: parts[0].to_string(),
        path: parts[1].to_string(),
        headers,
        body,
    })
}

fn authorized(request: &Request, shared: &Shared) -> bool {
    match request.headers.get(SESSION_HEADER) {
        Some(token) => constant_time_eq(token, &shared.token),
        None => false,
    }
}

/// Compares the session token in constant time: `==` on `String` short-circuits
/// at the first differing byte, which would let a network attacker recover the
/// token one byte at a time from response-timing differences. The length check
/// does not leak secret information since the expected token's length is fixed
/// and public (32 random bytes, hex-encoded).
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn forbidden(message: &str) -> HttpError {
    HttpError {
        status: "403 Forbidden",
        message: message.into(),
    }
}

fn io_http_error(error: io::Error) -> HttpError {
    HttpError {
        status: "400 Bad Request",
        message: format!("failed to read request: {error}"),
    }
}

fn write_method_error(stream: &mut TcpStream, allowed: &str) -> io::Result<()> {
    write_response(
        stream,
        "405 Method Not Allowed",
        "application/json; charset=utf-8",
        br#"{"error":"method not allowed"}"#,
        &[("Allow", allowed)],
    )
}

fn write_error(stream: &mut TcpStream, error: HttpError) -> io::Result<()> {
    let body = serde_json::json!({"error": error.message});
    let body = serde_json::to_vec(&body).map_err(io::Error::other)?;
    write_response(
        stream,
        error.status,
        "application/json; charset=utf-8",
        &body,
        &[],
    )
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    extra_headers: &[(&str, &str)],
) -> io::Result<()> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        response.push_str(name);
        response.push_str(": ");
        response.push_str(value);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes())?;
    stream.write_all(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        net::{Shutdown, TcpListener},
        path::PathBuf,
    };

    /// Serializes tests that invoke a real `rustc` compile+link (through
    /// `snacc_driver::build()` or a hand-compiled process-test helper).
    /// Running several of these concurrently overloads the linker on a
    /// modest machine, which surfaces as spurious "failed-to-start" or
    /// compile-tool failures unrelated to the behavior under test.
    static BUILD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn request(raw: String, token: &str) -> String {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let raw = raw.replace("127.0.0.1:0", &format!("127.0.0.1:{}", address.port()));
        let shared = Shared {
            page: PAGE.replace("__SNACC_SESSION_TOKEN__", token),
            snippets: SNIPPETS.to_string(),
            token: token.to_string(),
            address,
            active: Arc::new(AtomicBool::new(false)),
        };
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(stream, &shared).unwrap();
        });
        let mut client = TcpStream::connect(address).unwrap();
        client.write_all(raw.as_bytes()).unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        server.join().unwrap();
        response
    }

    #[test]
    fn positions_are_one_based_and_use_exact_source_bytes() {
        let source = "α\nprint(1)";
        assert_eq!(position(source, 0).line, 1);
        assert_eq!(position(source, 2).line, 1);
        assert_eq!(position(source, 3).line, 2);
        assert_eq!(position(source, source.len()).column, 9);
    }

    #[test]
    fn capture_stream_bounds_output_and_continues_draining() {
        let limited = Arc::new(AtomicBool::new(false));
        let captured = capture_stream(std::io::Cursor::new(b"abcdef"), 3, Arc::clone(&limited));
        assert_eq!(captured.bytes, b"abc");
        assert!(captured.truncated);
        assert!(limited.load(Ordering::Acquire));
    }

    #[test]
    fn html_and_snippet_routes_serve_embedded_content() {
        let token = "a".repeat(64);
        let html = request(
            format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:0\r\n\r\n"),
            &token,
        );
        assert!(html.contains("200 OK"));
        assert!(html.contains("Snacc Workbench"));

        let response = request(
            format!(
                "GET /api/snippets HTTP/1.1\r\nHost: 127.0.0.1:0\r\nX-Snacc-Session: {}\r\n\r\n",
                token
            ),
            &token,
        );
        assert!(response.contains("200 OK"));
        assert!(response.contains("arithmetic"));
    }

    #[test]
    fn constant_time_eq_matches_naive_equality() {
        assert!(constant_time_eq("same-token", "same-token"));
        assert!(!constant_time_eq("token-a", "token-b"));
        assert!(!constant_time_eq("short", "longer-value"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn missing_session_token_is_rejected() {
        let token = "c".repeat(64);
        let response = request(
            "GET /api/snippets HTTP/1.1\r\nHost: 127.0.0.1:0\r\n\r\n".to_string(),
            &token,
        );
        assert!(response.contains("403 Forbidden"));
        assert!(response.contains("missing or invalid session token"));
    }

    #[test]
    fn wrong_session_token_is_rejected() {
        let token = "d".repeat(64);
        let wrong = "e".repeat(64);
        let response = request(
            format!(
                "GET /api/snippets HTTP/1.1\r\nHost: 127.0.0.1:0\r\nX-Snacc-Session: {wrong}\r\n\r\n"
            ),
            &token,
        );
        assert!(response.contains("403 Forbidden"));
        assert!(response.contains("missing or invalid session token"));
    }

    #[test]
    fn foreign_origin_is_rejected() {
        let token = "f".repeat(64);
        let body = r#"{"source":"print(1)","stdin":""}"#;
        let response = request(
            format!(
                "POST /api/run HTTP/1.1\r\nHost: 127.0.0.1:0\r\nOrigin: http://evil.example\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Snacc-Session: {}\r\n\r\n{}",
                body.len(),
                token,
                body
            ),
            &token,
        );
        assert!(response.contains("403 Forbidden"));
        assert!(response.contains("invalid Origin header"));
    }

    #[test]
    fn mismatched_host_is_rejected() {
        let token = "g".repeat(64);
        let response = request(
            "GET / HTTP/1.1\r\nHost: example.com\r\n\r\n".to_string(),
            &token,
        );
        assert!(response.contains("400 Bad Request"));
        assert!(response.contains("invalid Host header"));
    }

    #[test]
    fn unsupported_method_returns_405_with_allow_header() {
        let token = "h".repeat(64);

        let response = request(
            "POST / HTTP/1.1\r\nHost: 127.0.0.1:0\r\n\r\n".to_string(),
            &token,
        );
        assert!(response.contains("405 Method Not Allowed"));
        assert!(response.contains("Allow: GET"));

        let response = request(
            "POST /api/snippets HTTP/1.1\r\nHost: 127.0.0.1:0\r\n\r\n".to_string(),
            &token,
        );
        assert!(response.contains("405 Method Not Allowed"));
        assert!(response.contains("Allow: GET"));

        let response = request(
            "GET /api/run HTTP/1.1\r\nHost: 127.0.0.1:0\r\n\r\n".to_string(),
            &token,
        );
        assert!(response.contains("405 Method Not Allowed"));
        assert!(response.contains("Allow: POST"));
    }

    #[test]
    fn non_json_content_type_is_rejected() {
        let token = "i".repeat(64);
        let body = "not json";
        let response = request(
            format!(
                "POST /api/run HTTP/1.1\r\nHost: 127.0.0.1:0\r\nOrigin: http://127.0.0.1:0\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nX-Snacc-Session: {}\r\n\r\n{}",
                body.len(),
                token,
                body
            ),
            &token,
        );
        assert!(response.contains("415 Unsupported Media Type"));
        assert!(response.contains("Content-Type must be application/json"));
    }

    #[test]
    fn malformed_json_body_is_rejected() {
        let token = "j".repeat(64);
        let body = "{not valid json";
        let response = request(
            format!(
                "POST /api/run HTTP/1.1\r\nHost: 127.0.0.1:0\r\nOrigin: http://127.0.0.1:0\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Snacc-Session: {}\r\n\r\n{}",
                body.len(),
                token,
                body
            ),
            &token,
        );
        assert!(response.contains("400 Bad Request"));
        assert!(response.contains("invalid JSON request"));
    }

    #[test]
    fn unknown_json_fields_are_rejected() {
        let token = "k".repeat(64);
        let body = r#"{"source":"print(1)","stdin":"","extra":true}"#;
        let response = request(
            format!(
                "POST /api/run HTTP/1.1\r\nHost: 127.0.0.1:0\r\nOrigin: http://127.0.0.1:0\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Snacc-Session: {}\r\n\r\n{}",
                body.len(),
                token,
                body
            ),
            &token,
        );
        assert!(response.contains("400 Bad Request"));
        assert!(response.contains("invalid JSON request"));
    }

    #[test]
    fn oversized_request_headers_are_rejected() {
        let token = "l".repeat(64);
        let prefix = "GET / HTTP/1.1\r\nHost: 127.0.0.1:0\r\nX-Big: ";
        // No closing CRLFCRLF: the terminator search must never succeed, so
        // the size check is what trips. The total length is kept just past
        // the 64 KiB threshold (not far past it) so the server's read loop
        // consumes every byte before erroring -- leaving no unread trailing
        // data, which would otherwise reset the connection instead of
        // closing it cleanly and fail the client's write with ECONNRESET.
        let total_len = 64 * 1024 + 1;
        let raw = format!("{prefix}{}", "a".repeat(total_len - prefix.len()));
        let response = request(raw, &token);
        assert!(response.contains("413 Payload Too Large"));
        assert!(response.contains("request headers are too large"));
    }

    #[test]
    fn oversized_request_body_is_rejected_from_content_length_alone() {
        let token = "m".repeat(64);
        // The Content-Length claim alone must trigger rejection before the
        // server tries to read that many body bytes, so this stays a cheap,
        // fast test instead of transmitting 320 KiB+.
        let response = request(
            format!(
                "POST /api/run HTTP/1.1\r\nHost: 127.0.0.1:0\r\nOrigin: http://127.0.0.1:0\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Snacc-Session: {}\r\n\r\n",
                MAX_REQUEST_BODY + 1,
                token
            ),
            &token,
        );
        assert!(response.contains("413 Payload Too Large"));
        assert!(response.contains("request body exceeds the 320 KiB limit"));
    }

    #[test]
    fn oversized_source_is_rejected() {
        let token = "n".repeat(64);
        let big_source = "a".repeat(MAX_SOURCE + 1);
        let body = serde_json::json!({"source": big_source, "stdin": ""}).to_string();
        let response = request(
            format!(
                "POST /api/run HTTP/1.1\r\nHost: 127.0.0.1:0\r\nOrigin: http://127.0.0.1:0\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Snacc-Session: {}\r\n\r\n{}",
                body.len(),
                token,
                body
            ),
            &token,
        );
        assert!(response.contains("413 Payload Too Large"));
        assert!(response.contains("source exceeds the 256 KiB limit"));
    }

    #[test]
    fn oversized_stdin_is_rejected() {
        let token = "o".repeat(64);
        let big_stdin = "a".repeat(MAX_STDIN + 1);
        let body = serde_json::json!({"source": "print(1)", "stdin": big_stdin}).to_string();
        let response = request(
            format!(
                "POST /api/run HTTP/1.1\r\nHost: 127.0.0.1:0\r\nOrigin: http://127.0.0.1:0\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Snacc-Session: {}\r\n\r\n{}",
                body.len(),
                token,
                body
            ),
            &token,
        );
        assert!(response.contains("413 Payload Too Large"));
        assert!(response.contains("stdin exceeds the 64 KiB limit"));
    }

    #[test]
    fn compile_failure_yields_null_execution() {
        let token = "p".repeat(64);
        let body = serde_json::json!({"source": "while nil do false end", "stdin": ""}).to_string();
        let response = request(
            format!(
                "POST /api/run HTTP/1.1\r\nHost: 127.0.0.1:0\r\nOrigin: http://127.0.0.1:0\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Snacc-Session: {}\r\n\r\n{}",
                body.len(),
                token,
                body
            ),
            &token,
        );
        assert!(response.contains("200 OK"));
        assert!(response.contains(r#""status":"failed""#));
        assert!(response.contains(r#""execution":null"#));
    }

    fn spawn_accepting_server(
        mut shared: Shared,
        connections: usize,
    ) -> (SocketAddr, thread::JoinHandle<()>) {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        shared.address = address;
        let shared = Arc::new(shared);
        let handle = thread::spawn(move || {
            for _ in 0..connections {
                let (stream, _) = listener.accept().unwrap();
                let shared = Arc::clone(&shared);
                thread::spawn(move || handle_connection(stream, &shared).unwrap());
            }
        });
        (address, handle)
    }

    fn send_raw(address: SocketAddr, raw: &str) -> String {
        let mut client = TcpStream::connect(address).unwrap();
        client.write_all(raw.as_bytes()).unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn second_run_while_one_is_active_receives_429() {
        let _guard = BUILD_LOCK.lock().unwrap();
        let token = "q".repeat(64);
        let shared = Shared {
            page: PAGE.replace("__SNACC_SESSION_TOKEN__", &token),
            snippets: SNIPPETS.to_string(),
            token: token.clone(),
            address: "127.0.0.1:0".parse().unwrap(),
            active: Arc::new(AtomicBool::new(false)),
        };
        let (address, server) = spawn_accepting_server(shared, 2);

        // An infinite loop keeps `active` set for the full MAX_EXECUTION
        // window, giving the second request a wide, non-flaky margin to land
        // while the first is still running.
        let slow_body =
            serde_json::json!({"source": "while true do 1 end", "stdin": ""}).to_string();
        let slow_raw = format!(
            "POST /api/run HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Snacc-Session: {token}\r\n\r\n{slow_body}",
            slow_body.len()
        );
        let first = thread::spawn(move || send_raw(address, &slow_raw));

        thread::sleep(Duration::from_millis(300));

        let quick_body = serde_json::json!({"source": "print(1)", "stdin": ""}).to_string();
        let quick_raw = format!(
            "POST /api/run HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Snacc-Session: {token}\r\n\r\n{quick_body}",
            quick_body.len()
        );
        let second_response = send_raw(address, &quick_raw);
        assert!(second_response.contains("429 Too Many Requests"));
        assert!(second_response.contains("another run is already active"));

        let first_response = first.join().unwrap();
        assert!(first_response.contains(r#""status":"timed-out""#));
        server.join().unwrap();
    }

    #[test]
    fn timeout_kills_child_and_reports_timed_out() {
        let _guard = BUILD_LOCK.lock().unwrap();
        let token = "r".repeat(64);
        let body = serde_json::json!({"source": "while true do 1 end", "stdin": ""}).to_string();
        let response = request(
            format!(
                "POST /api/run HTTP/1.1\r\nHost: 127.0.0.1:0\r\nOrigin: http://127.0.0.1:0\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Snacc-Session: {}\r\n\r\n{}",
                body.len(),
                token,
                body
            ),
            &token,
        );
        // Reaching this assertion at all proves child.wait() returned instead
        // of hanging forever after the kill.
        assert!(response.contains("200 OK"));
        assert!(response.contains(r#""status":"timed-out""#));
    }

    #[test]
    fn build_directory_with_a_space_still_compiles_and_runs() {
        // BUILD_LOCK also serializes this against every other build-invoking
        // test: it is the only test that mutates process-wide
        // TMP/TEMP/TMPDIR, so nothing else may observe the override mid-flight.
        let _guard = BUILD_LOCK.lock().unwrap();

        let spaced_dir =
            std::env::temp_dir().join(format!("snacc workbench build {}", std::process::id()));
        fs::create_dir_all(&spaced_dir).expect("failed to create spaced scratch directory");

        let original = (
            std::env::var_os("TMP"),
            std::env::var_os("TEMP"),
            std::env::var_os("TMPDIR"),
        );
        // SAFETY: ENV_LOCK (held above) prevents this from racing the restore
        // at the end of this same test, the only other env mutation here.
        unsafe {
            std::env::set_var("TMP", &spaced_dir);
            std::env::set_var("TEMP", &spaced_dir);
            std::env::set_var("TMPDIR", &spaced_dir);
        }

        let token = "s".repeat(64);
        let body = serde_json::json!({"source": "print(1 + 2)", "stdin": ""}).to_string();
        let response = request(
            format!(
                "POST /api/run HTTP/1.1\r\nHost: 127.0.0.1:0\r\nOrigin: http://127.0.0.1:0\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Snacc-Session: {}\r\n\r\n{}",
                body.len(),
                token,
                body
            ),
            &token,
        );

        // SAFETY: same justification as above; restores the pre-test values.
        unsafe {
            match original.0 {
                Some(value) => std::env::set_var("TMP", value),
                None => std::env::remove_var("TMP"),
            }
            match original.1 {
                Some(value) => std::env::set_var("TEMP", value),
                None => std::env::remove_var("TEMP"),
            }
            match original.2 {
                Some(value) => std::env::set_var("TMPDIR", value),
                None => std::env::remove_var("TMPDIR"),
            }
        }
        let _ = fs::remove_dir_all(&spaced_dir);

        assert!(response.contains("200 OK"), "response: {response}");
        assert!(
            response.contains(r#""status":"succeeded""#),
            "response: {response}"
        );
        assert!(
            response.contains(r#""stdout":"3\n""#),
            "response: {response}"
        );
    }

    fn scratch_dir(name: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "snacc-workbench-test-{}-{name}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("failed to create scratch directory for workbench test");
        dir
    }

    /// Compiles a standalone Rust helper program (not a Snacc program) so
    /// process-piping behavior (stdin delivery, threaded stdout/stderr
    /// draining) can be exercised directly through `run_process`: the Snacc
    /// language itself has no builtin for reading stdin or writing stderr, so
    /// only a hand-written child can exhibit that behavior on purpose.
    fn compile_rust_helper(name: &str, source: &str) -> (PathBuf, PathBuf) {
        let dir = scratch_dir(name);
        let src_path = dir.join("main.rs");
        fs::write(&src_path, source).expect("failed to write process-test helper source");
        let exe_path = dir.join(format!("helper{}", std::env::consts::EXE_SUFFIX));
        let status = Command::new("rustc")
            .arg("--edition=2021")
            .arg(&src_path)
            .arg("-o")
            .arg(&exe_path)
            .status()
            .expect("rustc must be on PATH to build workbench process-test helpers");
        assert!(
            status.success(),
            "failed to compile process-test helper '{name}'"
        );
        (dir, exe_path)
    }

    #[test]
    fn stdin_reaches_child_and_child_observes_eof() {
        let _guard = BUILD_LOCK.lock().unwrap();
        let (dir, exe) = compile_rust_helper(
            "stdin_echo",
            r#"
fn main() {
    use std::io::Read;
    let mut buf = Vec::new();
    std::io::stdin().read_to_end(&mut buf).expect("read stdin to EOF");
    println!("{}", buf.len());
}
"#,
        );
        let payload = "hello snacc workbench ".repeat(500);
        let report = run_process(&exe, &dir, &payload);
        assert_eq!(report.status, "exited");
        assert_eq!(report.exit_code, Some(0));
        assert_eq!(report.stdin_bytes, payload.len());
        // The child only prints this after read_to_end() returns, which only
        // happens once it observes EOF -- so a correct count here proves both
        // that the exact bytes arrived and that EOF was signaled.
        assert_eq!(report.stdout.trim(), payload.len().to_string());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn large_simultaneous_stdout_and_stderr_do_not_deadlock() {
        let _guard = BUILD_LOCK.lock().unwrap();
        let (dir, exe) = compile_rust_helper(
            "dual_stream_flood",
            r#"
fn main() {
    use std::io::Write;
    let out_chunk = vec![b'o'; 8192];
    let err_chunk = vec![b'e'; 8192];
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    for _ in 0..40 {
        stdout.write_all(&out_chunk).unwrap();
        stderr.write_all(&err_chunk).unwrap();
    }
}
"#,
        );
        // 40 * 8192 = 320 KiB per stream: comfortably past typical OS pipe
        // buffers (~64 KiB) so a regression to sequential (non-threaded)
        // draining would deadlock here, and comfortably under MAX_STDOUT /
        // MAX_STDERR (1 MiB) so this test is purely about the deadlock, not
        // truncation.
        let report = run_process(&exe, &dir, "");
        assert_eq!(report.status, "exited");
        assert_eq!(report.exit_code, Some(0));
        assert_eq!(report.stdout.len(), 40 * 8192);
        assert_eq!(report.stderr.len(), 40 * 8192);
        assert!(!report.stdout_truncated);
        assert!(!report.stderr_truncated);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_route_compiles_and_executes_in_process() {
        let _guard = BUILD_LOCK.lock().unwrap();
        let token = "b".repeat(64);
        let body = r#"{"source":"print(1 + 2)","stdin":""}"#;
        let response = request(
            format!(
                "POST /api/run HTTP/1.1\r\nHost: 127.0.0.1:0\r\nOrigin: http://127.0.0.1:0\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Snacc-Session: {}\r\n\r\n{}",
                body.len(),
                token,
                body
            ),
            &token,
        );
        assert!(response.contains("200 OK"));
        assert!(response.contains(r#""status":"exited""#));
        assert!(response.contains(r#""stdout":"3\n""#));
    }
}
