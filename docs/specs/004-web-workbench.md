# RFC 004: Local Web Workbench

Status: Completed

## Summary

Snacc will provide a local web workbench for editing, compiling, and running
small in-memory Snacc programs. The browser sends a source string and a
pre-supplied standard-input string to a loopback-only Rust server. The server
passes the source string directly to the Snacc compiler library, builds a
temporary executable through the active native-driver pipeline, runs it, and
returns compiler diagnostics plus the process's standard output, standard
error, exit status, and timings.

~~~text
Browser
  source string + stdin string
          |
          v
Local workbench server
  -> native driver
     -> snacc compiler library
     -> native object
     -> temporary executable
  -> child process with supplied stdin
          |
          v
Browser
  compiler diagnostics
  stdout | stderr | exit status | timings
~~~

`print` is the initial observable program operation and writes values to
standard output through the existing Rust runtime. Snacc does not yet expose an
input operation, but the workbench supplies and closes standard input correctly
from the first version so a future input feature does not require an API or UI
redesign.

The workbench is a local development tool. It is not a hosted playground and
does not claim to sandbox hostile native programs.

## Motivation

The workbench provides the shortest feedback loop for exploring language
features:

1. Select or type a small program.
2. Optionally provide standard input.
3. Run it.
4. Read the exact standard output, standard error, and exit result.

The design incorporates the useful properties of the Hexal workbench:

- The HTTP layer calls the compiler's in-memory API instead of invoking a CLI
  with a source file.
- The page is embedded in the server binary.
- Snippets reuse executable compiler conformance examples rather than defining
  a second source and expectation corpus.
- Compiler responses are structured JSON rather than parsed console text.
- The server binds only to loopback.
- Frontend, compiler, API, and process behavior have focused tests.

Snacc differs from Hexal because the primary output is an executed native
program rather than a collection of generated source files. Native execution
requires explicit process control, resource limits, stream capture, and a
strong distinction between compiler diagnostics and program standard error.

## Goals

- Compile Snacc directly from a UTF-8 string through the compiler library.
- Run the resulting native program without writing the source to a project
  file.
- Accept a pre-supplied stdin string and close stdin after writing it.
- Capture stdout and stderr independently without deadlock.
- Display compiler diagnostics separately from program stderr.
- Report the exit code, timeout state, output truncation, and phase timings.
- Provide small snippets for supported language features.
- Source every catalog snippet from an existing executable compiler
  conformance example.
- Reuse the native compilation and linking path used by the Snacc CLI.
- Keep the HTTP and browser implementation outside the compiler pipeline.
- Remain responsive and deterministic across repeated runs.
- Restrict native-code execution to an authenticated loopback session.

## Non-goals

- Hosting the workbench on a public or shared server.
- Providing a security sandbox for untrusted programs.
- Compiling Snacc to WebAssembly or running the compiler in the browser.
- Providing an interactive terminal or TTY in the first version.
- Streaming output while the process is still running.
- Supporting multiple source modules before the compiler supports them.
- Exposing arbitrary filesystem paths, compiler flags, linker flags, or
  environment variables to the browser.
- Displaying native object bytes, disassembly, or generated LLVM IR initially.
- Replacing `cargo snacc` as the application workflow described by RFC 003.
- Running Cargo dependency builds for each snippet.
- Automatically executing a snippet merely because it was selected.

## Relationship to other RFCs

- [RFC 002](archive/002-windows-llvm-toolchain.md) provides the permanent Inkwell and
  LLVM runtime integration.
- [RFC 003](archive/003-cargo-hosted-applications.md) provides Cargo-hosted applications
  and Rust crate integration.
- [RFC 005](005-remove-runtime-rs.md) defines the generated host and
  `snacc-runtime` linking contract.
- [RFC 006](006-workspace-organization.md) defines the application and
  implementation-crate locations and the rule for extracting shared native
  driver code.

The workbench depends only on `snacc-driver`, which builds one source string
into one temporary executable. It does not know about LLVM objects, generated
Rust hosts, or final linker invocation. RFC 003 is not required for initial
single-snippet execution. A future project-mode workbench may delegate
Cargo-aware builds to RFC 003, but that is a separate change.

## Architecture

The implementation has four concrete parts:

~~~text
snacc compiler library
  Pure source-string frontend and LLVM lowering

native driver
  Object production, runtime selection, and executable linking

workbench server
  HTTP validation, run lifecycle, process I/O, and response shaping

browser page
  Editor, snippet selection, stdin, results, and timings
~~~

Dependencies move only downward. The compiler library does not depend on the
native driver, HTTP types, JSON models, snippet catalog, or browser assets.

The workbench is the `apps/snacc-workbench` binary package defined by RFC 006.
It depends on `snacc-driver`; HTTP and server dependencies remain outside the
compiler and driver crates.

RFC 006 initially keeps native executable construction as a concrete module in
`apps/snacc` while that CLI is its only consumer. The change that introduces
the workbench promotes that module to `crates/snacc-driver`, then makes both
applications call it. The promotion must not introduce a backend trait or a
second compiler pipeline.

## Compiler API contract

The workbench passes the exact source string through `snacc-driver`, which calls
the compiler library in process. It must not:

- Start `snacc.exe` as a child process.
- Pass source through command-line arguments.
- Create a temporary `.nrs` file merely to satisfy the CLI.
- Parse rendered diagnostics from stderr.
- Reimplement lexing, parsing, checking, or lowering.

The initial compiler boundary is the existing string-based operation:

~~~rust
pub fn emit_object(source: &str) -> Result<Vec<u8>, Diagnostics>
~~~

Diagnostics cross the server boundary as data. Each diagnostic includes:

- Compiler phase.
- Message.
- Optional byte span.
- Derived one-based start and end line and column when a span exists.

The source string used for line/column calculation must be the exact string
passed to the compiler. A compiler failure stops the pipeline; no executable is
linked or run.

Compiler calls must not depend on mutable process-global source state. The
initial server serializes compiler calls and does not promise concurrent
compilation.

## Native-driver contract

The shared native driver accepts a source string, a private build directory,
and fixed build options. It returns either structured diagnostics or the exact
path of a newly built executable.

For the workbench it must:

1. Compile the source string through Snacc.
2. Write only native intermediate artifacts to the private build directory.
3. Use the version-matched Rust runtime containing `snacc_print_f64`,
   `snacc_print_i64`, `snacc_print_bool`, `snacc_print_nil`, and the
   `snacc_main` entry call.
4. Link an executable for the host target.
5. Reject unsupported targets and profiles before linking.
6. Never reuse an executable from a different source or compiler version.

The workbench uses a fixed development profile. Browser requests cannot select
the LLVM target, CPU features, optimization flags, linker, runtime path, output
path, or extra arguments.

## Local server

The executable is named `snacc-workbench`. The documented development command
is equivalent to:

~~~text
cargo run -p snacc-workbench
~~~

By default the server binds `127.0.0.1` on an operating-system-selected free
port and prints the complete local URL. The first version has no port or bind
address option.

The browser page is embedded with `include_str!` at build time so the binary
works outside the repository. Frontend changes take effect after rebuilding or
restarting the workbench. The first version has no live-file override or
browser-selectable asset path.

No external JavaScript, font, stylesheet, analytics, or CDN resource is loaded.
The workbench remains usable offline and its page cannot silently change
without rebuilding or editing the selected local file.

## HTTP API

The first version exposes exactly these routes:

| Method | Route | Purpose |
| --- | --- | --- |
| `GET` | `/` | Embedded workbench page |
| `GET` | `/api/snippets` | Embedded snippet catalog |
| `POST` | `/api/run` | Compile and run one source string |

Unknown paths return 404. Known routes reject unsupported methods with 405 and
an `Allow` header. API responses use `application/json; charset=utf-8`. The page
uses `text/html; charset=utf-8` and `Cache-Control: no-store` during local
development.

### Run request

~~~json
{
  "source": "print(1 + 2)\n",
  "stdin": ""
}
~~~

Both fields are strings. `source` is required and must contain at least one
non-whitespace character. `stdin` defaults to the empty string when omitted.
Unknown fields are rejected so client and server version drift is visible.

The initial limits are:

| Resource | Limit |
| --- | ---: |
| JSON request body | 320 KiB |
| UTF-8 source | 256 KiB |
| UTF-8 stdin | 64 KiB |
| Active run requests | 1 |
| Program execution time | 3 seconds |
| Captured stdout | 1 MiB |
| Captured stderr | 1 MiB |

Limits are server constants in the first version, not browser-controlled
options. A request exceeding a size limit fails before compilation. A second
run while one is active receives 429 rather than creating an unbounded queue.
The initial compiler API is an in-process, non-cancellable call, so this RFC
does not pretend to provide a hard compilation timeout. The source-size limit
bounds compiler input, and the single active-run limit keeps admission
deterministic. Cancellable compilation requires a separate compiler contract.

### Run response

A syntactically valid run request receives a structured response even when
compilation or execution fails:

~~~json
{
  "compile": {
    "status": "succeeded",
    "diagnostics": [],
    "durationMs": 18.42
  },
  "execution": {
    "status": "exited",
    "exitCode": 0,
    "stdinBytes": 0,
    "stdout": "3\n",
    "stderr": "",
    "stdoutTruncated": false,
    "stderrTruncated": false,
    "durationMs": 2.11
  },
  "totalMs": 20.53
}
~~~

`compile.status` is `succeeded` or `failed`. `execution` is `null` when
compilation fails. `execution.status` is one of `exited`, `timed-out`,
`output-limit`, or `failed-to-start`. `exitCode` is null when the process never
produced a meaningful application exit code.

`compile.durationMs` measures source-to-executable construction, including
compiler object emission and final linking. `execution.durationMs` measures
only the program process. `totalMs` covers the complete accepted request.

Malformed JSON, invalid fields, missing authentication, excessive input, and
server-busy conditions use appropriate non-200 HTTP statuses. A valid Snacc
program that exits nonzero still uses HTTP 200 because the program result is
the response, not an HTTP transport failure.

Compiler diagnostics are never copied into `execution.stderr`. Program stderr
contains only bytes read from the child process's stderr pipe.

## Process execution

Every accepted run owns a fresh temporary directory and executable. The server
starts that exact executable with:

- Its current directory set to the private temporary directory.
- Standard input, output, and error connected to pipes.
- No shell, command interpreter, or string-built command line.

The initial workbench runs trusted local snippets with the server process's
environment. The browser cannot add, remove, or override environment variables.
The workbench links no application bridge functions, so a snippet has no usable
environment, filesystem, network, subprocess, or dynamic-library operation.
Environment filtering or process isolation must be reconsidered in the same
specification that exposes any such capability or permits non-local users.

The server writes the complete stdin byte sequence, closes the child's stdin,
and drains stdout and stderr concurrently. Sequential reads are forbidden
because a child filling one pipe can deadlock while the server waits on the
other.

When either output limit is reached, the server kills the child, waits for it,
retains bytes only up to each declared bound, sets the corresponding truncation
flag, and reports `output-limit`. The UI visibly marks the truncated stream.

On timeout, the server kills the immediate child, waits for termination,
captures already-produced bounded output, and reports `timed-out`. Snacc cannot
currently spawn subprocesses, so the generated program cannot create a child
process tree. Reliable process-tree ownership must be added in the same RFC
that gives Snacc subprocess or equivalent FFI capabilities.

Temporary artifacts are removed after the response is complete. The first
version has no artifact-retention option.

## Security model

The workbench turns an HTTP request into native-code execution. Loopback alone
is necessary but not sufficient because a malicious webpage may attempt to
reach local services.

At startup the server creates a cryptographically random session token. The
served page receives it through server-side placeholder substitution and sends
it in a custom header on every API request. The server also:

- Accepts only loopback connections.
- Validates `Host` against the bound loopback host and actual port.
- Rejects absent or foreign `Origin` values on state-changing requests.
- Sends no permissive CORS headers.
- Requires `Content-Type: application/json` for `/api/run`.
- Requires the session token with a constant-time comparison.
- Applies request-size, concurrency, execution-time, and output limits.
- Never interpolates browser strings into a shell command.
- Sends a fixed policy containing `default-src 'none'`, `connect-src 'self'`,
  `style-src 'unsafe-inline'`, and `script-src 'unsafe-inline'`. The page
  contains only build-time embedded code, inserts response data with
  `textContent`, and loads no external resources. The first version has no
  nonce or inline bootstrap generation.

The startup log and page must state that the tool executes native programs and
is intended only for trusted local snippets. These controls reduce accidental
and cross-origin invocation; they do not sandbox filesystem, process, network,
or FFI capabilities that Snacc may gain later. Before such capabilities become
available to snippets, a separate sandboxing RFC is required for any untrusted
use.

## Browser experience

The initial page has five visible areas:

1. Snippet selector.
2. Snacc source editor.
3. Standard-input editor.
4. Run status and compiler diagnostics.
5. Standard-output and standard-error panels with exit information.

The page uses one editable source string because that matches the compiler's
current contract. The editor starts with a minimal `print` example. Selecting a
catalog snippet replaces the source and stdin but does not execute it. The user
presses **Run** or `Ctrl+Enter` to authorize native execution.

While a request is active, the Run button is disabled and the status reads
which phase is active when known. Editing source or stdin after a completed run
marks the displayed result as stale without deleting it.

The output presentation follows process terminology exactly:

- **stdin** shows the text supplied before execution.
- **stdout** shows values produced by `print`, preserving whitespace.
- **stderr** shows only program stderr, preserving whitespace.
- **Compiler diagnostics** show source errors with phase and source range.
- **Process result** shows exit code, timeout, truncation, and duration.

Empty streams remain visible and are labeled `(empty)` by presentation only;
the underlying returned string remains empty. Output is inserted with
`textContent`, never `innerHTML`.

The page must be keyboard-usable, use explicit labels, expose status changes to
assistive technology, remain readable at narrow widths, and use a plain
textarea that does not trap page scrolling. The first version has no syntax
highlighting.

## Snippet catalog

The workbench does not maintain a second program or expected-output corpus.
`apps/snacc-workbench/snippets.json` contains display metadata and points to
self-contained programs under `examples/`:

~~~json
[
  {
    "id": "core",
    "name": "Core language",
    "description": "Runs the core executable language examples.",
    "case": "arithmetic"
  }
]
~~~

`case` is a basename, not a browser-supplied path. At build time every entry
must resolve to an existing `examples/<case>.nrs` and its existing `.stdout`
sidecar; duplicate IDs or missing referenced files fail the build.
The source and display metadata are embedded in the binary and returned by
`GET /api/snippets`. The browser never reads repository paths.

The existing conformance runner remains the owner of expected program output.
The workbench catalog adds no `source`, `features`, `expected`, exit-code, or
stream fields. It discovers executable examples with `.stdout` sidecars and
verifies them independently. Adding a workbench snippet therefore requires
adding or reusing a passing executable example first; native object and
executable hashes are never baseline artifacts.

## Performance and concurrency

The workbench binary, including its linked compiler library, is built before the
server starts and is never rebuilt per run. One request may invoke object
generation and linking, but it must not run `cargo build` for the Snacc compiler
itself.

The response records source-to-executable compilation duration, program
execution duration, and total request duration.

The initial server allows one active compile-and-run request. This makes CPU,
memory, LLVM state, and process cleanup straightforward. Raising the limit and
defining concurrent compiler behavior are outside this RFC.

## Failure ownership

| Failure | Reported as |
| --- | --- |
| Invalid HTTP or JSON | Workbench request error |
| Authentication or origin rejection | Workbench security error |
| Unsupported Snacc syntax | Compiler diagnostic |
| Type error | Compiler diagnostic |
| LLVM or object-emission failure | Compiler backend diagnostic |
| Native linker failure | Build diagnostic |
| Executable cannot start | Execution `failed-to-start` |
| Program writes stderr | Execution stderr, not compiler failure |
| Program exits nonzero | Execution result with its exit code |
| Program exceeds deadline | Execution `timed-out` |
| Output exceeds a cap | Execution `output-limit` with truncation flag |

The server must not convert internal compiler failures into source diagnostics.
Unexpected server failures receive a request ID in both the log and response,
without exposing stack traces, environment variables, or absolute toolchain
paths to the browser by default.

## Testing

### Compiler and native-driver tests

- Compile a source string without creating a source file.
- Compile and run `print(1 + 2)` and require stdout `3\n`.
- Compile invalid source and require structured diagnostics with spans.
- Prove the runtime's print functions write to stdout rather than stderr.
- Prove each build uses an isolated temporary directory.
- Prove a stale executable is not reused after a source change.

### Catalog tests

- Every catalog entry resolves to an existing self-contained `.nrs` example and
  `.stdout` sidecar.
- Catalog IDs are unique and non-empty.
- `GET /api/snippets` returns the embedded source and display metadata without
  exposing repository paths.
- The existing conformance suite, rather than a second catalog runner, owns
  expected-output execution for referenced cases.

### HTTP tests

- `GET /` returns the embedded page.
- `GET /api/snippets` returns only validated catalog data.
- `POST /api/run` accepts source and stdin strings.
- Invalid methods, content type, JSON, unknown fields, and sizes are rejected.
- Missing token, foreign origin, and invalid host are rejected.
- Compiler failure returns no execution object.
- Successful execution keeps stdout and stderr separate.
- Busy-server behavior is bounded and deterministic.

### Process tests

- Supplied stdin reaches a helper child exactly and then observes EOF.
- Simultaneous stdout and stderr larger than pipe buffers do not deadlock.
- Timeout kills and waits for the child.
- Either output cap kills and waits for the child, bounds memory, and sets the
  corresponding truncation flag.
- Paths containing spaces compile and execute successfully.

### Browser smoke tests

- A snippet loads without running automatically.
- Run and `Ctrl+Enter` produce the same request.
- Compiler diagnostics render separately from program stderr.
- Empty and truncated streams are visibly distinguished.
- Editing after a run marks the result stale.
- Narrow layout and keyboard navigation keep every control reachable.

## Implementation phases

### Phase 1: reusable execution boundary

- Promote native executable construction from the concrete `apps/snacc` module
  defined by RFC 006 into `crates/snacc-driver` because the workbench becomes
  its second consumer.
- Preserve direct source-string compilation.
- Add child stdin/stdout/stderr capture, timeout, output caps, and
  temporary-directory cleanup.
- Prove `print` output end to end without HTTP.

### Phase 2: server and API

- Add the separate workbench crate and embedded page.
- Bind to an ephemeral loopback port.
- Implement the session token, host/origin validation, request limits, and the
  three HTTP routes.
- Add structured run responses and server tests.

### Phase 3: browser interface

- Add source, stdin, stdout, stderr, diagnostics, and process-result areas.
- Add explicit Run and `Ctrl+Enter` behavior.
- Add stale-result, truncation, timeout, and busy states.
- Complete accessibility and responsive-layout smoke tests.

### Phase 4: executable snippet catalog

- Split the current combined run corpus into small, independently runnable
  examples with `.stdout` sidecars for arithmetic, printing, functions,
  conditionals, and loops.
- Update the conformance runner to discover and verify those examples.
- Add display metadata pointing to selected executable examples and resolve and
  embed them at build time.
- Return embedded metadata and source through `/api/snippets`.
- Keep expected output owned by the existing conformance suite.

## Acceptance criteria

- `snacc-workbench` starts on an operating-system-selected loopback port and
  serves its embedded page without network access.
- The server calls Snacc's compiler library with the exact browser-provided
  source string and does not invoke the Snacc CLI or create a source file.
- A `print(1 + 2)` snippet produces stdout `3\n`, empty stderr, and exit code 0.
- Compiler diagnostics and program stderr are represented and displayed
  separately.
- Supplied stdin is written completely and closed, even before Snacc exposes an
  input operation.
- Native execution has fixed execution-time, output, request-size, and
  concurrency limits.
- Timeout and output overflow kill and wait for the immediate child; temporary
  artifacts are cleaned up.
- API execution requires the startup session token and valid loopback
  host/origin checks.
- The browser cannot select paths, tools, targets, linker arguments, or process
  environment variables.
- Selecting a snippet never runs native code until the user presses Run or
  `Ctrl+Enter`.
- Every catalog entry references an existing self-contained executable
  conformance example and defines no duplicate source or expected output.
- Frontend and compiler-only tests do not rebuild the Snacc compiler for each
  case.
- `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, and the
  relevant compiler, server, process, catalog, and browser tests pass.

## Future considerations

- Interactive stdin and streaming stdout/stderr over a bidirectional channel.
- Multiple in-memory modules after the compiler defines their source-map and
  entrypoint contracts.
- Optional normalized LLVM IR or object inspection for compiler developers.
- Cargo project mode using RFC 003, with explicit dependency and build limits.
- A real OS sandbox before accepting untrusted source or exposing filesystem,
  network, subprocess, dynamic-library, or FFI features.
- Reliable process-tree termination in the same specification that gives Snacc
  subprocess or equivalent FFI capabilities.
- A hosted playground using a separately designed remote isolation service;
  the loopback server must never be repurposed for that deployment.
