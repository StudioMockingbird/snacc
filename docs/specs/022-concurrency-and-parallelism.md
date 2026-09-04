# Specification 022: Concurrency and Parallelism

Status: Proposed (design proposal, not implementation-ready)

Document kind: Feature specification (Rust-style RFC)

## 1. Proposal state

This RFC proposes a structured concurrency facility for Snacc. It presents four
candidate language surfaces, recommends one for the first version, and
specifies its syntax, semantics, checker rules, lowering, and runtime ABI in
enough detail to build. Section 4.3 evaluates the runtime crates that can
implement it, including `may`, `rayon`, `crossbeam`, `corosensei`, and
`stackful`.

This RFC contains open questions, unresolved gaps, and accepted risks, all
recorded in section 14. It is not implementation-ready until section 14 is
closed. `LANGUAGE.md` remains authoritative until this specification is
accepted and implemented.

## 2. Dependencies

The recommended first version (section 5) depends on nothing that is not
already implemented. It uses `Ref<T>` reference parameters as they exist today
and adds no type constructor, no generic form, and no heap allocation.

Later layers depend on:

- [RFC 016](archive/016-box-indirection-and-recursive-data.md) for move-only ownership
  and scope-exit cleanup, required before a task handle or a channel can own a
  value;
- [Specification 019](019-collections-and-iteration.md) for arrays and index
  places, required before fan-out writes into distinct elements;
- [RFC 014](014-generic-programming.md) only if a task or channel type is ever
  made user-parameterizable. The forms proposed here are closed
  compiler-provided type forms, like `Box<T>`.

[Specification 023](023-input-and-output.md) is not a dependency of version 1
but is a dependency of the recommendation in section 4.4: until Snacc has a
suspending operation, a coroutine scheduler earns nothing. Its phase C is the
trigger that promotes `may` to the default.

[Specification 026](026-return-statements.md) defines return from ordinary
callables. A return that would cross a live `parallel` scope additionally needs
the scope-exit join described in section 6.2.

## 3. Motivation

Goal 9 of `AGENTS.md` is that "concurrency and multithreading should be
straightforward". Snacc currently has no way to express either.

The language's existing restrictions make this easier, not harder. Snacc has no
function values, no closures, no nested functions, no globals, and no implicit
shared state. A function reaches only its parameters and its own locals. There
is therefore no way, today, to write two pieces of code that reach the same
mutable storage -- except through `Ref<T>`, which is call-scoped and cannot be
stored, returned, or nested.

That is the whole data-race problem, already solved by the type system for
reasons that had nothing to do with concurrency. A concurrency feature built on
top does not need a `Send` marker, a `Sync` marker, an ownership qualifier, or
an escape analysis. It needs two rules about `Ref<T>` and a scope.

### 3.1 Why stackful coroutines rather than `async`

This section argues for a *model*. Which crate implements it first is a
separate question, answered in section 4.3.

The decisive property is that stackful coroutines need no language support.

An `async`/`await` design lowers each suspendable function into a state machine
and requires futures, which are values holding suspended computations. Snacc
has no function values and no way to spell one. Adopting `tokio` would mean
first adding closures, then generic futures, then function coloring, then a
poll-based trait -- four large language features in service of one runtime.

A stackful coroutine suspends by switching stacks. The generated code is
ordinary native code with an ordinary call stack; nothing in the compiler needs
to know a suspension can happen. `may` provides that: an M:N work-stealing
scheduler over native stacks, so `spawn` costs a stack allocation rather than a
thread, and tasks migrate across worker threads for real parallelism.

`may` 0.3.51 (May 2025, ~176k downloads, single maintainer) supports x86_64 and
AArch64 on Linux, macOS, and Windows -- including x86_64 Windows, the host this
compiler currently targets. Its adoption is modest for infrastructure a
language contract depends on, which section 4 addresses directly.

## 4. Architectural principle: the scheduler is not the contract

The language surface proposed here promises concurrency, task completion, and
data-race freedom. It does not promise coroutines, work stealing, or `may`.

Everything the backend emits goes through three C ABI functions (section 9).
Behind that boundary, `snacc-runtime` may implement tasks with `may`
coroutines, with `std::thread::scope`, or by running each task inline on the
calling thread. All three satisfy the semantics in section 6.

This is the answer to `may`'s adoption risk and to its platform limits. A
target `may` does not support falls back to `std::thread::scope` with the same
observable behavior and worse task-creation cost. No Snacc program changes, no
diagnostic changes, and the ABI version does not move. Keep the boundary this
narrow and the dependency stays replaceable.

### 4.1 What the runtime actually has to provide

Section 9's ABI asks for exactly three capabilities:

1. start a task from a plain `unsafe extern "C" fn(*mut u8)` and an opaque
   payload pointer;
2. run tasks on more than one processor;
3. block until every task started in one scope has finished.

Nothing more. No futures, no polling, no I/O reactor, no timers, no
cancellation, no task-local state. Any crate is judged on those three, on what
it costs a program that never spawns, and on what hazards it forces into
section 8's bridge contract.

### 4.2 The decisive fact: Snacc has no I/O

`print` is the only side effect a Snacc program can currently produce. Snacc
code contains no operation that can suspend a task -- only a Rust bridge could,
and no bridge does today. **Every Snacc task therefore runs to completion
without ever yielding.**

That is precisely a fork-join thread-pool workload, and precisely *not* the
workload stackful coroutines exist for. `may`'s advantage -- a hundred thousand
tasks parked on sockets for the price of their stacks -- is unrealized until
Snacc has sockets, which [Specification 023](023-input-and-output.md) phase C
supplies and this RFC does not.

Meanwhile every coroutine hazard is real from day one: fixed stacks
(section 9.3), the thread-local storage rule and the blocking-worker deadlock
rule (section 8), and the guard-page concern in section 14.9.

This does not change the argument in section 3.1. Stackful coroutines remain
the only concurrency model that costs Snacc no language surface, and that is
still why the eventual target is `may` rather than `tokio`. It changes only
*which crate implements the three functions first*.

[Specification 023](023-input-and-output.md) removes this finding. Its section
4 makes every I/O operation a runtime call, so the runtime alone decides
whether a read blocks a thread or suspends a task, with no Snacc source
difference. Its phase C adds sockets, which are the operations that actually
suspend -- files, per its section 4.1, block under every scheduler and justify
nothing here. When phase C lands, `may` earns its default.

### 4.3 Candidate evaluation

Figures are from crates.io as of September 2026.

| Crate | Version / last release | Downloads (total / recent) | Shape | Verdict |
| --- | --- | --- | --- | --- |
| `rayon` | 1.12.0, Apr 2026 | 526M / 118M | work-stealing fork-join pool | **default for v1** |
| `may` | 0.3.51, May 2025 | 176k / 33k | M:N coroutine scheduler + I/O | **feature-gated; default once I/O lands** |
| `std::thread::scope` | in std since 1.63 | -- | one OS thread per task | **fallback and test oracle** |
| `generator` | 0.8.9, Jun 2026 | 70M / 13M | stack switching only | indirect (via `may`) |
| `corosensei` | 0.3.4, May 2026 | 8.2M / 1.2M | stack switching only | contingency base |
| `crossbeam` | 0.8.4, Jan 2024 | 135M / 27M | concurrency toolbox | channels only, for Option C |
| `chili` | 0.2.1, Mar 2025 | 1.3M / 140k | low-overhead fork-join | watch, for Option D |
| `stackful` | 0.1.5, Jun 2022 | 45k / 1k | sync-to-async bridge | rejected |
| async family | -- | -- | poll-based futures | rejected (section 15) |

#### `rayon` -- recommended default implementation

Fits section 6's semantics exactly: `parallel do` *is* fork-join, and a task
that never suspends is exactly what a rayon pool runs best. It removes three
hazards outright. Tasks run to completion on one thread and never migrate
mid-execution, so section 8's thread-local storage obligation disappears. Tasks
run on real OS thread stacks, so section 9.3's fixed-stack failure mode and
section 14.9's guard-page concern disappear. The pool is persistent, so a loop
spawning ten thousand tasks allocates no stacks.

It is also, by a wide margin, the best-supported option: 526M downloads, an
active team, and a release five months old.

What it costs: a task that blocks on I/O blocks a pool thread, so it cannot
serve many concurrent connections -- irrelevant today (section 4.2), decisive
later. And one implementation detail needs care: `rayon::scope` is
closure-scoped, so it cannot be held across three separate FFI calls. The
begin/spawn/end shape is implemented instead with `rayon::spawn` plus a
completion counter, and `snacc_scope_end` must call `rayon::yield_now()` while
waiting so that a nested scope waiting inside a pool thread steals work instead
of deadlocking the pool. See section 14.14.

#### `may` -- the target, once there is I/O to multiplex

Everything section 3.1 says about it stands. It is the only candidate here that
supplies both the scheduler and the non-blocking network stack, and
`may::sync::mpmc` is the ready-made implementation of Option C's channels.

The concerns are adoption and staffing, not design: 176k total downloads is
thin for something a language contract depends on, it has one maintainer, and
its last release is sixteen months old. Its stack-switching layer, `generator`,
is by the same author and is far healthier (70M downloads, released June 2026),
which suggests the foundation is alive and the scheduler layer is quiet rather
than abandoned.

Ship it feature-gated in phase 3 so the seam in section 4 is exercised rather
than asserted, and promote it to the default in the same change that gives
Snacc its first suspending operation.

#### `std::thread::scope` -- the fallback and the oracle

Zero dependencies, in std since 1.63, and about forty lines. Its cost is one OS
thread per task, which caps useful task counts in the thousands.

Keep it not only as the fallback for a target neither other crate supports, but
as the **test oracle**: it is the simplest possible implementation of
section 6's semantics, so any behavior difference between it and the other two
is a bug in the other two. Acceptance criterion 4 exists for this.

#### `corosensei` -- the contingency

The same primitive `may` is built on -- stackful coroutine stack switching --
but better maintained (Amanieu, 8.2M downloads, released May 2026), with guard
pages, unwinding, and backtrace support in a safer API.

It supplies no scheduler and no I/O reactor. Choosing it means writing both. It
is the right base if `may` is ever abandoned, and it is not a v1 dependency.

#### `crossbeam` -- a toolbox, not a runtime

`crossbeam` provides no task scheduler. Judged part by part:

- `crossbeam-channel` is the obvious Option C implementation under a
  thread-based runtime, and the natural sibling of `may::sync::mpmc` under a
  coroutine one. Keep it in view for version 2.
- `crossbeam-utils::thread::scope` predates `std::thread::scope` and is now
  redundant; use std.
- `crossbeam-deque` is what one uses to *write* a work-stealing scheduler. Snacc
  should not be writing one.
- `crossbeam-epoch` solves reclamation for lock-free structures Snacc has none
  of.

Verdict: not a dependency for this RFC; a likely one for Option C.

#### `chili` -- worth watching for Option D

A low-overhead fork-join library using heartbeat scheduling, with notably lower
per-task cost than rayon for fine-grained work. That is exactly Option D's
profile: a parallel `for` over a large collection with a small body.

Young (0.2.1, March 2025) and small. Not a v1 dependency, but a candidate
optimization behind the same three functions, which is the point of section 4.

#### `stackful` -- rejected

0.1.5, last released June 2022, 45k downloads. It bridges blocking code into an
async runtime by running it on a separate stack, so it presupposes the async
runtime this RFC rejects. Wrong shape and effectively unmaintained.

#### `loom` and `shuttle` -- for testing the runtime, not for shipping

The three ABI functions contain the only `unsafe` in the feature: a `Send`
wrapper around a raw payload pointer whose contract is a lifetime argument, not
a type-system fact. That is exactly what a concurrency model checker is for.
Phase 3 should exercise `snacc_spawn`/`snacc_scope_end` under one of these.

### 4.4 Recommended sequencing

| Stage | Implementation | Trigger |
| --- | --- | --- |
| v1 | `rayon` default, `may` and std behind features | this RFC |
| v2 | `may` default | Specification 023 phase C (sockets) |
| contingency | `corosensei` plus a scheduler | `may` unmaintained when v2 arrives |
| optimization | `chili` for Option D | measured, not assumed |

The Snacc-visible contract is identical at every stage. That is the whole
reason section 4 keeps the boundary at three functions.

## 5. Candidate language surfaces

### 5.1 Option A -- scoped fan-out (`parallel` block with `spawn`)

~~~snacc
parallel do
    spawn sum_range(0, middle, left)
    spawn sum_range(middle, limit, right)
end
~~~

Tasks are named calls. The block ends only when every task it spawned has
finished. Results travel through the existing `Ref<T>` out-parameter idiom.

Adds two keywords and one statement form. Adds no type, no generic, no
allocation, no handle, no ownership rule, and no destructor obligation. Nothing
outlives the block, so there is no leak, no detach, and no lifetime question to
answer. Implementable against the language exactly as it stands.

**Recommended as version 1.**

### 5.2 Option A2 -- `parallel` block with no `spawn` keyword

~~~snacc
parallel do
    worker(1)
    worker(2)
end
~~~

Every call in the body becomes a task. One keyword instead of two.

Rejected. The body still needs ordinary sequential code to compute arguments
and guard spawns, and a rule that says "statements here mean something entirely
different" is exactly the implicit behavior goal 1 rules out. `spawn` at each
site costs one word and removes the ambiguity.

### 5.3 Option B -- task handles (`Task<T>` plus `join`)

~~~snacc
let task: Task<Int64> = spawn compute(30)
let value: Int64 = task.join()
~~~

More expressive: a task can be stored in a local, moved, returned, or held
across unrelated work.

Costs a closed generic type form, move-only classification for the handle
(RFC 016), a join-exactly-once rule or a join-on-destruction rule, heap-
allocated argument payloads instead of stack ones, and a decision about what a
never-joined task means. Returning a `Task<T>` from a function reintroduces
precisely the escaping-lifetime problem that `Ref<T>` was designed to avoid.

**Recommended: defer.** Revisit only when a real program cannot be written with
Option A. Option A composes: a scope may nest inside a task.

### 5.4 Option C -- channels (`Chan<T>`)

~~~snacc
parallel do
    let jobs: Chan<Int64> = channel(16)
    spawn produce(jobs)
    spawn consume(jobs)
end
~~~

Required for pipelines, actors, and any task that must communicate before it
finishes. `may::sync::mpmc` supplies the implementation directly.

The cost is that a channel endpoint must be reachable from two tasks at once,
which is the first and only sharing exception in the language. It is a
defensible one -- a channel is internally synchronized, so sharing it races
nothing -- but it needs RFC 016's ownership model to land first, and it needs a
rule saying a `Chan<T>` value is copyable while nothing else that owns a
resource is.

**Recommended as version 2**, after RFC 016.

### 5.5 Option D -- data-parallel iteration

~~~snacc
for item in parallel items do
    handle(item)
end
~~~

Sugar over Option A once Specification 019 lands `for`. Cheap to add, and the
common case for CPU-bound work over a collection. Needs disjointness of element
places, which is section 7.4's deferred work.

**Recommended as version 3.**

### 5.6 Summary

| Option | New surface | Blocked on | Verdict |
| --- | --- | --- | --- |
| A: `parallel`/`spawn` | 2 keywords | nothing | **v1** |
| A2: implicit spawn | 1 keyword | nothing | rejected |
| B: `Task<T>`/`join` | type form, move-only handle, heap payload | RFC 016 | deferred |
| C: `Chan<T>` | type form, sharing exception | RFC 016 | v2 |
| D: parallel `for` | 1 keyword position | Spec 019 | v3 |

The rest of this RFC specifies Option A.

## 6. Option A: syntax and semantics

### 6.1 Grammar

~~~ebnf
block-element        = variable-declaration
                     | assignment
                     | while-statement
                     | break-statement
                     | if-form
                     | parallel-statement
                     | spawn-statement
                     | expression ;

parallel-statement   = "parallel", "do", block, "end" ;
spawn-statement      = "spawn", postfix ;
~~~

`parallel` and `spawn` become reserved words.

`spawn-statement` reuses `postfix` so a spawn target is written exactly like an
ordinary call, including a method call through a receiver. Section 6.3
restricts which `postfix` forms are accepted; the grammar does not.

### 6.2 `parallel`

`parallel do block end` is a statement, never an expression, exactly like
`while`. Its body is a no-result block and may contain any block element,
including nested `parallel` statements.

The body's own statements execute in the enclosing task, in source order,
exactly as they would without the `parallel` keyword. The keyword changes one
thing: it delimits a scope in which `spawn` is permitted, and the statement
does not complete until every task spawned in that scope has completed.

A `parallel` statement with no `spawn` in its body is legal and equivalent to
its bare block. It is not diagnosed; a body may spawn conditionally.

Tasks may run concurrently, may run on different processors, and may also run
sequentially. A program must not depend on which. Interleaving between tasks,
and between a task and the enclosing task, is unspecified.

`break` is not permitted as a direct element of a `parallel` body, and neither
is a `break` in a nested `if` whose target loop encloses the `parallel`
statement. A `break` inside a `while` that is itself inside the body is
ordinary and permitted. This restriction exists only because leaving a scope
early requires the same scope-exit cleanup machinery RFC 016 introduces; it is
lifted once that machinery exists.

Specification 026 applies the same rule to `return`: a return inside a
`parallel` body is rejected until a checked exit edge can call
`snacc_scope_end`, wait for every spawned task, and then continue the callable
return. A return executed inside the separately declared function or method
invoked by `spawn` exits only that task's callable and does not cross the
spawning scope.

### 6.3 `spawn`

`spawn call` is a statement valid only as an element of a `parallel` body,
directly or inside `if` and `while` forms nested in that body. A `spawn`
outside any `parallel` body is an error.

The call target must be a declared function or a method reached through a
receiver. It must have no result, because a task's completion is observed by
the scope, not by an expression. `spawn f(x)` where `f` has a result is an
error; the program writes the result through a `Ref<T>` parameter instead.

An `extern rust` bridge may be spawned. Section 8 states the extra obligations
that places on the host.

Arguments are evaluated in the enclosing task, at the point the `spawn`
statement executes, left to right, under the ordinary argument rules. A value
argument is copied then; a `Ref<T>` argument fixes its place then. The task
begins after every argument of that spawn is evaluated. The enclosing task then
proceeds to the next statement without waiting.

Every task spawned in a scope completes before the `parallel` statement
completes. A task that does not terminate prevents the statement from
completing; the compiler diagnoses neither, exactly as it diagnoses no other
nontermination.

### 6.4 Worked example

~~~snacc
fun sum_range(from: Int64, to: Int64, into: Ref<Int64>) do
    let mut total: Int64 = 0
    let mut index: Int64 = from
    while index < to do
        total = total + index
        index = index + 1
    end
    into = total
end

fun sum_to(limit: Int64): Int64 do
    let mut left: Int64 = 0
    let mut right: Int64 = 0
    let middle: Int64 = limit / 2
    parallel do
        spawn sum_range(0, middle, left)
        spawn sum_range(middle, limit, right)
    end
    left + right
end

print(sum_to(1000000))
~~~

`left` and `right` are distinct places, so the two tasks share nothing. The
addition after the block reads them only once the scope has joined both tasks,
which section 7.2 enforces and section 9 delivers.

### 6.5 What is deliberately absent

No cancellation, no timeout, no priority, no task identity, no yield, no sleep,
no affinity, and no stack-size syntax. `may` supports cancellation; it is not
exposed, because unwinding a Snacc task has no defined meaning in a language
with no error model.

## 7. Checker rules

### 7.1 Placement and target

1. `spawn` outside a `parallel` body is an error.
2. A spawn target that is not a declared function, bridge, or resolved method
   call is an error, with the same diagnostic vocabulary as an ordinary call.
3. A spawn target with a result type is an error, naming the result type and
   pointing at `Ref<T>` as the way to return a value.
4. `break` or `return` that crosses a live parallel scope as described in
   section 6.2 is an error until its required join cleanup edge exists.

### 7.2 Reference exclusivity across a scope

The existing rule -- two `Ref<T>` arguments in one call must not overlap -- is
extended across the scope, because tasks in a scope overlap in time:

5. Two `Ref<T>` arguments to two spawns in the same scope must not overlap.
   Overlap is the existing definition: identical places, or one reached by
   selecting fields from the other.
6. Between a `spawn` and the end of its scope, the enclosing task must not read
   or write any place passed by `Ref<T>` to that spawn, nor any place
   overlapping one. Both the read and the borrow are reported.
7. A `spawn` with a `Ref<T>` argument must not appear inside a `while` in the
   body, unless the argument's root is declared inside that loop. A loop
   otherwise creates an unbounded number of simultaneous borrows of one place.

Rule 6 is the only new dataflow analysis, and it is narrow: one lexical block,
places rooted in locals and parameters, no interprocedural reasoning. The
existing overlap predicate does the work.

A value argument needs no rule. It is copied before the task starts, and Snacc
values are either scalars or aggregates of scalars, with no interior sharing.

### 7.3 Why nothing else is needed

There is no `Send` rule because there is nothing unsendable to name. There is
no capture rule because there are no closures. There is no global rule because
there are no globals. There is no escape rule because `Ref<T>` cannot be
stored, returned, or nested, and the scope outlives every task.

This holds only while the language has no shared-ownership facility. RFC 016
section 9 explicitly leaves `Shared<T>` unspecified. **If a shared-ownership
type is ever added, it must be specified together with its concurrency rule in
the same change**, or this section becomes false.

### 7.4 Deferred checker work

Fan-out into distinct elements of one array -- `spawn work(i, results[i])` --
requires proving index places disjoint. That needs Specification 019's index
places and is deferred. Until then, fan-out is written with one named local per
task, or with a channel once Option C lands.

## 8. Runtime obligations and the Rust bridge

A task runs on a scheduler worker. Two host obligations follow, and both are
contract text, not compiler-checkable:

1. **A bridge called from a task must not block its worker thread on something
   only another task can release.** Blocking on a `std::sync::Mutex` held by a
   coroutine that is itself waiting for a worker is a deadlock. A bridge that
   synchronizes at all must use the scheduler's own primitives.
2. **A bridge must not hold a reference to thread-local storage across a
   suspension point.** A task may resume on a different worker thread, so a TLS
   address cached before a suspension is wrong after it. This is `may`'s
   documented hazard; Snacc itself emits no thread-local storage.

3. **A bridge must fit a task's stack.** Under a coroutine runtime a task's
   stack is fixed and far smaller than a thread's. Host code that recurses
   deeply, holds a large stack buffer, or calls a library written for ordinary
   thread stacks can overflow it, and the resulting fault names no bridge. See
   section 14.10.

A bridge that blocks the worker without deadlocking is legal and costs
throughput, not correctness. Blocking file and socket I/O is in this category.

Obligations 2 and 3 apply only to a coroutine runtime. Under the recommended
`rayon` default (section 4.3) a task never suspends and runs on a real thread
stack, so both are vacuous -- which is one reason to start there.

The existing rule that a bridge must not unwind across the ABI boundary is
unchanged and now also protects the scheduler.

`print` from a task is permitted. Each `print` writes one complete line
atomically; the order of lines across tasks is unspecified. Conformance tests
for concurrent programs must therefore either join before printing or compare
output as a set.

## 9. Runtime ABI

ABI version advances to **7**. Version 7 adds three imports and changes nothing
else. No version 6 object, runtime, or cached artifact is accepted by a
version 7 build.

| Symbol | Rust signature |
| --- | --- |
| `snacc_scope_begin` | `extern "C" fn() -> *mut SnaccScope` |
| `snacc_spawn` | `unsafe extern "C" fn(*mut SnaccScope, unsafe extern "C" fn(*mut u8), *mut u8)` |
| `snacc_scope_end` | `unsafe extern "C" fn(*mut SnaccScope)` |

`SnaccScope` is opaque to generated code, which only ever holds the pointer.
The scope is allocated by the runtime rather than by an `alloca` so that its
size and alignment are not frozen into an object file.

`snacc_scope_end` joins every task spawned into the scope, then frees it. It
returns only when all of them have completed.

The `entry` pointer is a compiler-generated thunk (section 10.2). The `payload`
pointer addresses storage in the spawning function's frame. The runtime treats
both as opaque and must not touch the payload after the corresponding task
returns.

### 9.1 Reference implementation

This is the `may` implementation, shown first because it is the one with
interesting safety obligations. The `std::thread::scope` implementation differs
only in which `spawn` and `JoinHandle` it names. The `rayon` implementation
cannot use this shape at all and is described in section 14.14.

~~~rust
pub struct SnaccScope {
    handles: Vec<may::coroutine::JoinHandle<()>>,
}

/// The payload lives in the spawning frame, which `snacc_scope_end` outlives,
/// and the checker forbids the spawner from touching it before the join. The
/// pointers are therefore valid for the task's whole lifetime and reach no
/// storage any other task reaches.
struct Task(unsafe extern "C" fn(*mut u8), *mut u8);
unsafe impl Send for Task {}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_scope_begin() -> *mut SnaccScope {
    Box::into_raw(Box::new(SnaccScope {
        handles: Vec::new(),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snacc_spawn(
    scope: *mut SnaccScope,
    entry: unsafe extern "C" fn(*mut u8),
    payload: *mut u8,
) {
    let task = Task(entry, payload);
    // SAFETY: see `Task`. The spawned body neither unwinds nor uses TLS.
    let handle = unsafe {
        may::coroutine::spawn(move || {
            let task = task;
            unsafe { (task.0)(task.1) }
        })
    };
    // SAFETY: `scope` came from `snacc_scope_begin` and is not yet ended.
    unsafe { &mut *scope }.handles.push(handle);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snacc_scope_end(scope: *mut SnaccScope) {
    // SAFETY: `scope` came from `snacc_scope_begin` and is ended once.
    let scope = unsafe { Box::from_raw(scope) };
    for handle in scope.handles {
        handle.join().expect("a Snacc task panicked");
    }
}
~~~

Every alternative implementation (section 4.3) replaces the body of these three
functions and nothing else.

### 9.2 Scheduler configuration

Worker count defaults to the processor count and is overridden by
`SNACC_WORKERS`; under a coroutine runtime, task stack size defaults to a value
large enough for generated frames and is overridden by `SNACC_STACK_SIZE`.
`may::config().set_workers(n)` must run before the first spawn; `rayon` takes
the same setting through its global pool builder. The runtime owns both, not
the language.

Configuration stays out of the language because it is a deployment property,
not a program property, and because putting it in the language would freeze
`may`'s configuration model into the contract that section 4 keeps loose.

### 9.3 Stack exhaustion under a coroutine runtime

This section applies to `may`, not to the recommended `rayon` default, whose
tasks run on ordinary thread stacks.

`may` gives each coroutine a fixed stack with a guard page; overflowing it
faults -- provided the frame that overflows is small enough to land on the
guard page rather than jump it, which section 14.9 does not currently
guarantee. Snacc programs already accept this failure mode: `LANGUAGE.md` states
that unbounded recursion "exhausts the native call stack, or fails some other
platform-dependent way". A guard-page fault is a hard stop, not a memory error,
so goals 13 and 15 hold. The practical difference is that a task's stack is
smaller than a thread's, so a program that recurses deeply inside a task fails
where the same code outside one does not.

`snacc_main` should therefore run inside a task itself, with the same stack
size as every other task, so that one knob governs all Snacc code and the main
task is not a privileged special case. This is a change to both host templates.

## 10. Backend

### 10.1 Scope lowering

A `parallel` statement lowers to:

1. `%scope = call ptr @snacc_scope_begin()`
2. the body's own lowering, unchanged except for `spawn`
3. `call void @snacc_scope_end(ptr %scope)`

The scope pointer is an ordinary SSA value in the enclosing function. Nested
`parallel` statements nest their pointers. Because section 6.2 forbids `break`
out of a body, step 3 is on every path out of the block and needs no cleanup
landing pad.

### 10.2 Spawn lowering

A `spawn` site lowers to a payload store and one call. The payload struct type
is exactly the LLVM parameter tuple of the callee, which `function_type`
already computes: a value parameter contributes its lowered type, and a
`Ref<T>` parameter contributes an opaque pointer -- the same pointer an
ordinary call would pass, so a reference argument needs no special treatment.

Payload storage is an `alloca` in the enclosing function's entry block,
following the existing `entry_alloca` convention. A spawn inside a loop reuses
one `alloca` per site, which is sound because rule 7 restricts what a looping
spawn may borrow and value arguments are copied into the payload before the
task starts. Nothing is heap-allocated.

For `spawn sum_range(0, middle, left)` the backend emits, once per spawn site:

~~~llvm
%payload.0 = type { i64, i64, ptr }

define internal void @snacc_task_sum_range_0(ptr %payload) {
entry:
  %from.ptr = getelementptr inbounds %payload.0, ptr %payload, i32 0, i32 0
  %from = load i64, ptr %from.ptr
  %to.ptr = getelementptr inbounds %payload.0, ptr %payload, i32 0, i32 1
  %to = load i64, ptr %to.ptr
  %into.ptr = getelementptr inbounds %payload.0, ptr %payload, i32 0, i32 2
  %into = load ptr, ptr %into.ptr
  call void @snacc_fn_sum_range(i64 %from, i64 %to, ptr %into)
  ret void
}
~~~

and at the site:

~~~llvm
  ; store arguments, evaluated left to right, into %args.0
  call void @snacc_spawn(ptr %scope, ptr @snacc_task_sum_range_0, ptr %args.0)
~~~

Thunks carry internal linkage and a generated symbol
`snacc_task_<callee>_<site>`, matching the existing `snacc_fn_` and
`snacc_method_` conventions. A method spawn's payload leads with the receiver
pointer, exactly as a method call's argument list does.

The thunk is why no closure is required. The compiler already knows the callee
at every spawn site -- Snacc has no indirect calls -- so one monomorphic
adapter per site replaces the entire captured-environment machinery an `async`
design would need.

### 10.3 Scale of the change

- `syntax`: two keywords, two statement nodes, two parser arms.
- `semantics`: two statement variants, seven rules from section 7, one new
  dataflow pass over one block.
- `backend`: one payload struct per spawn site, one thunk per spawn site, three
  imported declarations, two statement lowerings.
- `runtime`: three functions, one dependency, one configuration hook.

No change to the type system, the layout algorithm, the calling convention, or
the existing lowering of any construct.

## 11. Build integration

This is the largest engineering risk in the proposal, and it is not a language
problem.

`snacc-driver` builds the host by writing `snacc-runtime`'s source into one
file and compiling it with bare `rustc`. That path cannot depend on `may`.
`cargo-snacc` builds a real Cargo host that can. The direct driver is what the
`snacc` CLI and the whole `tests/cases/run` conformance suite use, so
concurrency has to work there.

Three options:

1. **Link the built runtime rlib.** `snacc-driver` stops embedding the runtime
   source and instead passes `--extern snacc_runtime=<path>` plus
   `-L dependency=<deps>`. Smallest conceptual change, but the rlib path must
   be discovered at run time and stays correct only for a toolchain shipped
   with its dependency graph.
2. **Generate a Cargo project.** `snacc-driver` writes a small crate with a
   path dependency on `snacc-runtime` and runs `cargo build`. Robust and
   reuses machinery `cargo-snacc` already has; costs a Cargo invocation per
   build and needs a stable way to locate the runtime crate.
3. **Make `cargo-snacc` the only hosted path.** The direct driver keeps working
   for programs that do not spawn and rejects `parallel` with a diagnostic.
   Cheapest, and wrong: one language with two capability levels contradicts
   goals 1 and 2.

**Recommended: option 2**, with option 1 as a fallback if Cargo invocation cost
proves unacceptable. This is a prerequisite phase, not a side effect, and it
should be spiked before the language work starts.

## 12. Detailed implementation plan

### Phase 0: build integration

Resolve section 11 with a working spike: a `parallel`-free program built
through the new host path, byte-identical behavior, conformance suite green.
Nothing after this phase is worth starting until this one lands.

### Phase 1: syntax

Reserve `parallel` and `spawn`; add `Parallel(Block)` and `Spawn(Call)` to the
AST; parse both; update `GRAMMAR.ebnf` and `LANGUAGE.md`'s copy together.
Parser tests in `tests/parse` covering nesting, a spawn of a method call, and
rejection of `spawn` as an expression.

### Phase 2: checker

Add the two statement variants to the checked program. Implement rules 1--4,
then the scope-aware borrow analysis for rules 5--7. Diagnostics name both the
spawn and the conflicting access. Tests in `tests/typecheck` for each rule.

### Phase 3: runtime and ABI

Add the three functions to `snacc-runtime` with the `rayon` implementation as
the default (section 4.4) and the `may` and `std::thread::scope`
implementations behind feature flags, so the seam in section 4 is exercised
rather than asserted. Resolve section 14.14 before writing the rayon wait loop.
Test the raw-pointer handoff under `loom` or `shuttle`. Raise `ABI_VERSION` to
7 in both crates. Under a coroutine runtime only, move `snacc_main` into a task
and re-run the existing suite against the chosen stack size (section 14.11).
Update the ABI table in `LANGUAGE.md`.

### Phase 4: backend

Payload struct types, thunk emission, the three imported declarations, and the
two statement lowerings. Verify emitted IR through `emit_llvm_ir`, which is the
only place calling-convention detail is observable.

### Phase 5: conformance and documentation

Run cases in `tests/cases/run/pass` for scoped fan-out, nested scopes,
conditional spawn, spawn of a bridge, and a task count well above the worker
count. Fail cases for every diagnostic in phase 2. Order-insensitive output
comparison for interleaved `print`. `LANGUAGE.md` gains a concurrency section;
this file's status becomes terminal.

## 13. Acceptance criteria

1. Section 6.4 compiles, runs, and prints the same value as its sequential
   equivalent, on both the direct and the Cargo-hosted path.
2. With `SNACC_WORKERS` above one, a two-task CPU-bound program completes in
   measurably less wall-clock time than its sequential form. Run on demand,
   not in the always-on suite (section 14.25).
3. With `SNACC_WORKERS=1`, every conformance case still passes.
4. Every conformance case passes identically under all three implementations
   -- `rayon`, `may`, and `std::thread::scope` -- with the std one treated as
   the oracle when they disagree.
5. Every rule in section 7 has a rejecting test with a diagnostic naming both
   involved places.
6. A `parallel` statement never completes before its tasks do, verified by a
   case whose tasks write through `Ref<T>` and whose enclosing code reads the
   results immediately after the block.
7. The ABI version mismatch check fires for a version 6 artifact.

## 14. Open questions, risks, and concerns

Everything known to be unresolved or uncomfortable about this proposal is
recorded here. Items marked **gap** are places where the specification above is
incomplete and must be written before implementation; items marked **risk** are
decided but dangerous; the rest are choices awaiting a decision.

### Language and semantics

**14.1 Keyword name.** `parallel` says what it is for; `concurrent` is more
accurate, since section 6.2 permits sequential execution. `parallel` is
recommended for length and familiarity, but the semantics text then has to keep
saying "may run in parallel", which is exactly the mismatch goal 1 dislikes.

**14.2 A `parallel` body with no `spawn`.** Section 6.2 says it is legal and
equivalent to its bare block. Goal 2 says there should be one obvious way to do
things, and a keyword that sometimes means nothing is two ways. The alternative
-- diagnose a scope that provably spawns nothing -- is easy for a body with no
`spawn` token anywhere and useless for one whose only `spawn` sits inside a
false `if`. Decide whether the cheap syntactic check is worth having.

**14.3 Method receivers are not covered by section 7. (gap)** Section 10.2 says
a method spawn's payload leads with the receiver pointer, which makes the
receiver a borrow with exactly the aliasing hazard rules 5--7 exist for. Rules
5--7 mention only `Ref<T>` arguments. The receiver place must be added to the
overlap set, and `spawn point.translate(1.0, 2.0)` must conflict with any other
spawn in the same scope that touches `point`. Until this is written, the
specification is unsound for method spawns.

**14.4 Receiver-mutability fixed point. (gap)** `LANGUAGE.md` computes "may
assign through `self`" as a fixed point over the whole call graph. A `spawn` of
a method is a call edge in that graph and must participate. The interaction
with a `parallel` block inside a method body is unwritten.

**14.5 Value arguments read from a borrowed place.** Rule 6 forbids the
enclosing task from reading a place borrowed by a live spawn, which already
covers `spawn a(x, result)` followed by `spawn b(result)` passing `result` by
value. This is correct and non-obvious; the specification should say so
explicitly rather than leave it to be re-derived.

**14.6 Unbounded task creation. (risk)** A `spawn` inside a `while` inside a
`parallel` has no bound, no backpressure, and no admission control. Under a
coroutine runtime each task costs a stack; a loop spawning a million tasks
exhausts memory with a program that contains no obvious error. Nothing in the
proposal caps this, and nothing warns. Options: a runtime queue with
backpressure, a documented cost model and nothing else, or a diagnostic on an
unbounded spawn loop. Currently: nothing, deliberately, and that may be wrong.

**14.7 Recursive and nested scope explosion.** A spawned function may itself
contain a `parallel` block, including recursively. Task counts multiply. This
is the correct semantics -- it is how divide-and-conquer is written -- but the
cost model needs stating alongside 14.6.

**14.8 `break` and `return` restriction.** Lift both in the same change as RFC
016's scope-exit cleanup by joining every spawned task before control leaves,
or keep them permanently as a structured-concurrency property? Specification
026 requires that a return never bypass the join and otherwise permits either
staging choice. Leaving a scope early while tasks run has no meaning without
the join or cancellation, which section 6.5 declines to add.

### Safety and soundness

**14.9 Large stack frames can jump a guard page. (risk)** A single guard page
stops a stack that grows one page at a time. A function whose frame is larger
than the guard region can write past it and corrupt unrelated memory, which is
a memory error, not a clean fault -- so goal 15 fails, not merely goal 13.
Snacc passes structs by value and has no frame-size limit, so large frames are
reachable from ordinary source. Rust emits stack probes on the affected
targets; Snacc emits none. Mitigations: emit the `probe-stack` function
attribute in the backend, bound frame size, or size coroutine stacks so far
above any reachable frame that the question is moot. **This must be resolved
before `may` becomes the default implementation.** It does not arise under the
recommended `rayon` default, which uses real thread stacks.

**14.10 A bridge does not know it is on a small stack. (risk)** A spawned
`extern rust` bridge runs on a coroutine stack under `may`. Host code written
against ordinary thread stacks -- a recursive parser, a large stack buffer, a
library that assumes megabytes -- can overflow it. Section 8 must state this as
a third host obligation, and the diagnosis when it happens is a fault with no
attribution to the bridge.

**14.11 Deep recursion regresses. (risk)** Section 9.3 puts `snacc_main` inside
a task for uniformity. A conformance case that recurses deeply and passes today
on an 8 MB main-thread stack can fault on a smaller task stack. The existing
suite must be re-run against the chosen stack size before this is accepted, and
the result may force the stack size rather than the other way around.

**14.12 Goal 14 weakens. (risk)** "If it compiles, it runs" survives the rules
in section 7 -- there is no shared state, so no data race and no nondeterministic
result. It does not survive section 8: a bridge that blocks a worker on
something only another task can release deadlocks a program that compiled
cleanly. The language's strongest promise now has a documented hole whose edge
is enforced by prose. This is the single largest philosophical cost of the
proposal and should be stated in `LANGUAGE.md`, not buried here.

**14.13 Section 7.3 has an expiry date.** The claim that no `Send` reasoning is
needed holds only while nothing in the language shares ownership. RFC 016
leaves `Shared<T>` open. If it ever lands without a concurrency rule in the
same change, this specification silently becomes false. The dependency should
be recorded in RFC 016, not only here.

### Runtime and dependencies

**14.14 `rayon` and the begin/spawn/end shape. (gap)** `rayon::scope` is
closure-scoped and cannot be held across three FFI calls, so section 9.1's
`Vec<JoinHandle>` implementation does not transfer. The rayon implementation
needs `rayon::spawn` plus a completion counter, and `snacc_scope_end` must
`rayon::yield_now()` while waiting so a nested scope inside a pool thread
steals work rather than deadlocking. Section 9.1 currently shows only the `may`
implementation; the rayon one has to be written and reviewed, because that
deadlock is the classic way to get this wrong.

**14.15 Cost to programs that never spawn.** Goal 12 asks for no or low runtime
overhead. Linking a scheduler into every binary costs size even when no
`parallel` appears, and the runtime must be lazily initialized so it costs no
startup time or threads. Measure both. If the cost is real, the runtime can
feature-gate the scheduler, but the compiler cannot know at runtime-build time
whether a future program will spawn, so the gate has to be a build-time choice
of the host -- which reopens section 11.

**14.16 Stack size unit and default.** `may`'s `set_stack_size` is documented
as "in usize", not in bytes. The default must be measured against real
generated frames and against 14.9 and 14.11, and the value belongs in the
runtime, not in this document.

**14.17 `print` line atomicity.** Guaranteed by section 8. That constrains the
runtime to one write per `print`, which the current `println!` implementation
satisfies but does not promise. Either make the runtime hold the lock
explicitly or drop the guarantee. [Specification 023](023-input-and-output.md)
section 8.1 takes the first option: `print` writes through the same `Output`
object `stdout()` returns, and atomicity becomes deliberate rather than
inherited. This item is closed if that specification lands first.

**14.18 `print` becomes a contention point.** Every task printing takes the
same process-wide stdout lock. Under a thread-based runtime that serializes
throughput; under `may` it blocks a worker, which is legal per section 8 but
uncomfortable in the runtime's own code. Buffering per task changes ordering
and interacts with 14.17. Specification 023 section 15.18 carries this forward
with the buffering options laid out.

**14.19 Task panics.** `snacc_scope_end` aborts on a panicking task. A bridge
must not unwind and Snacc cannot panic, so this is unreachable in a correct
program -- but "unreachable" needs to be a stated contract, and the abort
message needs to name the situation clearly enough to debug.

**14.20 Windows behavior is assumed, not verified.** `may` lists x86_64 Windows
as supported, which is this compiler's own host. Guard-page behavior, fault
reporting, and worker thread setup on Windows are untested by this proposal and
should be part of the phase 0 spike, not discovered in phase 4.

**14.21 The `unsafe` contract is a lifetime argument, not a type.** The `Send`
wrapper in section 9.1 is sound only because the checker enforces rules 5--7
and because `snacc_scope_end` joins before the frame dies. A checker bug is
memory corruption, not a diagnostic. That is a reason to test the runtime under
`loom` or `shuttle` (section 4.3), and a reason to keep the `std::thread::scope`
oracle.

### Build, test, and tooling

**14.22 Build integration.** Which of section 11's three options, and does a
Cargo invocation per direct build cost more than goal 8 tolerates? This is the
first thing to resolve and the only item that blocks every other phase.

**14.23 Diagnostics need two spans, and the infrastructure has one. (gap)**
`Diagnostic` carries a single optional span. Rule 6's message must point at the
borrow and at the conflicting use; rule 5's must point at both spawns. Either
multi-span diagnostics arrive first, or these messages degrade to one span plus
prose. RFC 016's move checker needs the same thing, so this is shared
infrastructure worth building once rather than a concurrency-specific cost.

**14.24 The conformance harness compares exact stdout.** Section 8 makes
interleaved output unordered, so the harness needs an order-insensitive
comparison mode, or every concurrent case has to join before printing. The
latter is cheaper and tests less.

**14.25 Acceptance criterion 2 is a benchmark, not a test.** A wall-clock
speedup assertion is flaky on shared CI. It should be an explicitly ignored
benchmark run on demand, in the style of the existing ignored cargo-hosted
tests, and the always-on criteria should be the deterministic ones.

**14.26 `snacc-workbench` executes built programs** and will now execute
multi-threaded ones. Its output capture and process handling need a look, kept
to the minimum `AGENTS.md` allows for the workbench.

**14.27 Reproducible builds.** Thunk symbols are named by spawn-site index, so
the numbering must derive from source order and nothing else -- not from hash
map iteration, which the module builder is already careful about for functions.

### Interactions with pending specifications

**14.28 RFC 016 destruction inside a task.** A task that owns a box must run
its cleanup before the task ends, inside the thunk. This is the ordinary
scope-exit plan applied to a function the compiler generates, but the two
specifications should say so in the same words.

**14.29 RFC 016 makes handoff better, not worse.** Passing a list or a box
*by value* into a spawn is a move: the task takes sole ownership, and the
spawner cannot touch it afterwards because the move checker already says so.
That is a clean, checked ownership transfer that needs no new rule -- worth
stating positively, since it is the main reason Option C's channels become
tractable after RFC 016.

**14.30 Specification 019 element disjointness.** Section 7.4 defers fan-out
into array elements. Until it lands, the natural way to write a parallel map is
unavailable, and every example needs one named local per task. This is the
largest expressiveness gap in version 1.

**14.31 Single-worker deadlock arrives with channels.** With
`SNACC_WORKERS=1`, version 1 is safe: tasks never communicate, so they simply
run one after another. Option C breaks that -- a task blocked on a channel with
one worker and a non-yielding peer deadlocks. Option C must specify a minimum
worker count or require yielding channel operations.

## 15. Rejected alternatives

### `async`/`await` over `tokio`

Requires closures, function values, generic futures, function coloring, and a
state-machine lowering, each a language feature Snacc has deliberately
declined. It would also make every concurrent function a different kind of
function, contradicting goal 3. Stackful coroutines buy the same concurrency
for zero language surface, which is the entire argument of section 3.1.

Section 4.3 evaluates the async runtimes as implementations rather than as
models; the rejection here is about language surface, not runtime quality.

### Raw OS threads as the model

`std::thread::scope` alone is a legitimate implementation (section 4) but a
poor model: a thread per task caps useful task counts in the thousands, and any
blocking call ties up a whole thread. Adopting it as the *contract* would
forbid the coroutine implementation later. Adopting it as an *implementation*
costs nothing, which is why section 4.4 keeps it.

### Shared memory with a `Mutex<T>` type

The obvious C-shaped design, and the one Snacc is best positioned to refuse.
Shared mutable state is exactly what the language does not currently have, and
adding it would require the `Send`/`Sync` reasoning that section 7.3 shows is
otherwise unnecessary. Message passing and `Ref<T>` out-parameters cover the
cases a small language needs.

### Exposing `may` in the language surface

Spelling `may`'s configuration, cancellation, or scheduling in Snacc syntax
would freeze one crate's model into the language contract and make section 4's
fallback impossible.

## 16. Related work: HTTP, and what it actually needs

`may_minihttp` (0.1.11, September 2024, ~33k downloads) is the HTTP server
built on `may`; there is no maintained `may-http`. It is worth being direct
about its maturity before any part of the toolchain depends on it.

More importantly, **nothing in this RFC lets a Snacc program serve HTTP**, and
the missing piece is not concurrency. `may_minihttp` owns its accept loop and
calls a handler. Calling a handler means Rust calling into Snacc, and the Rust
bridge runs only in the Snacc-to-Rust direction. There is no way to name a
Snacc function from the host.

The missing feature is an **export direction for the bridge** -- something like
`export fun handle(...)` giving a declared Snacc function a stable C ABI symbol
the host can call. That is a separate, small, and independently useful RFC, and
it is the correct place for the HTTP story:

- the Rust host owns the server, the runtime configuration, and the socket
  types;
- Snacc supplies a handler function over bridge-representable arguments;
- Snacc's own concurrency feature never has to model I/O readiness, sockets, or
  request lifetimes at all.

### 16.1 Specification 023 changes this conclusion

[Specification 023](023-input-and-output.md) gives Snacc its own sockets, which
opens a second route that did not exist when this section was written:

| Route | Needs | Server owner |
| --- | --- | --- |
| host-owned | bridge export, Spec 019 views | Rust (`may_minihttp`) |
| Snacc-owned | Spec 023 phase C only | Snacc, over `TcpListener` |

Specification 023 section 11 writes the accept loop in Snacc, with `spawn`
moving each `TcpStream` into its own task. That is a working concurrent server
with no bridge export, no `may_minihttp`, and no dependency on a 0.1.x crate
last released in 2024 -- at the cost of implementing HTTP itself.

The host-owned route remains the right answer for reusing a real HTTP stack,
and bridge export remains independently valuable. But it is no longer the
*only* route, which materially lowers the risk this section raises about
`may_minihttp`'s maturity.

Bridge-representable arguments are today's scalars, so a host-owned handler
also waits on Specification 019's borrowed-view bridge adapters for request
bodies. The dependency order for that route is therefore: this RFC, then
bridge export, then
Specification 019's views, then an HTTP host that is a library rather than a
language feature.

## 17. Deferred work

- task handles and `join` (section 5.3);
- channels (section 5.4);
- parallel `for` (section 5.5);
- fan-out into array elements (section 7.4);
- cancellation, timeouts, and task-local state;
- non-blocking sockets and files, which belong to a host library;
- bridge export, and HTTP above it (section 16);
- the concurrency rule for any future shared-ownership type (section 7.3);
- backpressure or admission control for unbounded spawn loops (section 14.6);
- multi-span diagnostics, shared with RFC 016's move checker (section 14.23).

## 18. References

- [`LANGUAGE.md`](../../LANGUAGE.md)
- [RFC 014: Generic Programming](014-generic-programming.md)
- [RFC 016: Box Indirection and Recursive Data](archive/016-box-indirection-and-recursive-data.md)
- [RFC 017: UTF-8 Strings and Views](017-utf8-strings-and-views.md)
- [Specification 018: Inline Sum Types](archive/018-inline-sum-types.md)
- [Specification 019: Collections and Iteration](019-collections-and-iteration.md)
- [Specification 023: Input and Output](023-input-and-output.md)
- [Specification 026: Return Statements](026-return-statements.md)
- `may` 0.3.51 -- https://github.com/Xudong-Huang/may
- `may_minihttp` 0.1.11 -- https://github.com/Xudong-Huang/may_minihttp
- `generator` 0.8.9 -- https://github.com/Xudong-Huang/generator-rs
- `rayon` 1.12.0 -- https://github.com/rayon-rs/rayon
- `crossbeam` 0.8.4 -- https://github.com/crossbeam-rs/crossbeam
- `corosensei` 0.3.4 -- https://github.com/Amanieu/corosensei
- `chili` 0.2.1 -- https://github.com/dragostis/chili
- `stackful` 0.1.5 -- https://github.com/nbdd0121/stackful
