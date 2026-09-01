use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
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
    let start = Instant::now();
    let mut command = Command::new(executable.path());
    command
        .current_dir(executable.directory())
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
    request.headers.get(SESSION_HEADER) == Some(&shared.token)
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
    use std::net::{Shutdown, TcpListener};

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
    fn run_route_compiles_and_executes_in_process() {
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
