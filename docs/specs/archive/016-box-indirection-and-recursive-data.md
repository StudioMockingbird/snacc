# RFC 016: Box Indirection and Recursive Data Structures

Status: Closed

Document kind: Feature specification (Rust-style RFC)

## 1. Proposal state

This RFC is implemented. It adds explicit, uniquely owned heap
indirection through `Box<T>`. The first version supports finite recursive
layouts, construction, traversal, mutation through mutable roots, whole-value
ownership transfer, and deterministic destruction. It does not add shared
ownership, raw pointers, user-visible lifetimes, ownership cycles, cloning, or
Rust bridge support.

`LANGUAGE.md` remains authoritative; the implemented contract is incorporated
there.

Inline optional links such as `Box<Node> | Nil` use the structural sum types
defined by [Specification 018](018-inline-sum-types.md), which must be
implemented before those forms. `Box<T>` itself does not depend on general
generic programming.

This RFC contains no open design questions. Section 12 fixes the implementation
order and phase boundaries.

## 2. Summary

`Box<T>` owns one heap allocation containing one `T`. Its representation has a
fixed size even when `T` is recursive:

~~~snacc
type IntList is union
    | Empty
    | Cons is struct
        head: Int64,
        tail: Box<IntList>,
      end
end
~~~

Allocation is explicit:

~~~snacc
let list: Box<IntList> = box(
    IntList.Cons(
        head: 1,
        tail: box(IntList.Empty())
    )
)
~~~

A box is non-null and has exactly one owner. Assigning, passing, returning, or
placing it in another value transfers ownership. The source cannot subsequently
be used. Field access and method dispatch automatically traverse boxes, while
allocation and ownership transfer remain visible.

## 3. Motivation

A direct recursive field has infinite size and remains invalid:

~~~snacc
type Node is struct
    next: Node,
end
~~~

Indirection breaks the by-value layout cycle:

~~~snacc
type IntLink is union
    | Empty
    | Item is struct
        value: Int64,
        next: Box<IntLink>,
      end
end
~~~

Type and ownership checking must finish before LLVM lowering. Safe programs
cannot observe addresses, double-free allocations, use moved values, or
construct ownership cycles.

## 4. Syntax

### 4.1 Box type

`Box<T>` is a closed built-in parameterized type form with exactly one argument,
like `Ref<T>`. It does not depend on or enable user-defined generic declarations,
generic calls, type inference, or monomorphization. `Box` is reserved and cannot
be declared as a user type, callable, method, parameter, or local name.

`T` must be a fully resolved, storable value type. A no-result type and a
call-scoped `Ref<T>` are not storable and cannot be boxed. `Box<Box<T>>` is
valid. Nominal recursive types may refer to themselves through box edges.

### 4.2 Allocation

The allocation expression is:

~~~snacc
box(expression)
~~~

`box` is a reserved built-in expression form, not a callable function. Its
operand is evaluated exactly once. Its result is `Box<T>`, where `T` is the
checked operand type.

`Box(value)` is not allocation syntax. There is no implicit conversion from
`T` to `Box<T>` or from `Box<T>` to `T`.

### 4.3 Access

Field access and method calls automatically dereference as many box layers as
member resolution requires:

~~~snacc
type Point is struct
    x: Int64,
end

method Point.value(): Int64 do
    self.x
end

let point: Box<Point> = box(Point(x: 10))
print(point.x)
print(point.value())
~~~

Automatic access never clones, moves, allocates, frees, or changes the static
type of the box expression. The first version has no general dereference
operator and no operation that consumes a box to extract a bare `T`.

## 5. Type and layout rules

### 5.1 Finite layout

`Box<T>` has pointer-sized, pointer-aligned storage independent of `T`. The
finite-layout graph treats a box occurrence as an indirection edge and does not
traverse through it.

These definitions remain invalid:

~~~snacc
type A is struct
    b: B,
end

type B is struct
    a: A,
end
~~~

These definitions have finite layouts:

~~~snacc
type A is struct
    b: Box<B>,
end

type B is struct
    a: Box<A>,
end
~~~

Finite layout does not imply constructibility. A recursive group without a
terminating constructor may be well-formed yet have no finite constructible
value.

### 5.2 Nullability

`Box<T>` itself is never null. Absence may be represented either inside a named
union or by an inline sum outside the box.

A named recursive list can put its empty alternative inside the pointee:

~~~snacc
type IntLink is union
    | Empty
    | Item is struct
        value: Int64,
        next: Box<IntLink>,
      end
end

let end: Box<IntLink> = box(IntLink.Empty())
~~~

`nil` does not inhabit `Box<T>`. With Specification 018, an inline optional
link is valid:

~~~snacc
type Node is struct
    value: Int64,
    next: Box<Node> | Nil,
end

let end: Box<Node> | Nil = nil
~~~

Here `nil` inhabits the surrounding sum, never `Box<Node>`. Dereferencing is
permitted only after an `is Box<Node>(node)` test binds the box member.

### 5.3 Move-only classification

`Box<T>` is move-only regardless of whether `T` is copyable. Move-only status
is structural and transitive:

- a struct is move-only when any field is move-only;
- a union is move-only when any member payload is move-only;
- existing values otherwise remain copyable unless another specification says
  differently.

The checked representation records this property after type resolution.
Lowering does not infer ownership from LLVM types.

## 6. Ownership and moves

### 6.1 Whole-value transfer

Using a move-only place in a consuming context transfers the complete value.
Consuming contexts are initialization, assignment's right operand, a by-value
argument, a function result, and an aggregate constructor argument.

~~~snacc
let first: Box<IntLink> = box(IntLink.Empty())
let second: Box<IntLink> = first
let third: Box<IntLink> = first
~~~

The final statement is rejected because `first` was moved. A transfer copies
the pointer representation and its cleanup obligation, not the pointee.
Values transitively containing a box transfer as a whole under the same rule.

### 6.2 Control flow

Every move-only root is available or moved at each program point. A use
requires availability. At a merge it is available only when available on every
reachable predecessor. Loops are checked to a fixed point; a move is rejected
when another iteration or the loop exit may use the value without definite
reinitialization.

Diagnostics identify the consuming operation and the invalid later use.

### 6.3 Assignment

Assignment to an available move-only destination:

1. evaluates the right operand completely;
2. transfers its result to temporary ownership;
3. destroys the old destination;
4. installs the new value.

A move whose source overlaps its destination is rejected, including
`value = value` and transfers between a value and one of its projections.
Assignment to a moved mutable local reinitializes it and restores availability.

### 6.4 Subplace moves

The first version rejects moving a move-only value out of a field, union payload
projection, automatic box dereference, or other subplace. Such a move would
leave a partially initialized aggregate.

This does not prevent reading copyable fields, borrowing or mutating fields,
replacing an entire owning root, or moving a complete aggregate. Consuming
decomposition and replace-and-return operations require a later specification.
The initial feature therefore supports construction, traversal, in-place
updates, and whole-structure transfer, but not every efficient consuming
collection algorithm.

### 6.5 Cloning

This RFC provides no box clone. A later capability may define deep cloning.
Copying only the pointer is never cloning.

## 7. Borrowing, mutation, and decomposition

### 7.1 Root mutability

Root mutability extends through automatic box dereference:

~~~snacc
type Node is struct
    value: Int64,
end

let node: Box<Node> = box(Node(value: 10))
node.value = 20
~~~

This is rejected because `node` is immutable.

~~~snacc
let mut node: Box<Node> = box(Node(value: 10))
node.value = 20
~~~

This is valid. A mutable root permits replacement of the box and mutation of
its pointee. An immutable root permits neither. A normal by-value parameter
remains an immutable root even when it owns a box.

### 7.2 Call-scoped references

`Ref<T>` remains a call-boundary borrowing mode, not recursive storage. When a
call expects `Ref<T>`, an argument place of type `Box<T>` automatically lends
its pointee:

~~~snacc
fun increment(node: Ref<Node>) do
    node.value = node.value + 1
end

let mut node: Box<Node> = box(Node(value: 10))
increment(node)
~~~

The call does not consume `node`; the borrow ends when the call returns.
Mutation requires a mutable originating root.

An argument of type `Box<T>` may instead be borrowed as `Ref<Box<T>>` when that
is the declared parameter type. The expected parameter type decides whether the
box or its pointee is borrowed. Automatic dereference never turns a by-value
argument into a borrow.

Existing overlap rules apply after dereference. A borrowed allocation cannot be
moved, destroyed, or independently mutably borrowed during the call.

### 7.3 Union type tests

A union type test never copies or consumes its payload:

~~~snacc
type Tree is union
    | Empty
    | Branch is struct
        value: Int64,
        left: Box<Tree>,
        right: Box<Tree>,
      end
end

let tree: Box<Tree> = box(
    Tree.Branch(
        value: 1,
        left: box(Tree.Empty()),
        right: box(Tree.Empty())
    )
)

if tree is Tree.Branch(branch) then
    print(branch.value)
end

print(sum(tree))
~~~

The member binding is a branch-scoped place alias to the active payload, not a
copied value or first-class `Ref<T>`. It:

- exists only inside that branch;
- cannot escape through a result, box, aggregate, or call result;
- permits reads and call-scoped borrowing;
- permits mutation only when the tested place has a mutable root;
- cannot move a move-only payload or field;
- canonicalizes to the tested place for overlap and move checking.

For a tested rvalue, the compiler retains the temporary through the branch and
makes the alias read-only. The temporary is destroyed after the conditional.

This supports recursive traversal without general local references:

~~~snacc
fun sum(tree: Ref<Tree>): Int64 do
    if tree is Tree.Empty then
        0
    elseif tree is Tree.Branch(branch) then
        branch.value + sum(branch.left) + sum(branch.right)
    end
end
~~~

## 8. Allocation and destruction

### 8.1 Deterministic destruction

An owner destroys its allocation exactly once when overwritten or when its
scope ends. Moving transfers the cleanup obligation and disarms the source.

Generated drop operations follow these rules:

- `Box<T>` drops its `T` and releases its allocation;
- a struct drops move-only fields in source declaration order;
- a union drops only its active payload;
- a moved value performs no cleanup;
- locals drop in reverse successful-initialization order;
- temporaries drop at the end of their full-expression or extended branch
  lifetime.

Cleanup occurs on every supported normal edge leaving an owning scope. The
checked program carries explicit cleanup obligations; LLVM lowering does not
rediscover them.

The first implementation may recursively destroy recursive structures. It does
not guarantee constant-stack destruction of arbitrarily deep trees or lists.

### 8.2 Allocation failure

Allocation failure terminates the process through a new runtime fatal-error
path added with this feature. It does not return `Nil`, a null box, or a
recoverable result. The allocator accepts size and alignment and either returns
valid, aligned, non-null storage or does not return. Adding this runtime path
and its symbols is part of the ABI change in phase 5.

Boxing a zero-sized value is valid. The runtime may allocate a minimum-sized
block or use another non-null representation. Programs cannot observe its
address.

### 8.3 Equality and printing

Direct equality involving a box and direct printing of a box are unsupported
initially. Pointer identity is not language-visible. Because safe unique
ownership cannot form cycles, later structural equality or formatting needs no
cycle rule, though it must define recursion-depth behavior.

## 9. Cycles, sharing, and safety

Safe operations in this RFC cannot construct an ownership cycle. A cycle would
require duplicated ownership, a raw address, or placing an allocation inside
itself; none is expressible. Recursive type definitions remain valid because
type recursion is not a runtime ownership cycle.

Multiple owners, shared directed acyclic graphs, parent pointers, doubly linked
lists, and general graphs require a separate facility such as `Shared<T>`,
`Weak<T>`, arena handles, or garbage collection. This RFC does not choose one.
Raw pointers are not a safe substitute.

## 10. Rust bridge and ABI

`Box<T>` and every type transitively containing one are rejected in `extern
rust` parameters and results. A Snacc box follows Snacc's allocation,
ownership, move, and destruction contract; it is neither Rust `Box<T>` nor a C
ABI scalar merely because lowering uses a pointer.

An opaque bridge handle requires a later specification. No representation or
ownership compatibility with Rust's `Box<T>` is promised.

Internal Snacc calls may pass and return boxes through the compiler's private
ABI. Implementation advances the applicable compiler/runtime ABI version and
rejects incompatible artifacts. The numeric successor is selected from the
last implemented ABI specification when this RFC lands rather than reserved
ahead of its dependencies.

## 11. Diagnostics

The frontend diagnoses at least:

- the wrong number of `Box` arguments;
- a non-storable or unresolved pointee;
- an infinite by-value cycle;
- use of a moved value;
- path-dependent availability after control-flow merging;
- a prohibited move from a subplace;
- overlapping move source and destination;
- mutation through an immutable root;
- a move or conflicting borrow during a call-scoped borrow;
- escape of a union payload alias;
- box equality or direct box printing;
- a box-containing Rust bridge signature.

Ownership failures are checker errors, never LLVM lowering failures.

## 12. Detailed implementation plan

### Phase 1: syntax and resolved types

1. Reserve `Box` in type positions and `box` in expression position.
2. Parse `Box<T>` directly as a closed built-in type form using the already
   established angle-bracket tokenization for `Ref<T>`. Do not wait for or
   enable general generic syntax.
3. Parse `box(expression)` as a distinct allocation expression.
4. Add resolved and checked box types. Keep compiler type descriptors copyable;
   move-only is a property of represented Snacc values.
5. Reject invalid pointees and box-containing Rust bridge signatures.

### Phase 2: layout and properties

1. Separate the by-value layout graph from the complete semantic dependency
   graph.
2. Terminate layout recursion at box edges while retaining dependencies for
   resolution and destruction.
3. Compute size, alignment, storable status, and transitive move-only status for
   each resolved type.
4. Diagnose invalid by-value cycles with explicit visitation states.
5. Test direct and indirect cycles, box-broken cycles, mutual recursion, nested
   boxes, unions, and zero-sized pointees.

### Phase 3: places, aliases, and ownership

1. Give checked uses an explicit copy, borrow, mutate, or consume mode.
2. Represent places canonically as a root plus field, payload, and automatic
   dereference projections.
3. Track available or moved roots through sequential code, branch merges, and
   loop fixed points.
4. Mark consuming contexts and reject later uses, subplace moves, and
   overlapping source/destination moves.
5. Represent union bindings as scoped aliases, retain rvalue temporaries through
   their branch, and reject alias escape.
6. Extend call-overlap analysis through boxes and aliases.
7. Test transitive ownership, branches, loops, reinitialization, mutability,
   pointee borrowing, union traversal, and prohibited field moves.

### Phase 4: checked cleanup plan

1. Compute which concrete types require destruction.
2. Generate internal drop operations for structs, active union payloads, and
   nested boxes.
3. Attach initialization state and cleanup obligations to checked scopes and
   exits.
4. Transfer and disarm obligations on moves.
5. Evaluate assignment sources before destroying destinations.
6. Assert that each owned value has exactly one cleanup owner on every reachable
   path.

### Phase 5: runtime and LLVM

1. Add internal allocation and deallocation operations with explicit size,
   alignment, and fatal allocation failure.
2. Lower boxes to non-null target pointers in the private Snacc ABI.
3. Lower allocation with single operand evaluation and one registered cleanup
   obligation.
4. Lower checked automatic dereference projections for fields and methods.
5. Lower moves as ownership transfers and suppress source cleanup.
6. Emit the checked cleanup plan on assignment and every scope exit.
7. Advance the compiler/runtime ABI version and its compatibility diagnostic.

### Phase 6: conformance and integration

1. Test construction, traversal, mutation, passing, returning, and destruction
   for lists, binary trees, and mutually recursive types.
2. Add negative tests for every diagnostic in section 11.
3. Instrument runtime tests to prove one deallocation per allocation and no
   double-free after branch-dependent moves or replacement.
4. Test recursive destruction at a practical depth without claiming unbounded
   stack safety.
5. Verify generated programs outside the repository without Cargo, repository
   paths, network access, or compiler sources at runtime.
6. Update `LANGUAGE.md`, its formal grammar, `GRAMMAR.ebnf`, compiler comments,
   examples, and diagnostics in the implementation change. Keep both grammar
   copies identical.
7. Run formatting, workspace checks, and the complete workspace test suite.

## 13. Acceptance criteria

Implementation is complete only when:

1. each accepted recursive layout cycle crosses a box edge;
2. unbroken by-value cycles fail before lowering;
3. `box(expression)` allocates one non-null, uniquely owned value;
4. automatic access supports ordinary tree and list traversal;
5. boxes and aggregates containing boxes move rather than copy;
6. use-after-move and subplace moves are rejected;
7. root `mut` consistently controls box replacement and pointee mutation;
8. boxes lend pointees to compatible `Ref<T>` parameters without consumption;
9. union bindings inspect recursive values without copying or consuming them;
10. each allocation is destroyed exactly once on supported normal exits;
11. safe code cannot create shared ownership or an ownership cycle;
12. box-containing types cannot cross the Rust bridge;
13. `LANGUAGE.md`, both grammar copies, and implementation agree;
14. all positive, negative, runtime, and workspace tests pass.

## 14. Rejected alternatives

### Wait for general generics

`Box<T>` needs one checked pointee type but no generic declaration, call,
inference, specialization, or monomorphization machinery. Treating it like the
already implemented `Ref<T>` form keeps recursive storage independent of the
unsettled generic-programming design.

### Implicitly box recursive fields

Implicit boxing would make allocation, ownership, destruction, and layout cost
depend on cycle detection rather than visible source syntax. An explicit
`Box<T>` keeps the representation boundary obvious.

### Use `Box(value)` for allocation

That spelling makes allocation resemble an ordinary nominal constructor.
`box(value)` is a reserved operation and visibly distinguishes allocation from
constructing the pointee value.

### Copy a box when its pointee is copyable

Copying the pointer would create two owners, while implicitly copying an
arbitrarily large pointee would hide allocation and traversal. Every box is
therefore move-only regardless of `T`; deep cloning is separate and explicit if
later added.

### Make boxes nullable

A hidden null state would duplicate the language's explicit union model and
make every dereference require an implicit failure rule. Absence remains a
named or inline sum alternative: `Box<T> | Nil` is nullable as a sum, while its
`Box<T>` member is always non-null.

### Start with shared ownership or garbage collection

Shared ownership introduces reference-count cost, cycle handling, and shared
mutation rules. Garbage collection adds a global runtime policy and
nondeterministic reclamation. Unique ownership is sufficient for the first
tree and linked-list use cases.

## 15. Deferred work

- consuming decomposition and moves out of fields;
- replace-and-return or take operations;
- deep cloning;
- structural equality and formatting;
- constant-stack destruction guarantees;
- shared and weak ownership;
- arena handles or garbage collection;
- opaque Rust bridge handles;
- recoverable allocation failure;
- unsafe raw pointers;
- integration with user-defined generic types after generic programming is
  specified.

## 16. Findings from Specifications 022 and 023

Specifications 022 and 023 depend on this RFC's ownership and cleanup model and
surfaced six items that belong here rather than in either of them.

None of them reopens a rule above, and section 1's readiness claim stands: this
RFC can be implemented exactly as written. Items marked **gap** need text here
before the *dependent* specification can land -- they are forward obligations
that activate when concurrency or I/O is accepted, not holes in the design
below. 16.1 and 16.3 constrain this RFC's successor rather than this RFC.

**16.1 Shared ownership must arrive with its concurrency rule. (gap)**
Section 9 leaves `Shared<T>`, `Weak<T>`, arena handles, and garbage collection
unchosen. Specification 022 section 7.3 shows that its entire data-race
argument -- no `Send` marker, no `Sync` marker, no escape analysis -- holds
*only* while nothing in the language shares ownership. A future shared-ownership
facility added without a concurrency rule in the same change silently
invalidates that argument and makes concurrent Snacc programs unsound. Whatever
this RFC's successor chooses, the concurrency rule is part of the same
specification, not a follow-up.

**16.2 Cleanup cannot report a failure.** Section 8.1's deterministic
destruction runs on scope exit with nowhere to put an error. Specification 023
section 9.1 hits this directly: a buffered file's close error is discarded
unless the program calls `flush` first, and nothing makes it. Every language
with RAII has this wart. Specification 025 deliberately accepts only
no-result deferred calls, so a fallible flush or close remains an explicit
ordinary call before exit rather than silently losing or replacing an error.

**16.3 A consuming receiver method is a distinct need. (gap)** Section 6.4
defers moving a move-only value *out of* a subplace. Specification 023 wants
something different: a method that consumes its whole receiver, so that
`file.close()` can report an error and make the handle unusable afterwards.
That is a whole-value transfer of the receiver, not a subplace move, and the
current consuming-context list in section 6.1 does not include a method
receiver. Decide whether it should.

**16.4 Who destroys a value moved into a concurrent task? (gap)**
Specification 022 section 10.2 passes spawn arguments in a payload struct and
runs them through a compiler-generated thunk. A move-only value passed by value
into a spawn is therefore owned by the *task*, and the thunk -- not the
spawning frame -- must run its cleanup plan. Specification 023 section 11 does
exactly this with a `TcpStream`. This RFC's checked cleanup plan must cover a
generated thunk as a scope, which no section currently says.

**16.5 Adding a move-only field is a breaking change.** Section 5.3 propagates
move-only status structurally, so adding one `Box<T>` or one file handle to an
existing copyable struct silently makes every existing copy of that struct a
move, and every second use an error. That is correct behavior and a real
migration hazard worth stating in the specification rather than discovering in
a diagnostic.

**16.6 The move checker needs multi-span diagnostics.** Section 11's messages
must point at the consuming operation *and* the invalid later use.
`Diagnostic` in the compiler currently carries a single optional span.
Specification 022 section 14.23 needs the same facility for its borrow rules,
so this is shared infrastructure worth building once rather than a cost either
specification should carry alone.

## 17. References

- [`LANGUAGE.md`](../../LANGUAGE.md)
- [RFC 015: Function and Method Recursion](015-function-recursion.md)
- [Specification 018: Inline Sum Types](018-inline-sum-types.md)
