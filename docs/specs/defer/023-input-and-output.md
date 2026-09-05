# Specification 023: Input and Output

Status: Proposed (design proposal, not implementation-ready)

Document kind: Feature specification (Rust-style RFC)

## 1. Proposal state

This RFC proposes Snacc's first input and output facility: a recoverable error
type, standard streams, files, and network sockets, all provided by the
runtime rather than by user Rust bridges.

Its central claim is section 4: **an I/O operation is a runtime call, and the
runtime alone decides whether it blocks a thread or suspends a task.** Snacc
source is identical either way. That is what makes
[Specification 022](022-concurrency-and-parallelism.md)'s staged scheduler
choice work, and it is why this RFC exists before that one is finished.

This RFC contains open questions, gaps, and accepted risks, recorded in
section 15. It is not implementation-ready until section 15 is closed.
`LANGUAGE.md` remains authoritative until this specification is accepted and
implemented.

## 2. Dependencies

This RFC cannot be implemented before:

- [RFC 016](archive/016-box-indirection-and-recursive-data.md) for move-only ownership
  and deterministic destruction. A file handle is the archetypal owned
  resource; without 016 there is no way to close one exactly once.
- [RFC 017](../archive/017-utf8-strings-and-views.md) for `String`, `Byte`, and
  `View<Byte>`.
- [Specification 018](archive/018-inline-sum-types.md) for `T | Error`, which is how
  every fallible operation reports failure.
- [Specification 024](../archive/024-error-handling.md) for the predeclared `Error`
  struct, stable categories, and `return_on_error` propagation.
- [Specification 019](019-collections-and-iteration.md) for `List<Byte>` read
  buffers and for `View<T>`'s physical bridge expansion.
- [Specification 020](archive/020-literal-cleanup-and-numeric-radices.md) for the
  `Float64` and `Byte` names used throughout.
- [Specification 026](archive/026-return-statements.md) for the explicit early-return
  path used to propagate `Error` values without failure flags.

It has a two-way relationship with
[Specification 022](022-concurrency-and-parallelism.md): 022 needs this RFC's
section 11 to justify a coroutine scheduler, and this RFC needs 022's task
model for its server example. Neither blocks the other's first phase.

## 3. Motivation

Snacc can compute and it can `print`. It cannot read a file, write a file, read
a line, or open a socket. A language aiming to be "a better C" that cannot do
what `stdio.h` does is not yet a systems language, and every program that needs
one byte of input today has to be a Rust host with hand-written glue.

There is a second, narrower motivation. Specification 022 section 4.2 found
that Snacc has no operation that can suspend a task, so a coroutine scheduler
buys nothing over a thread pool and costs several hazards. That finding is a
statement about missing I/O, not about concurrency. This RFC removes it.

## 4. The central principle: I/O is a runtime call, never a colored function

Every operation in this RFC lowers to a call into `snacc-runtime`. None of them
is a user `extern rust` bridge, and none of them is visible to the backend as
anything but a call.

The runtime is built against whichever scheduler the host selected
(Specification 022 section 4.4). It therefore implements one operation two
ways:

| Operation | Thread-based runtime | Coroutine runtime |
| --- | --- | --- |
| `stream.read(...)` | `std::io::Read`, blocks the thread | `may::net`, suspends the task |
| `listener.accept()` | `std::net`, blocks the thread | `may::net`, suspends the task |

**The Snacc source is byte-identical in both cases.** There is no `async`
keyword, no second spelling of `read`, no function coloring, and no rule about
where an I/O call may appear. A function that reads a socket is an ordinary
function, callable from a task or from `snacc_main`.

This is only sound because the operations belong to the runtime. Specification
022 section 8 warns that a *user* bridge which blocks a worker thread is a
hazard, and that stays true. The runtime's own I/O is not a hazard, because the
runtime knows which scheduler it was compiled against.

Three consequences follow, and they are the reason this RFC is worth its size:

1. Specification 022's staged crate swap (`rayon` now, `may` when I/O lands)
   becomes a runtime rebuild with no language change and no program change.
2. Snacc never acquires the two-worlds problem that `async` imposes on Rust.
   Goal 3 -- keep the surface small -- survives contact with I/O.
3. A Snacc program is portable across schedulers by construction, so
   Specification 022 acceptance criterion 4 extends to I/O for free.

### 4.1 The exception: files genuinely block

Sockets suspend cleanly under a coroutine scheduler. Ordinary file I/O does
not: the operating system offers no general non-blocking file interface that
`may` wraps, so a file read blocks its worker thread under every scheduler.

This is worth stating plainly because it bounds the argument in section 3.
**Files do not justify a coroutine scheduler; sockets do.** A program that only
reads files gains nothing from `may` and pays every hazard in Specification 022
section 14. The runtime may offload file operations to a small blocking thread
pool to keep workers free -- see section 15.20.

## 5. The error model

I/O fails routinely for reasons the program must be able to distinguish.
Specification 024 owns the language-wide error contract; this section applies
that contract to I/O and fixes the standard category strings produced here.

### 5.1 Options

| Option | Cost | Verdict |
| --- | --- | --- |
| `T \| Nil` | nothing new | insufficient -- loses the reason |
| `T \| Error` with the predeclared `Error` struct | one predeclared type | **required by Specification 024** |
| `Result<T, E>` | user-facing generics and a second convention | rejected by Specification 024 |
| error out-parameter, C style | nothing new | silently ignorable |
| abort on failure | nothing new | makes I/O unusable |

`T | Nil` is already the pending convention for a single-reason failure --
RFC 017's `String.from_utf8` returns `String | Nil` because invalid UTF-8 is
the only way it can fail. That convention is correct and stays. It does not
extend to I/O, where "file not found" and "permission denied" demand different
handling.

`Result<T, E>` would duplicate the direct inline-sum convention and is rejected
by Specification 024 even if general generics later land.

The error out-parameter is C's answer and is rejected for the reason C's answer
is bad: nothing forces the caller to look at it.

### 5.2 The predeclared `Error` struct

~~~snacc
type Error is struct
    category: String,
    header: String,
    message: String,
end
~~~

`Error` is predeclared by the compiler and constructed by the runtime as an
ordinary immutable struct. `category` is the stable programmatic identifier;
`header` and `message` are human-readable and may vary by platform. I/O
categories use the `io.` prefix. This RFC requires at least
`io.not_found`, `io.permission_denied`, `io.already_exists`,
`io.invalid_input`, `io.invalid_data`, `io.unexpected_end`,
`io.interrupted`, `io.timed_out`, `io.would_block`,
`io.connection_refused`, `io.connection_reset`, `io.broken_pipe`,
`io.address_in_use`, and `io.other`. Additional categories do not change the
`Error` type and are not a language compatibility break.

Every fallible operation returns an inline sum with `Error` as one member,
which Specification 018 already permits everywhere a type is written:

~~~snacc
let opened: File | Error = File.open("input.txt")

if opened is File(file) then
    print(file.size())
elseif opened is Error(error) then
    print(error.message)
end
~~~

An operation that produces nothing on success returns `Nil | Error`, which
Specification 018 also already permits, since `Nil` is legal alongside a
non-`Nil` member:

~~~snacc
let flushed: Nil | Error = output.flush()
~~~

The rule for the whole language becomes one sentence: **`T | Nil` when there is
exactly one way to fail, `T | Error` when the reason matters.**

### 5.3 Handling and propagation

Specification 018 requires a decomposition of `T | Error` either to cover
both direct members or supply `else`. Specification 024 additionally provides
`return_on_error` for concise propagation. A value-producing fallible call
cannot appear as a bare no-result statement, although a program may explicitly
store, pass, or discard its result under ordinary value rules.

### 5.4 Category stability

Standard category strings are API contracts. An implementation shall not use a
platform's numeric error value or localized text as `category`. Unknown
platform failures use `io.other`; their platform description and number may be
included in `message`.

## 6. Where I/O lives

### 6.1 Option A -- built-in types provided by the runtime

`File`, `TcpStream`, and the standard streams are compiler-known types whose
methods lower to runtime imports.

Costs a handful of predeclared types. Buys portable I/O with no Rust host, a
single implementation the runtime can retarget per scheduler (section 4), and
access to compiler-private types like `List<Byte>` that no user bridge may
touch.

### 6.2 Option B -- no language I/O; an opaque bridge handle instead

Add one thing to the Rust bridge -- an opaque, move-only `Handle<T>` that
carries a foreign resource across the boundary -- and let the host implement
I/O, and everything else, in Rust.

This is genuinely attractive. It is the smallest possible language change, it
unlocks every Rust library rather than just I/O, and RFC 016 section 15 already
lists opaque bridge handles as deferred work.

It fails on three counts. A bridge takes only scalars and immutable views, so a
host `read` still cannot fill a Snacc buffer. Section 4's scheduler neutrality
is lost, because a user bridge cannot know which scheduler the runtime was
built against -- it becomes exactly the blocking hazard Specification 022
section 8 warns about. And a language whose only way to read a file is "write
some Rust" is not a better C.

### 6.3 Recommendation: Option A now, Option B as its own specification

Build the small runtime-provided core here. Opaque handles remain independently
valuable and belong in their own RFC alongside bridge export (Specification 022
section 16); they are not a prerequisite for reading a file.

## 7. Scope and staging

| Tier | Contents | Stage |
| --- | --- | --- |
| 0 | `Error`, standard streams, line input | phase A |
| 1 | files: open, create, read, write, flush, size | phase B |
| 2 | TCP sockets: listen, accept, connect, read, write | phase C |
| 3 | filesystem operations, process, environment, time, randomness | later |

Phase C is the one that changes Specification 022's recommendation. Phases A
and B are what make Snacc usable on its own.

Not in this RFC at any tier: UDP, TLS, async DNS beyond name resolution,
memory mapping, file locking, permissions, symbolic links, directory
traversal, non-blocking modes exposed to the program, timeouts, `select`, or
formatted input parsing.

## 8. Standard streams

Snacc has no globals: a function reaches only its parameters and its own
locals, so a top-level `stdout` binding would be unreachable from inside a
function. The standard streams are therefore built-in call heads, valid
anywhere:

~~~snacc
stdin()
stdout()
stderr()
~~~

Each produces a value of a predeclared type -- `Input` for `stdin`, `Output`
for the other two. Unlike every other resource in this RFC, these are **copy
types that own nothing**: destroying one closes nothing, and two copies name
the same process-wide stream. They are the one shared resource in the language,
and the runtime synchronizes them internally.

~~~snacc
method Input.read_line(): String | Error
method Output.write(text: String): Nil | Error
method Output.write_bytes(bytes: View<Byte>): Nil | Error
method Output.flush(): Nil | Error
~~~

`read_line` returns the line without its terminator, and returns category
`io.unexpected_end` at end of input. Invalid UTF-8 on input is
`io.invalid_data`; a program that must accept arbitrary bytes reads them with
`read_bytes` (section 9) and converts explicitly with RFC 017's
`String.from_utf8`.

~~~snacc
let line: String | Error = stdin().read_line()

if line is String(text) then
    print(text)
elseif line is Error(error) then
    print(error.message)
end
~~~

### 8.1 `print` and buffering

`print` stays exactly as it is. It remains the only formatting facility in the
language, and the stream API writes only text and bytes, so the two do not
overlap and goal 2 is not violated.

`print` is respecified as writing through the same `Output` object `stdout()`
returns, so there is one buffer and one lock. Without that, a program mixing
`print` and `stdout().write` gets interleaving that depends on buffer flush
timing.

Standard output is line-buffered when it names a terminal and block-buffered
otherwise, flushed when the process exits normally. Standard error is
unbuffered. One `print` or one `write` emits its bytes atomically with respect
to other tasks, which closes Specification 022 sections 14.17 and 14.18: line
atomicity becomes a property the runtime implements deliberately rather than
one it inherits from `println!`.

## 9. Files

~~~snacc
static File.open(path: String): File | Error
static File.create(path: String): File | Error
static File.append(path: String): File | Error

method File.read(into: Ref<List<Byte>>, max: Int64): Int64 | Error
method File.write(bytes: View<Byte>): Int64 | Error
method File.flush(): Nil | Error
method File.size(): Int64 | Error
~~~

`File` is move-only. It owns an operating-system handle, and RFC 016's
deterministic destruction closes it exactly once at the end of its owner's
scope.

`read` appends at most `max` bytes to the end of `into` and returns how many it
appended; zero means end of file. Appending rather than filling avoids needing
a mutable borrowed view, which Specification 019 section 9 explicitly defers,
and it lets a loop reuse one buffer's capacity instead of allocating per read.

~~~snacc
fun read_all(path: String, into: Ref<List<Byte>>): Nil | Error do
    let mut file: File = return_on_error File.open(path)
    let mut done: Bool = false

    while done == false do
        let count: Int64 = return_on_error file.read(into, 65536)
        if count == 0 then
            done = true
        end
    end

    return nil
end
~~~

Specification 024's `return_on_error` uses Specification 026's explicit return
path, removing failure flags and nested result threading while preserving the
unchanged `Error` value.

Two conveniences cover the common cases and are worth their weight:

~~~snacc
File.read_bytes(path: String): List<Byte> | Error
File.read_text(path: String): String | Error
File.write_text(path: String, text: String): Nil | Error
~~~

### 9.1 Closing

There is no `close` method in version 1. Closing happens at destruction.

An explicit close would have to consume its receiver, and RFC 016 section 6.4
defers moves out of a receiver. `flush` exists precisely so a program can
observe a write error before destruction discards it, which is the part that
risks data loss. Adding a consuming `close` is deferred work, not a gap.

## 10. Sockets

~~~snacc
static TcpListener.bind(address: String): TcpListener | Error
static TcpStream.connect(address: String): TcpStream | Error

method TcpListener.accept(): TcpStream | Error

method TcpStream.read(into: Ref<List<Byte>>, max: Int64): Int64 | Error
method TcpStream.write(bytes: View<Byte>): Int64 | Error
method TcpStream.flush(): Nil | Error
~~~

Both types are move-only and close on destruction. `address` is
`"host:port"`; name resolution happens inside `bind` and `connect`.

`accept` and `read` are the operations that suspend a task under a coroutine
scheduler and block a thread under a thread-based one. They are the reason
Specification 022 section 4.4 has a second stage.

## 11. Concurrency interaction

This section discharges Specification 022's dependency on I/O and closes three
of its open questions.

A server, written with Specification 022's `parallel` and `spawn`:

~~~snacc
fun handle(client: TcpStream) do
    let mut request: List<Byte> = []
    let read: Int64 | Error = client.read(request, 4096)

    if read is Int64(count) then
        let written: Int64 | Error = client.write(request.view())
        if written is Error(error) then
            print(error.message)
        elseif written is Int64(sent) then
            print(sent)
        end
    elseif read is Error(error) then
        print(error.message)
    end
end

fun serve(listener: Ref<TcpListener>) do
    parallel do
        while true do
            let accepted: TcpStream | Error = listener.accept()
            if accepted is TcpStream(client) then
                spawn handle(client)
            elseif accepted is Error(error) then
                print(error.message)
                break
            end
        end
    end
end
~~~

Four things in that example are worth naming:

1. **`spawn handle(client)` moves the stream into the task.** `TcpStream` is
   move-only, so the accepting task cannot touch it afterwards -- the move
   checker already says so. Ownership transfer across a task boundary needs no
   new rule, which is Specification 022 section 14.29 realized.
2. **No `Ref<T>` crosses the spawn**, so Specification 022 rules 5 through 7
   are satisfied trivially, including rule 7's ban on a borrowing spawn inside
   a loop.
3. **The scope holds every connection task.** That is correct for a server and
   is exactly the unbounded task creation Specification 022 section 14.6
   flags. A real server needs admission control, and neither specification
   provides it yet.
4. **`client.read` suspends the task** under a coroutine scheduler, so one
   worker serves many connections. Under `rayon` the same source occupies a
   pool thread per connection. Same program, different throughput, no
   diagnostic difference.
5. **The `break` is legal.** Specification 022 section 6.2 forbids `break` as a
   direct element of a `parallel` body, but this one targets the `while` nested
   inside that body, which is ordinary. Without that allowance the loop would
   need a flag variable.

Two further interactions:

- A `File` or `TcpStream` cannot be shared by two tasks, because it is
  move-only and there is no shared ownership. Concurrent access to one stream
  is not expressible, which is the correct default.
- The standard streams are the sole exception (section 8) and are internally
  synchronized.

## 12. Runtime ABI and backend

### 12.1 Representation

`File`, `TcpListener`, and `TcpStream` lower to one pointer-sized opaque value
holding a runtime-owned handle. `Input` and `Output` lower to a small integer
selecting the stream. Every representation is compiler-private and none crosses
a user Rust bridge, for the reasons Specification 019 section 16.1 gives for
collections.

### 12.2 Imports

Each operation is one runtime import taking the handle and the operation's
scalar or view arguments, following the physical expansion Specification 019
section 16.3 already defines for `View<T>`. A fallible result uses the
caller-provided result-slot convention: the caller reserves compiler-sized and
compiler-aligned storage, passes its address as the final hidden argument, and
the runtime writes the inline-sum tag plus the active payload into that slot.
The caller owns the initialized payload and its destruction after the call.
There is no returned inline-sum value and no payload copy across the import
boundary. For `Nil | Error`, the `Nil` tag leaves no payload initialized; for
`String | Error`, exactly one `String` or `Error` payload is initialized.

Destruction of an owned handle calls its close import from RFC 016's cleanup
plan; the compiler emits no other lifetime logic.

### 12.3 Backend work

Almost none. These are ordinary calls to declared imports, plus one entry in
the cleanup plan per owned handle type. The type checker gains predeclared
types and their method signatures; the layout algorithm, calling convention,
and every existing lowering are unchanged.

The real implementation cost is in the runtime, which must present one API over
two schedulers, and in RFC 016, whose destruction machinery this RFC assumes.

### 12.4 ABI version

Adding imports and predeclared owned types advances the compiler/runtime ABI
version when their physical signatures or representations change. The numeric
successor is assigned by the shared ABI policy; source-only checker or syntax
changes do not bump the version.

## 13. Implementation plan

### Phase A: error model and standard streams

Implement Specification 024's predeclared `Error` struct. Predeclare `Input`
and `Output`, the
three call heads, and their methods. Respecify `print` through `Output`.
Runtime implements the streams with buffering and line atomicity. This phase
alone gives Snacc line-oriented input, which is what most small programs want.

### Phase B: files

`File` and its methods, the three conveniences, and destruction through
RFC 016's cleanup plan. Requires RFC 016 and Specification 019 to have landed.

### Phase C: sockets

`TcpListener` and `TcpStream`. In the same change, implement the runtime's
socket layer twice -- once on `std::net` for the thread-based scheduler and
once on `may::net` -- and promote `may` to Specification 022's default, which
is the whole point of section 4.

### Phase D: conformance

Cases covering each standard I/O error category, end of input, a read loop over a large file,
a round-trip socket test, and the server in section 11 under both schedulers.

## 14. Acceptance criteria

1. A program reads a line from standard input and writes it back, with no Rust
   host code.
2. A program reads a file that does not exist and distinguishes
   `io.not_found` from `io.permission_denied` through `error.category`.
3. A fallible value-producing call used as a bare no-result statement does not
   compile.
4. A file is closed exactly once, at the end of its owner's scope, verified by
   a runtime handle count.
5. A `File` used after being moved into a function is a compile error.
6. The section 11 server passes identical functional tests under the
   `rayon`, `may`, and `std::thread::scope` runtimes, with no source change.
7. Under `may`, one worker serves more concurrent connections than there are
   worker threads -- the observable proof that section 4 is real.
8. `print` and `stdout().write` interleave in program order.

## 15. Open questions, risks, and concerns

Everything known to be unresolved or uncomfortable about this proposal is
recorded here. Items marked **gap** are places where the specification above is
incomplete and must be written before implementation; items marked **risk** are
decided but dangerous; the rest are choices awaiting a decision.

### The error model

**15.1 Standard category coverage. (risk)** Specification 024 makes categories
extensible strings, so adding a category is not a type-system break. The list
in section 5.2 must still be mapped consistently across operating systems and
tested without depending on localized messages.

**15.2 One `Error` shape does not advertise per-operation categories. (risk)**
A signature says that an operation may fail but not which category strings it
may produce. Each operation's API contract must list its categories. This is a
documentation limitation, not a reason to add a second error convention.

**15.3 User code can construct `Error`. (decided)** Specification 024
deliberately permits ordinary construction. User functions may return
`T | Error` and should namespace their stable category strings.

**15.4 Platform error numbers are diagnostic text only. (decided)** Runtime
mapping selects a stable `io.` category. A platform number may appear in
`message` but programs cannot access it as a portable structured field.

**15.5 Error creation allocates three strings. (risk)** `Error` owns
`category`, `header`, and `message`. This cost occurs only on a recoverable
failure path and buys one uniform, self-contained value.

**15.6 Human-readable error text is platform-dependent. (decided)** Tests
compare `category`; they do not use `header` or `message` as golden output.

**15.7 Automatic propagation is specified. (closed)** Specification 024's
`return_on_error` propagates the unchanged `Error` member through the explicit
return and cleanup path.

**15.8 Fallible results are always truthy. (decided)** Specification 021's
truthiness rule is unchanged. Programs use `is` or `return_on_error`; a bare
condition does not test success.

### Language surface this RFC assumes but the language lacks

**15.9 Associated functions are a language feature. (decided)** Calls such as
`File.open(path)`, `File.create(path)`, and `TcpListener.bind(address)` are
real type-namespaced functions with no implicit receiver. Their declaration
uses the shared form `static Type.name(...) do ... end`; for example:

~~~snacc
static File.open(path: String): File | Error do
    ...
end
~~~

User-defined types use the same mechanism. RFC 017 records the corresponding
`String` constructors. Phase A must add the `static` declaration grammar and
resolver path while retaining `method` for receiver-bearing functions.

**15.10 Six predeclared type names, and the count grows.** `Error`, `File`,
`Input`, `Output`, `TcpListener`, and `TcpStream` are nominal types the program
never declared, occupying the top-level type namespace and colliding with user
names. `LANGUAGE.md` requires top-level type names to be unique among built-in
and user-defined names, so this works -- but it is a real cost to a language
that prizes a small surface, and tier 3 adds more.

**15.11 The examples assume affordances Specification 019 must supply.**
`request.view()` in section 11 assumes `List<T>.view()`; the read loop assumes
list capacity is reused across appends. Both are consistent with
Specification 019 but neither is quoted from it. Verify before phase B.

### Resources, ownership, and destruction

**15.12 No consuming `close`. (gap-by-choice)** Section 9.1 explains the
reasoning. A close error on a buffered writable file is discarded at
destruction unless the program calls `flush` first, and nothing makes it.

**15.13 Destruction cannot report errors at all.** This is more general than
15.12: RFC 016's cleanup runs on scope exit with nowhere to put a failure.
Every language with RAII has this wart; naming it here stops it being
rediscovered.

**15.14 A handle inside an aggregate makes the aggregate move-only.** RFC 016
section 5.3 propagates move-only status structurally, so a struct with a `File`
field is move-only and a `List<File>` closes every element when destroyed. That
is the correct behavior and a genuinely nice one, but it means adding a handle
field to an existing struct is a breaking change for every copy of that struct.

**15.15 Who destroys a move-only value moved into a task? (gap)** Section 11
moves a `TcpStream` into a spawned task by value. The value lives in the spawn
payload, so the *task* must destroy it, not the spawner, and the generated
thunk in Specification 022 section 10.2 must run cleanup for by-value
move-only arguments. Neither specification says so. Specification 022
section 14.28 gestures at destruction inside a task; this is the concrete case
and it must be written in both documents in the same words.

### Reading, writing, and buffers

**15.16 `read` returns zero at end of file.** That is C's convention, and it is
easy to get wrong in a loop. `io.unexpected_end` exists but is used only
where a partial result is definitely wrong, such as `read_line`. Consider
whether one convention should cover both cases.

**15.17 The runtime must know the private layouts of `List` and `String`.**
`read` appends into a caller's list and `read_text` returns a `String`, so the
runtime calls the same allocation and growth entry points Specification 019's
and RFC 017's lowering use. The coupling is compiler-private and acceptable,
but it means the runtime versions together with both collection and string
implementations, and it is wider than a single "list growth" hook.

**15.18 Buffering interacts with task interleaving.** Section 8.1 guarantees
per-write atomicity. It does not say what happens when two tasks write partial
lines, and one shared buffer under a coroutine scheduler is contended by
definition. Per-task buffers would break ordering; one buffer serializes
writers.

**15.19 Concurrent reads of standard input are unspecified.** Two tasks calling
`stdin().read_line()` split the stream between them in an unspecified order.
Per-call atomicity is achievable; which task gets which line is not.

### Scheduler and platform

**15.20 File I/O blocks under every scheduler. (risk)** Section 4.1. Should the
runtime offload file operations to a small blocking thread pool so workers stay
free? That adds a second pool, a queue, and a copy, and changes file-operation
latency. Measure before deciding.

**15.21 Name resolution is hidden inside `bind` and `connect`.** Under a
coroutine scheduler a blocking DNS lookup blocks a worker even though the
surrounding operation suspends correctly, and `may` provides no resolver.

**15.22 Standard streams are copy types naming a shared resource.** They are
the only such thing in the language. Specification 022 section 7.3 argues that
no `Send` reasoning is needed because nothing is shared; that argument must be
restated to exclude these explicitly. They are safe because they carry no
state, but the exception has to be written down rather than assumed.

**15.23 No timeouts, no cancellation.** A read that never completes hangs its
task forever, and Specification 022 section 6.5 declines to add cancellation.
For a server that is a denial-of-service surface, not a nicety.

### Paths

**15.24 Paths are `String`, and operating-system paths are not. (risk)**
Windows paths are UTF-16 with unpaired surrogates permitted; Unix paths are
arbitrary bytes. A `String` is valid UTF-8 by RFC 017's invariant, so some real
files are unnameable. Options: accept and document the gap, add a `Path` type
over `List<Byte>`, or accept `View<Byte>` as an alternative path form. Version
1 accepts the gap; a language that cannot open every file on the disk should
not accept it forever.

### ABI, build, and testing

**15.25 The fallible-result ABI is now selected. (resolved)** Section 12.2
uses the caller-provided result-slot convention defined normatively by
Specification 024 §9.1. The remaining implementation work is to
make the inline-sum layout algorithm spell out the slot's exact size, alignment,
tag encoding, and payload offsets, and to test that the caller destroys exactly
the active member. This convention keeps large `String` and `Error` results
out of a return-value copy while making ownership explicit.

**15.26 Testing I/O in the conformance suite.** Cases now need temporary files,
ports, and cleanup; socket tests are inherently flaky in CI; and 15.6 makes
error text unusable as golden output. The existing harness compares stdout for
a process with no filesystem side effects, and that model has to grow.

**15.27 Goal 12 revisited.** A program that performs no I/O should link no
socket layer and start no threads. Verify that I/O support is lazily
initialized and that binary size for a compute-only program does not move --
the same concern Specification 022 section 14.15 raises for the scheduler.

### Scope

**15.28 Command-line arguments and environment may matter more than sockets.**
A Snacc program cannot currently see its own arguments. For a language aiming
to be a better C, `argv` and environment access are arguably more essential
than either files or sockets, and they are cheap: scalars and strings, no
handles, no ownership, no scheduler interaction. Section 7 puts them in tier 3
on the grounds that Specification 022 needs sockets first. That ordering serves
the concurrency specification rather than the language's users, and it may be
the wrong call.
## 16. Rejected alternatives

### Every operation returns `T | Nil`

Cheapest possible, and it makes "file not found" indistinguishable from
"permission denied". A systems language that cannot tell a user which of those
happened is not usable for the programs it targets.

### `Result<T, E>` with user-facing generics

It duplicates `T | Error` and would give Snacc two error conventions. General
generics may still be useful, but they do not replace the error contract in
Specification 024.

### Errors as `Ref<Error>` out-parameters

C's model. Nothing forces the caller to inspect the error, which discards the
one guarantee section 5.3 buys.

### Abort on I/O failure

Consistent with Specification 019's treatment of bounds and allocation failure,
and wrong here: a missing file is an ordinary condition a program must handle,
not a defect in the program.

### An `async` variant of each operation

The two-worlds problem, imported deliberately. Section 4 exists to avoid it.

### No language I/O; opaque bridge handles only

Section 6.2. Rejected as the *only* mechanism, recommended as an additional one
in its own specification.

## 17. Deferred work

- a `Path` type for non-UTF-8 paths (section 15.24);
- consuming `close` (section 9.1);
- opaque Rust bridge handles, with bridge export (section 6.3);
- tier 3: filesystem operations, process spawning, environment, time,
  randomness;
- UDP, TLS, timeouts, cancellation, `select`, memory mapping, file locking;
- mutable borrowed buffers, which would let `read` fill a caller's array
  instead of appending to a list (Specification 019 section 9);
- formatted input parsing.

## 18. References

- [`LANGUAGE.md`](../../LANGUAGE.md)
- [RFC 014: Generic Programming](../archive/014-generic-programming.md)
- [RFC 016: Box Indirection and Recursive Data](archive/016-box-indirection-and-recursive-data.md)
- [RFC 017: UTF-8 Strings and Views](../archive/017-utf8-strings-and-views.md)
- [Specification 018: Inline Sum Types](archive/018-inline-sum-types.md)
- [Specification 019: Collections and Iteration](019-collections-and-iteration.md)
- [Specification 020: Literal Cleanup and Numeric Radices](archive/020-literal-cleanup-and-numeric-radices.md)
- [Specification 022: Concurrency and Parallelism](022-concurrency-and-parallelism.md)
- [Specification 024: Error Handling](../archive/024-error-handling.md)
- [Specification 025: Deferred Calls](../archive/025-defer.md)
- [Specification 026: Return Statements](archive/026-return-statements.md)
