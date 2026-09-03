# RFC 016 Implementation Plan: Box Indirection and Recursive Data Structures

Document kind: Execution plan

Specification: [docs/specs/016-box-indirection-and-recursive-data.md](016-box-indirection-and-recursive-data.md)

Base: after Specification 018 lands in full (Task A committed at ce19c80; Task
B in flight). Section 5.2 of RFC 016 shows an inline-optional-link example
(`Box<Node> | Nil`) that needs `Ty::Sum` to exist; the rest of RFC 016 (box
allocation, ownership, moves, cleanup, LLVM lowering) does not depend on
generics and does not strictly need inline sums, but landing after 018 avoids
a second pass over `Ty`/checker plumbing.

This plan exists only to fix task boundaries and flag the architectural gap
this RFC closes; the specification is the authority on behavior.

## Prior state (verified, not assumed)

- There is currently **no move/ownership machinery of any kind** in this
  compiler. `Binding { name, ty, mutable }` (checker.rs) tracks only a
  boolean mutability flag per local; nothing tracks "available or moved."
  Every existing Snacc value (scalars, structs, named unions, `Ref<T>`
  referents, and — after RFC 018 — inline sums) is copyable. RFC 016 is the
  **first** feature to introduce a move-only value category, so this plan
  must design that dataflow analysis from scratch, not extend an existing one.
- `Place`/`PlaceRoot` (checker.rs) already model "a root plus field
  selectors" and are reused unchanged by `Ref<T>` arguments and union-test
  bindings (Specification 011). RFC 016 section 7.3's branch-scoped union
  alias reuses this same shape — confirm it still does once `Box<T>` fields
  exist, rather than inventing a second place representation.
- The existing receiver-write analysis (`solve_receiver_writes`, a monotone
  least-fixed-point worklist over a call graph) is the direct precedent for
  RFC 016's move-only structural computation (spec section 5.3: move-only is
  transitive through fields/members) — that one is a fixed point over the
  *type* dependency graph instead of the *call* graph, but the same
  worklist-to-a-fixed-point pattern applies and should be reused, not
  reinvented.
- The by-value layout-cycle graph (`reject_layout_cycles` in types.rs, a
  three-state DFS) already exists and, after RFC 018, already treats a sum's
  members as edges (see `contained()`). RFC 016 phase 2 needs this same graph
  with box edges *excluded* — i.e., two separate graphs: one for finite layout
  (box terminates the edge) and one for complete semantic/resolution
  dependency (box's pointee still needs its type resolved). Do not collapse
  these into one graph with a "skip if box" flag scattered through call
  sites; give layout-cycle detection its own edge function that differs from
  `contained()` only at the box case.
- No allocator, drop/cleanup, or "runtime fatal error" path exists yet in
  `snacc-runtime`. Section 8.2's allocation-failure fatal path is new runtime
  surface, not a variation on something existing.
- Rust bridge rejection for a category of type (standalone `Nil`, `Ref<T>`,
  and after RFC 018, inline sums) already has an established pattern in
  `apps/cargo-snacc/src/main.rs`'s `rust_abi_type`/`rust_abi_param_type` and
  in declaration-collection checks — box-containing types reuse this same
  pattern, transitively (a struct containing a box is also rejected).

## Why this is split into three tasks, not one

RFC 016 section 12 already lists six phases. This plan collapses them into
three dispatch units along the same "front end / middle / back end" line
used for RFC 018, but each unit here is larger than RFC 018's because phase 3
(ownership analysis) is genuinely new compiler infrastructure with no
existing analog to extend — it needs its own task rather than being folded
into either neighbor.

## Task A: syntax, resolved types, and layout (RFC phases 1-2)

1. Reserve `Box` in type position and `box` in expression position (parser).
   Parse `Box<T>` using the same closed-angle-bracket tokenization already
   established for `Ref<T>` — do not add a new generic-application grammar
   rule. Parse `box(expression)` as a distinct allocation expression node,
   not a call.
2. Add a resolved `Ty::Box(BoxId)` (or equivalent boxed-pointee interning,
   mirroring how RFC 018's `SumTable` interns member sets — a box's identity
   is just its pointee type, so this may be simpler: intern by pointee `Ty`
   directly, no separate ID needed unless recursion through the type table
   requires one). Reject a non-storable pointee (no-result types, `Ref<T>`)
   and reject `Box` used as a user-declared name anywhere Snacc validates
   reserved words today.
3. Reject box-containing types in `extern rust` parameters/results (extend
   the existing bridge-rejection pattern transitively).
4. Compute per-type storable/size/alignment/move-only status as one
   structural fixed point over the type dependency graph (see "Prior state"
   above re: reusing the receiver-write fixed-point pattern). A struct/union
   is move-only iff any field/member is; `Box<T>` is always move-only
   regardless of `T` (spec 5.3). This is the point where RFC 018's inline
   sums *do* need their move-only property finally computed for real
   (018 Task A deliberately left it a structural no-op since nothing
   consumed it yet — confirm 018 Task B didn't accidentally hardcode
   "always copyable" somewhere that this task now needs to generalize).
5. Give the by-value layout-cycle graph a second, box-excluding edge
   function (see "Prior state") so `type A is struct b: Box<B> end` /
   `type B is struct a: Box<A> end` passes layout-cycle checking while an
   unbroken direct cycle still fails exactly as it does today.
6. Tests: parser tests for `Box<T>`/`box(...)` in every permitted and
   rejected position (mirror `Ref<T>`'s existing parser test set); checker
   tests for storable/pointee validation, reserved-word rejection, bridge
   rejection, layout-cycle acceptance across a box edge, layout-cycle
   rejection for an unbroken direct cycle, nested `Box<Box<T>>`, and
   zero-sized pointee.
7. Verify: `cargo fmt --all`, `cargo check --workspace --all-targets`,
   `cargo test -p snacc-compiler`. Do not touch lowering, ABI, or runtime.

## Task B: places, aliases, and ownership analysis (RFC phase 3)

This is the novel-infrastructure task; budget it accordingly and consider
splitting it further at dispatch time if it looks too large for one agent
context (e.g. "control-flow availability tracking" vs. "consuming-context
classification and diagnostics" as two sub-passes) — this plan does not force
a single-shot dispatch here the way it does for Task A/C.

1. Give every checked use of a place an explicit copy/borrow/mutate/consume
   mode (spec 12.1 phase 3 step 1). Consuming contexts: initialization,
   assignment's right operand, a by-value argument, a function/method
   result, and an aggregate constructor argument (spec 6.1).
2. Track "available or moved" per root through sequential statements,
   `if`/`elseif` branch merges (available only if available on every
   reachable predecessor), and `while` loop fixed points (reject a move when
   another iteration or the loop exit may use the value without definite
   reinitialization) — spec 6.2. This is a new per-function dataflow pass;
   model it after the existing branch/exhaustiveness analysis shape
   (`analyze_chain`) for how this checker already threads per-branch state,
   but note move-availability is a genuinely different lattice (available/
   moved, not exhaustive/inexhaustive) and needs its own pass, not a bolt-on
   to `analyze_chain`.
3. Reject: use of a moved value; a move whose source overlaps its
   destination (spec 6.3, including `value = value`); a move out of a field,
   union payload projection, automatic box dereference, or other subplace
   (spec 6.4) — reading, borrowing, or mutating a field remains fine, only
   *moving out of* one is rejected; escape of a union-test payload alias
   beyond its branch (spec 7.3).
4. Represent union-test bindings as branch-scoped place aliases (extending
   the existing `Place`/binding machinery per spec 7.3), retaining an rvalue
   temporary through its branch. Canonicalize the alias to the tested place
   for overlap/move checking.
5. Extend the existing call-argument overlap analysis (Specification 011's
   `Ref<T>` overlap checking) through box dereferences and aliases (spec
   7.2's last paragraph): a borrowed allocation cannot be moved, destroyed,
   or independently mutably borrowed during the call.
6. Automatic dereference through field access and method calls (spec 4.3):
   never clones/moves/allocates/frees; just extends place-path resolution
   through a `Box<T>` field the way it already extends through struct
   fields.
7. `Ref<T>` compatibility (spec 7.2): a `Box<T>` argument place lends its
   pointee to a `Ref<T>` parameter automatically; a `Box<T>` place may
   instead bind to `Ref<Box<T>>` when that is the declared parameter type.
   The expected parameter type disambiguates — no new inference.
8. Tests: transitive ownership through nested structs/unions, branch and
   loop availability (positive and negative), reinitialization after a
   moved `let mut`, mutation through mutable vs. immutable roots, pointee
   borrowing via `Ref<T>`, union traversal without consuming the payload,
   prohibited field/subplace moves, overlapping source/destination
   rejection, and alias-escape rejection.
9. Verify: `cargo fmt --all`, `cargo check --workspace --all-targets`,
   `cargo test -p snacc-compiler`. Still no lowering/ABI/runtime changes.

## Task C: checked cleanup plan, runtime, LLVM lowering, ABI, and conformance (RFC phases 4-6)

1. Compute which concrete types require destruction (any transitively-owned
   `Box`). Generate an internal drop plan per struct (fields in declaration
   order), per union (active payload only), attached to checked scopes and
   exits; transfer/disarm the obligation on a move; evaluate assignment
   sources before destroying old destinations (spec 8.1).
2. Add runtime allocate/deallocate operations to `snacc-runtime` with
   explicit size/alignment and a new fatal-error path for allocation failure
   (spec 8.2) — this is new runtime surface, not a variant of an existing
   function; give it its own minimal, well-documented `unsafe` boundary per
   AGENTS.md's unsafe-Rust convention.
3. Lower `Box<T>` to a non-null target pointer; lower `box(expr)` with
   single-operand evaluation and one registered cleanup obligation; lower
   automatic dereference projections; lower moves as ownership transfer
   (suppressing source cleanup, not zeroing/asserting on it at runtime
   unless that's the simplest correct implementation); emit the checked
   cleanup plan on assignment and every scope exit.
4. Advance `snacc_compiler::ABI_VERSION`/`snacc_runtime::ABI_VERSION` by one
   from whatever RFC 018 lands it at, update
   `crates/snacc-runtime/tests/abi.rs`, and add the matching
   `apps/cargo-snacc/tests/cargo_hosted.rs` cache-invalidation test following
   the existing per-bump precedent. Reject box-containing bridge signatures
   in `apps/cargo-snacc/src/main.rs` (already partly done in Task A for the
   type-level rejection; confirm the bridge-assertion renderer path is
   covered too).
5. Update `LANGUAGE.md` and `GRAMMAR.ebnf` (byte-identical grammar content)
   for `Box<T>`/`box(...)`, ownership/move semantics, and the diagnostics in
   spec section 11.
6. Conformance: lists, binary trees, and a mutually recursive box-linked pair
   (construction, traversal, mutation, passing, returning, destruction);
   negative tests for every diagnostic in spec section 11; a runtime test
   instrumented to prove exactly one deallocation per allocation and no
   double-free after a branch-dependent move or replacement (spec 12.6 item
   3 — this may need a small counting/instrumentation hook in
   `snacc-runtime`'s test-only surface, mirroring how existing ABI tests
   already probe runtime symbols directly).
7. Full verification: `cargo fmt --all`, `cargo check --workspace
   --all-targets`, `cargo test --workspace`. All green.

## Explicit non-goals (spec section 15 / "Deferred work")

Consuming decomposition out of fields, replace-and-return/take operations,
deep cloning, structural equality/formatting for boxes, constant-stack
destruction guarantees, shared/weak ownership, arena handles, garbage
collection, opaque Rust bridge handles, recoverable allocation failure,
unsafe raw pointers, and any integration with user-defined generics. Do not
let any of these leak in as "while I'm in here" additions.
