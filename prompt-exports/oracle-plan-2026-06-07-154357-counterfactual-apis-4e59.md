## Final Prompt
<taskname="Counterfactual APIs"/>
<task>
Create an implementation plan, not code, for adding counterfactual simulator support APIs to the `l4-book` crate. The plan must have up to 5 concrete work items with key files and done criteria. Cover: queue position queries, deterministic taker matching/fill deltas, a `submit_limit_order` helper with policies, enriched operation reason/cause metadata, amend priority semantics, synthetic ID guidance/namespace if feasible, snapshot preserve/replace policy, tolerant replay outcomes, and public docs/tests updates.
</task>

<architecture>
- `src/book.rs` is the core venue-agnostic `OrderBook`: slab arena of `OrderNode`s, `HashMap<OrderId, slot>`, `BTreeMap<Price, Level>` per side, `wallet_index`, FIFO linked lists per price level, and public mutations/queries.
- `src/types.rs` owns public domain aliases/types (`OrderId`, `Price`, `Qty`, `Ts`, `WalletId`, `Order`, `Side`, `BookError`). New result structs/enums/errors likely belong here or in `book.rs` and must be re-exported from `src/lib.rs`.
- `src/level.rs` contains crate-private `Level` and `OrderNode` used by `OrderBook`; queue/matching logic must respect this linked-list structure.
- `src/dwellir.rs` is feature-gated adapter/replay decoding. It emits `BookOp::{Add, Remove, UpdateSize, AmendSize}` plus `UpdateBatch`, `Capture`, and stats. Existing replay callers manually match `BookOp` to `OrderBook` mutations.
- `docs/api.md` and `README.md` document the public API. `docs/l4-protocol.md` is the Dwellir/Hyperliquid feed reference and explains order statuses/diff causes.
- `tests/book_basic.rs` contains core behavior tests including FIFO priority, size update/amend semantics, snapshot replacement, top-of-book snapshots, and slippage estimation. Dwellir unit tests live inside `src/dwellir.rs`.
</architecture>

<selected_context>
Cargo.toml: crate features, optional `dwellir`, examples/bench declarations, dependency limits.
README.md: public overview, core API snippets, design choices, notes that the current book has “No matcher”.
docs/api.md: current API reference for mutations, queries, slippage, and Dwellir adapter loop.
docs/l4-protocol.md: Dwellir L4 protocol reference, status values, `book_diffs` causes, and lifecycle patterns useful for reason/cause metadata.
src/book.rs: full `OrderBook` implementation; current methods include `add`, `remove`, `update_size`, `amend_size`, `apply_snapshot`, `depth`, `orders_at`, `orders_by_wallet`, `estimate_slippage`, top-N snapshots, and invariant checks.
src/types.rs: full public types/errors; current `BookError` has duplicate/unknown/zero/non-decreasing/wallet parse variants.
src/level.rs: full private level/node storage model used for FIFO and O(1) unlinking.
src/lib.rs: public re-exports; any new public structs/enums must be exported here.
src/dwellir.rs: full adapter implementation; current `BookOp` variants are unreasoned, `decode_updates` collapses complete fill zero-size updates and duplicate removes, `load_capture` returns `Vec<Vec<BookOp>>` plus counts.
tests/book_basic.rs: full test suite for current core behavior and slippage semantics; extend for simulator APIs and deterministic fill/delta outputs.
examples/replay_capture.rs: applies capture ops manually and counts errors; relevant for tolerant replay outcomes.
examples/dwellir_live_sp500.rs: live apply loop, slippage reporting; will need adapting if `BookOp` or replay helpers change.
examples/dwellir_debug_log_sp500.rs: detailed raw/parsed op logging; useful when enriching op reason/cause metadata.
benches/replay.rs: benchmark replay path manually applying `BookOp`; important for preserving performance and updating helper APIs.
</selected_context>

<relationships>
- `OrderBook::add/remove/update_size/amend_size` mutate slab/index/levels/wallet_index and are the foundation for any simulator mutation helper.
- `orders_at(side, price)` yields FIFO `&Order`s from the private linked list and is the natural basis for queue position and taker matching walks.
- `estimate_slippage(taker_side, qty, limit_price)` already walks aggregate opposite depth without mutation; deterministic taker fill APIs should likely share semantics but return per-order fills/deltas and optionally mutate.
- `BookOp` in `src/dwellir.rs` maps feed diffs to `OrderBook` methods; all examples, bench, and docs currently match on its four variants directly.
- Dwellir `order_statuses` and `raw_book_diff` shapes are parsed in `decode_updates`; reason/cause metadata can be derived partly from diff type and status strings, but trade-level execution details are not in L4 per `docs/l4-protocol.md`.
- `apply_snapshot` currently always clears and replaces state; snapshot preserve policy would need new API or options while maintaining current behavior.
- `amend_size` currently preserves queue position even on size increase; comments note adapters can drop-and-readd when venue policy is “amend-up loses priority”.
</relationships>

<plan_requirements>
Produce up to 5 work items. For each item include: purpose, key files, API surfaces/symbols to add or alter, tests/docs to add, and done criteria. Keep the plan concrete enough for a follow-up implementation model, but do not implement. Address compatibility impacts for existing examples/bench and whether to preserve old APIs alongside new helpers.
</plan_requirements>

<ambiguities>
- The requested “counterfactual simulator support APIs” does not specify exact public names or whether helpers should mutate the real book, return a separate delta, or support both. Plan should explicitly decide or present minimal API alternatives.
- “Synthetic ID guidance/namespace” may be documentation-only or a typed API; `OrderId` is currently a plain `u64`, so namespace enforcement is not possible without a compatibility decision.
- Dwellir L4 lacks individual trade executions/counterparties; reason/cause enrichment can identify status/diff cause but cannot reconstruct full trades without another feed.
</ambiguities>

## Selection
- Files: 14 total (14 full)
- Total tokens: 38083 (Auto view)
- Token breakdown: full 38083

### Files
### Selected Files
/Users/janelafina/code/janelafina/l4-book/
├── benches/
│   └── replay.rs — 1,325 tokens (full)
├── docs/
│   ├── api.md — 1,734 tokens (full)
│   └── l4-protocol.md — 9,372 tokens (full)
├── examples/
│   ├── dwellir_debug_log_sp500.rs — 4,775 tokens (full)
│   ├── dwellir_live_sp500.rs — 2,561 tokens (full)
│   └── replay_capture.rs — 854 tokens (full)
├── src/
│   ├── book.rs — 5,313 tokens (full)
│   ├── dwellir.rs — 5,938 tokens (full)
│   ├── level.rs — 212 tokens (full)
│   ├── lib.rs — 191 tokens (full)
│   └── types.rs — 764 tokens (full)
├── tests/
│   └── book_basic.rs — 2,806 tokens (full)
├── Cargo.toml — 282 tokens (full)
└── README.md — 1,956 tokens (full)


---

## Generated Plan

## Chat Send ✅
- **Chat**: `counterfactual-apis-5EC94B` | **Mode**: plan

### Response
## 1. Summary

Add counterfactual simulator support as a targeted extension to the existing `OrderBook` rather than a broad refactor. The current slab + per-level FIFO linked-list model already supports queue-position inspection and deterministic maker-order walking. The plan adds: queue position APIs, deterministic taker fill simulation/application, limit-order submission policies, amend priority policies, core book operation metadata, tolerant replay helpers, synthetic ID guidance, snapshot preservation options, and documentation/tests. Existing mutation APIs remain available and preserve current behavior.

---

## 2. Current-state analysis

`OrderBook` in `src/book.rs` owns all mutable book state:

- `slab: Vec<Option<OrderNode>>` stores resting orders.
- `order_index: HashMap<OrderId, usize>` maps order ID to slab slot.
- `bids` / `asks: BTreeMap<Price, Level>` store price levels.
- Each `Level` points to FIFO order nodes through `head` / `tail`.
- `wallet_index` maps each `WalletId` to live `OrderId`s.

Current mutation flow:

- `add(Order)` allocates a slab slot, inserts indexes, appends to the level tail.
- `remove(OrderId)` unlinks from the level, removes indexes, recycles the slot.
- `update_size(OrderId, Qty)` only allows shrinking; `0` removes.
- `amend_size(OrderId, Qty)` allows growth/shrink and preserves FIFO position.
- `apply_snapshot(...)` always clears all state before adding snapshot orders.

Current query flow:

- `orders_at(side, price)` walks the private linked list in FIFO order.
- `depth(side)` walks aggregate price levels only.
- `estimate_slippage(...)` walks aggregate opposite-side depth and does not expose per-order fills.

Current Dwellir flow:

- `src/dwellir.rs` has its own `BookOp` enum.
- `decode_line(...)` returns `Decoded::{Skip, Snapshot(Vec<Order>), Updates(UpdateBatch)}`.
- `UpdateBatch` contains plain ops and feed-quirk counts.
- Examples and benches manually match `BookOp` to `OrderBook` mutations.
- Operation cause metadata is not retained, despite Dwellir messages containing diff kinds and order status strings.

Reusable extension points:

- `orders_at` and level linked lists are the correct basis for queue and matching.
- Existing `add/remove/update_size/amend_size` should remain the single mutation primitives.
- Existing `estimate_slippage` semantics should be mirrored by new deterministic fill APIs.
- Dwellir diff/status parsing should be extended, not duplicated.

Blocking gaps:

- No queue-position API.
- No per-order taker matching/fill deltas.
- No helper for applying `BookOp`.
- No operation reason/cause metadata.
- `amend_size` has only preserve-priority semantics.
- `apply_snapshot` cannot preserve simulator/synthetic orders.
- Replay loops cannot distinguish strict errors from tolerated feed inconsistencies.

---

## 3. Design

### Work item 1 — Queue position and amend priority APIs

**Purpose**

Expose queue position for live and hypothetical orders, and make amend priority behavior explicit while preserving the existing default.

**Key files**

- `src/book.rs`
- `src/types.rs`
- `src/lib.rs`
- `tests/book_basic.rs`
- `docs/api.md`
- `README.md`

**API surfaces**

Add public types, preferably in `src/book.rs` and re-export from `src/lib.rs`:

```rust
pub struct QueuePosition {
    pub id: Option<OrderId>,
    pub side: Side,
    pub price: Price,
    pub orders_ahead: u64,
    pub qty_ahead: Qty,
    pub orders_behind: u64,
    pub qty_behind: Qty,
    pub level_order_count: u64,
    pub level_total_qty: Qty,
}

pub enum AmendPriorityPolicy {
    Preserve,
    LosePriorityOnIncrease,
}
```

Add methods on `OrderBook`:

```rust
pub fn queue_position(&self, id: OrderId) -> Result<QueuePosition, BookError>;

pub fn queue_position_for_new_order(
    &self,
    side: Side,
    price: Price,
) -> QueuePosition;

pub fn amend_size_with_policy(
    &mut self,
    id: OrderId,
    new_qty: Qty,
    policy: AmendPriorityPolicy,
) -> Result<(), BookError>;
```

Existing `amend_size(id, new_qty)` remains and delegates to `amend_size_with_policy(..., Preserve)`.

**Behavior**

- `queue_position(id)`:
  - Looks up the slab slot through `order_index`.
  - Walks that order’s price-level linked list from `head`.
  - Counts orders and quantity before and after the target.
  - Returns `BookError::UnknownOrderId(id)` if absent.
- `queue_position_for_new_order(side, price)`:
  - Treats the hypothetical order as joining the tail.
  - `orders_ahead = level.order_count`.
  - `qty_ahead = level.total_qty`.
  - If level is absent, all counts are zero.
- `AmendPriorityPolicy::Preserve`:
  - Existing behavior.
- `AmendPriorityPolicy::LosePriorityOnIncrease`:
  - If `new_qty > current_qty`, update size and move the order to the tail of the same price level.
  - If `new_qty <= current_qty`, preserve position.
  - `new_qty == 0` still removes.

**Tests/docs**

Add tests for:

- Queue position for first, middle, last order in a level.
- Queue position for missing order.
- Hypothetical new-order queue position.
- Amend-up moves to level tail under `LosePriorityOnIncrease`.
- Amend-down preserves priority under `LosePriorityOnIncrease`.
- Existing `amend_size` still preserves priority.

**Done criteria**

- Existing tests pass unchanged.
- New queue-position tests prove FIFO counts and quantities.
- New amend policy tests prove tail movement only on size increase.
- Public docs describe zero-based queue position semantics via `orders_ahead`.

---

### Work item 2 — Deterministic taker matching and limit-order submission policies

**Purpose**

Add simulator APIs that deterministically walk maker orders by price-time priority, return per-order fill deltas, and optionally mutate the book.

**Key files**

- `src/book.rs`
- `src/types.rs`
- `src/lib.rs`
- `tests/book_basic.rs`
- `docs/api.md`
- `README.md`

**API surfaces**

Add public result structs:

```rust
pub struct FillDelta {
    pub maker_order_id: OrderId,
    pub maker_wallet: WalletId,
    pub maker_side: Side,
    pub price: Price,
    pub filled_qty: Qty,
    pub maker_qty_before: Qty,
    pub maker_qty_after: Qty,
    pub maker_removed: bool,
}

pub struct TakerMatch {
    pub taker_side: Side,
    pub requested_qty: Qty,
    pub filled_qty: Qty,
    pub unfilled_qty: Qty,
    pub filled_notional: u128,
    pub limit_price: Price,
    pub reference_price: Option<Price>,
    pub average_price: Option<f64>,
    pub fills: Vec<FillDelta>,
    pub limit_stopped: bool,
    pub exhausted_book: bool,
}
```

Add methods:

```rust
pub fn match_taker_order(
    &self,
    taker_side: Side,
    qty: Qty,
    limit_price: Price,
) -> TakerMatch;

pub fn apply_taker_order(
    &mut self,
    taker_side: Side,
    qty: Qty,
    limit_price: Price,
) -> Result<TakerMatch, BookError>;
```

Add limit submit policy:

```rust
pub enum LimitOrderPolicy {
    Gtc,
    Ioc,
    Fok,
    PostOnly,
}

pub enum SubmitRejectReason {
    PostOnlyWouldCross,
    FillOrKillWouldNotFill,
}

pub struct SubmitLimitOrderOutcome {
    pub policy: LimitOrderPolicy,
    pub rejected: Option<SubmitRejectReason>,
    pub taker_match: TakerMatch,
    pub rested_order: Option<Order>,
    pub operations: Vec<ReasonedBookOp>,
}
```

Add method:

```rust
pub fn submit_limit_order(
    &mut self,
    order: Order,
    policy: LimitOrderPolicy,
) -> Result<SubmitLimitOrderOutcome, BookError>;
```

`ReasonedBookOp` is introduced in work item 3; this method should depend on that type.

**Behavior**

`match_taker_order`:

- Does not mutate.
- For `Side::Bid`, walks asks ascending.
- For `Side::Ask`, walks bids descending.
- Within each price level, walks FIFO through the linked list.
- Stops when requested quantity is filled, book is exhausted, or next price violates limit.
- Fill deltas include maker before/after quantity and removal flag.
- Aggregate fields must match `estimate_slippage` for the same inputs.

`apply_taker_order`:

- Calls the same matching logic.
- Applies each delta:
  - `maker_qty_after == 0` → `remove(maker_order_id)`.
  - otherwise → `update_size(maker_order_id, maker_qty_after)`.
- Because matching and mutation happen on the same `&mut self`, mutation errors should only happen if invariants are broken; still return `Result`.

`submit_limit_order`:

- Rejects `order.qty == 0` with `BookError::ZeroQty`.
- Checks `order.id` is not already live before any mutation.
- `Gtc`:
  - Match as taker.
  - Rest remaining quantity as the same order ID if `unfilled_qty > 0`.
- `Ioc`:
  - Match as taker.
  - Cancel remainder; no resting order.
- `Fok`:
  - Simulate first.
  - If not fully fillable, return outcome with `rejected = Some(FillOrKillWouldNotFill)` and no mutation.
  - If fillable, apply fills.
- `PostOnly`:
  - If the order would cross the current opposite best, return `rejected = Some(PostOnlyWouldCross)` and no mutation.
  - Otherwise add the full order as resting.

**Tests/docs**

Add tests for:

- Non-mutating match returns FIFO per-order fills.
- Partial maker fill leaves maker live with updated size.
- Full maker fill removes order and updates wallet index.
- Limit stop versus exhausted book.
- `Gtc` match-and-rest behavior.
- `Ioc` cancels remainder.
- `Fok` rejects without mutation when insufficient liquidity.
- `PostOnly` rejects crossing and rests non-crossing.
- `match_taker_order` aggregate fields match existing `estimate_slippage`.

**Done criteria**

- Deterministic fill ordering is proven by tests with multiple orders at same price.
- No mutation occurs for pure matching or rejected submit policies.
- Existing `estimate_slippage` behavior remains unchanged.
- README no longer says “No matcher” without qualification; it should clarify that matching is simulator/counterfactual only.

---

### Work item 3 — Core operation metadata, tolerant replay, synthetic IDs, and snapshot policies

**Purpose**

Move venue-neutral book operations into the core crate, attach optional reason/cause metadata, add replay helpers with strict/tolerant outcomes, and support snapshot replacement while preserving simulator orders.

**Key files**

- `src/types.rs`
- `src/book.rs`
- `src/lib.rs`
- `src/dwellir.rs`
- `tests/book_basic.rs`
- `docs/api.md`
- `README.md`

**API surfaces**

Move/add `BookOp` to core `src/types.rs`:

```rust
pub enum BookOp {
    Add(Order),
    Remove(OrderId),
    UpdateSize { id: OrderId, new_qty: Qty },
    AmendSize { id: OrderId, new_qty: Qty },
}
```

In `src/dwellir.rs`, re-export the core type so this remains valid:

```rust
use l4_book::dwellir::BookOp;
```

Add reason/cause metadata:

```rust
pub enum OperationSource {
    Venue,
    Simulator,
    User,
    Unknown,
}

pub enum VenueDiffKind {
    New,
    Remove,
    Update,
    Modified,
    CompleteFillCollapsed,
}

pub enum SimulatorCause {
    TakerFill,
    LimitOrderRest,
    SnapshotPreserve,
}

pub enum OperationCause {
    Venue {
        source: &'static str,
        diff: VenueDiffKind,
        status: Option<String>,
    },
    Simulator(SimulatorCause),
    Snapshot,
    User,
    Unknown,
}

pub struct ReasonedBookOp {
    pub op: BookOp,
    pub cause: OperationCause,
}
```

Add strict apply helper:

```rust
pub fn apply_op(&mut self, op: BookOp) -> Result<(), BookError>;
```

Add replay policy/outcome:

```rust
pub struct ReplayPolicy {
    pub missing_order: MissingOrderPolicy,
    pub duplicate_add: DuplicateAddPolicy,
    pub non_decreasing_update: NonDecreasingUpdatePolicy,
}

pub enum MissingOrderPolicy {
    Error,
    Skip,
}

pub enum DuplicateAddPolicy {
    Error,
    Skip,
}

pub enum NonDecreasingUpdatePolicy {
    Error,
    TreatAsAmend,
}

pub enum ReplayApplyOutcome {
    Applied { op: BookOp },
    Skipped { op: BookOp, reason: ToleratedReplayReason },
    Coerced { original: BookOp, applied: BookOp, reason: ToleratedReplayReason },
    Error { op: BookOp, error: BookError },
}
```

Add constructors:

```rust
impl ReplayPolicy {
    pub const STRICT: ReplayPolicy = ...;
    pub const DWELLIR_TOLERANT: ReplayPolicy = ...;
}
```

Add method:

```rust
pub fn apply_op_with_policy(
    &mut self,
    op: BookOp,
    policy: ReplayPolicy,
) -> ReplayApplyOutcome;
```

Add synthetic ID helpers in `src/types.rs`:

```rust
pub const SYNTHETIC_ORDER_ID_FLAG: u64 = 1 << 63;

pub fn is_synthetic_order_id(id: OrderId) -> bool;

pub fn synthetic_order_id(local_id: u64) -> Option<OrderId>;
```

Guidance:

- Synthetic IDs use the high bit.
- `synthetic_order_id(local_id)` returns `None` if `local_id` already uses the high bit.
- This is a convention, not enforced by `OrderId`.
- Dwellir/Hyperliquid IDs observed in docs are far below this range, but adapters should validate before relying on the namespace.

Add snapshot policy:

```rust
pub enum SnapshotPolicy {
    Replace,
    PreserveSynthetic,
}

pub struct SnapshotOutcome {
    pub inserted: usize,
    pub preserved: usize,
}
```

Add method:

```rust
pub fn apply_snapshot_with_policy<I>(
    &mut self,
    orders: I,
    policy: SnapshotPolicy,
) -> Result<SnapshotOutcome, BookError>
where
    I: IntoIterator<Item = Order>;
```

Existing `apply_snapshot(...)` remains equivalent to `SnapshotPolicy::Replace`.

**Behavior**

Replay tolerance:

- Strict policy mirrors current manual matching behavior.
- `DWELLIR_TOLERANT`:
  - Missing remove/update/amend → skipped.
  - Duplicate add → skipped.
  - Non-decreasing update → coerced to `AmendSize`.
- Outcomes explicitly report applied/skipped/coerced/error.

Snapshot policy:

- `Replace` clears all state and applies snapshot, preserving current behavior.
- `PreserveSynthetic`:
  - Copies currently live synthetic orders.
  - Clears book.
  - Applies venue snapshot.
  - Re-adds preserved synthetic orders.
  - If a snapshot order collides with a synthetic ID, return duplicate error.

**Tests/docs**

Add tests for:

- `apply_op` strict behavior.
- Tolerant missing remove skip.
- Tolerant duplicate add skip.
- Tolerant non-decreasing update coerces to amend.
- Synthetic ID helper behavior.
- Snapshot replace removes synthetic orders.
- Snapshot preserve keeps synthetic orders and replaces venue orders.

**Done criteria**

- `dwellir::BookOp` import path remains compatible.
- Existing manual match loops can be replaced with `book.apply_op(...)`.
- Tolerant replay outcomes expose enough detail to count errors/skips/coercions.
- Snapshot preservation is deterministic and documented.

---

### Work item 4 — Dwellir reasoned decoding and replay integration

**Purpose**

Enrich Dwellir adapter outputs with operation causes while preserving existing `decode_line` / `load_capture` APIs and updating examples/benches to use core helpers.

**Key files**

- `src/dwellir.rs`
- `examples/replay_capture.rs`
- `examples/dwellir_live_sp500.rs`
- `examples/dwellir_debug_log_sp500.rs`
- `benches/replay.rs`
- `docs/api.md`
- `README.md`

**API surfaces**

Keep existing APIs:

```rust
pub fn decode_line(line: &str, scales: Scales) -> Result<Decoded, AdapterError>;
pub fn load_capture(path: impl AsRef<Path>, scales: Scales) -> Result<Capture, AdapterError>;
```

Add reasoned APIs:

```rust
pub enum DecodedWithMeta {
    Skip,
    Snapshot {
        orders: Vec<Order>,
        cause: OperationCause,
    },
    Updates(ReasonedUpdateBatch),
}

pub struct ReasonedUpdateBatch {
    pub ops: Vec<ReasonedBookOp>,
    pub collapsed_complete_fills: usize,
    pub dropped_duplicate_removes: usize,
    pub unresolved_new: usize,
}

pub fn decode_line_with_meta(
    line: &str,
    scales: Scales,
) -> Result<DecodedWithMeta, AdapterError>;
```

Optional if useful for benchmarks/debugging:

```rust
pub struct ReasonedCapture {
    pub snapshot: Vec<Order>,
    pub updates: Vec<Vec<ReasonedBookOp>>,
    pub stats: CaptureStats,
}

pub fn load_capture_with_meta(
    path: impl AsRef<Path>,
    scales: Scales,
) -> Result<ReasonedCapture, AdapterError>;
```

**Behavior**

Dwellir cause mapping:

- `raw_book_diff.new` + matching status:
  - `VenueDiffKind::New`
  - status from `order_statuses`, usually `"open"`.
- `raw_book_diff.remove`:
  - `VenueDiffKind::Remove`
  - status from matching `order_statuses` if present, such as `"filled"` or `"canceled"`.
- `raw_book_diff.update` with `newSz > 0`:
  - `VenueDiffKind::Update`
  - status if present.
- `raw_book_diff.update` or `modified` with zero size:
  - emit one `Remove`
  - cause `VenueDiffKind::CompleteFillCollapsed`.
- `raw_book_diff.modified`:
  - `VenueDiffKind::Modified`.
- `new` without matching `order_status`:
  - skip as today.
  - increment `unresolved_new`.

Existing `decode_line` should call the same internal parser and strip metadata so old consumers keep behavior.

Examples/bench updates:

- Replace local `match BookOp` apply helpers with `book.apply_op(...)` or `book.apply_op_with_policy(...)`.
- `replay_capture.rs` should report:
  - strict errors, or use `ReplayPolicy::DWELLIR_TOLERANT` and print applied/skipped/coerced/error counts.
- `dwellir_debug_log_sp500.rs` should use `decode_line_with_meta` so logs include causes.
- `benches/replay.rs` should benchmark `apply_op` helper to keep the benchmark path aligned with the public API.

**Tests/docs**

Add/extend Dwellir unit tests for:

- Reasoned new/update/modified/remove causes.
- Complete-fill collapse cause.
- Duplicate remove dropped count.
- Unresolved new count.
- Existing `decode_line` tests still pass.

**Done criteria**

- Existing `Decoded` and `Capture` consumers remain source-compatible.
- New metadata API exposes Dwellir status/diff cause without requiring raw JSON inspection.
- Examples compile under `--features dwellir`.
- Bench compiles and measures the helper path.

---

### Work item 5 — Public documentation and compatibility-focused test coverage

**Purpose**

Make the new simulator APIs discoverable and prove compatibility with the existing public API.

**Key files**

- `docs/api.md`
- `README.md`
- `docs/l4-protocol.md`
- `tests/book_basic.rs`
- `src/dwellir.rs`

**Docs updates**

`docs/api.md`:

- Add sections:
  - Queue position.
  - Deterministic taker matching.
  - Submit limit order policies.
  - Amend priority policy.
  - Core `BookOp` and reason metadata.
  - Replay policies and outcomes.
  - Snapshot policies.
  - Synthetic ID convention.
- Keep existing mutation/query sections and mark old APIs as still valid.

`README.md`:

- Update design choices:
  - Replace “No matcher” with “No venue matcher; simulator matching is deterministic and local.”
- Add concise counterfactual example:
  - Clone or preserve synthetic IDs.
  - Submit a GTC/IOC/FOK/PostOnly order.
  - Inspect fill deltas and queue position.
- Add Dwellir replay note:
  - Strict replay for validation.
  - Tolerant replay for long-running diagnostics.

`docs/l4-protocol.md`:

- Do not rewrite protocol body.
- Add a short local note near adapter discussion, if desired, clarifying:
  - L4 causes can identify diff/status transitions.
  - L4 cannot reconstruct individual trade counterparties without the trades stream.

**Test coverage**

Add tests across `tests/book_basic.rs` and Dwellir module tests to cover all new public behavior.

**Done criteria**

- `cargo test` passes.
- `cargo test --features dwellir` passes.
- Examples compile with `--features dwellir`.
- Public docs mention compatibility and explicitly state that synthetic ID namespace is advisory.

---

## 4. File-by-file impact

### `src/types.rs`

Changes:

- Add core `BookOp`.
- Add `OperationCause`, `OperationSource`, `VenueDiffKind`, `SimulatorCause`, `ReasonedBookOp`.
- Add replay policy enums if kept in core domain types.
- Add synthetic ID constants/functions.
- Possibly add submit/replay reject reason enums if not placed in `book.rs`.

Why:

- These are public domain types used by core, Dwellir, examples, and docs.

Dependencies:

- Must land before `src/book.rs` uses `BookOp` and `ReasonedBookOp`.

---

### `src/book.rs`

Changes:

- Add queue-position structs/methods.
- Add amend priority policy method.
- Add taker matching structs/methods.
- Add submit limit order helper.
- Add `apply_op` and `apply_op_with_policy`.
- Add snapshot policy method.
- Reuse existing mutation methods internally.

Why:

- `OrderBook` owns all state and is the only component that can safely inspect and mutate linked-list FIFO order.

Dependencies:

- Depends on new public types from `types.rs`.

---

### `src/lib.rs`

Changes:

- Re-export all new public types and helper functions.
- Re-export core `BookOp`.

Why:

- Existing API style re-exports crate-level domain types.

Dependencies:

- After `types.rs` and `book.rs` additions.

---

### `src/dwellir.rs`

Changes:

- Replace local `BookOp` definition with core `BookOp`.
- Preserve `l4_book::dwellir::BookOp` by re-exporting/importing core type.
- Add `decode_line_with_meta`.
- Add `DecodedWithMeta` and `ReasonedUpdateBatch`.
- Internally derive operation causes from `book_diffs` and `order_statuses`.
- Keep existing `decode_line` and `load_capture`.

Why:

- Metadata belongs at the adapter boundary, but operation type should be venue-neutral.

Dependencies:

- Depends on core `BookOp` and `OperationCause`.

---

### `tests/book_basic.rs`

Changes:

- Add simulator API tests:
  - Queue position.
  - Amend priority.
  - Taker matching.
  - Submit policies.
  - Replay outcomes.
  - Snapshot preservation.
  - Synthetic ID helpers.

Why:

- Core behavior is deterministic and should be covered by unit tests.

Dependencies:

- After core APIs are added.

---

### `examples/replay_capture.rs`

Changes:

- Use `book.apply_op_with_policy(...)`.
- Print applied/skipped/coerced/error counts.
- Optionally allow strict/tolerant mode through env var.

Why:

- Demonstrates tolerant replay outcomes.

Dependencies:

- After replay policy APIs.

---

### `examples/dwellir_live_sp500.rs`

Changes:

- Replace local `apply_op` helper with `book.apply_op(...)` or tolerant policy.
- Keep existing reporting logic.

Why:

- Avoid duplicate apply logic.

Dependencies:

- After `OrderBook::apply_op`.

---

### `examples/dwellir_debug_log_sp500.rs`

Changes:

- Use `decode_line_with_meta`.
- Include `OperationCause` in logged parsed update output.
- Use replay outcome helper for apply logging.

Why:

- This example is the natural place to expose enriched cause metadata.

Dependencies:

- After Dwellir metadata API and replay helpers.

---

### `benches/replay.rs`

Changes:

- Replace manual apply match with `book.apply_op(...)`.

Why:

- Bench the public operation helper path.

Dependencies:

- After core `apply_op`.

---

### `docs/api.md`

Changes:

- Add complete API reference for new simulator, replay, snapshot, and metadata APIs.

Why:

- Primary public API document.

---

### `README.md`

Changes:

- Add high-level simulator examples.
- Update design choices.
- Add synthetic ID and replay guidance.

Why:

- User-facing overview.

---

## 5. Risks and migration

- **Compatibility risk: `BookOp` location.**  
  Mitigation: re-export core `BookOp` from `dwellir` so `l4_book::dwellir::BookOp` remains valid.

- **Synthetic ID convention is not enforceable.**  
  `OrderId` is a `u64` alias. The high-bit namespace must be documented as advisory. Adapters should validate venue IDs before relying on `PreserveSynthetic`.

- **Replay tolerance can hide data issues.**  
  Default policy must remain strict. Tolerant outcomes must explicitly count skips/coercions/errors.

- **Submit helper atomicity.**  
  `submit_limit_order` must simulate before mutation for `Fok`, `PostOnly`, and duplicate-ID checks to avoid partial mutation on rejected outcomes.

- **Dwellir metadata limitation.**  
  L4 can identify status/diff causes, but cannot reconstruct individual trade executions/counterparties without the trades stream.

---

## 6. Implementation order

1. **Core type additions**
   - Add core `BookOp`, metadata types, synthetic ID helpers, replay/snapshot/submit enums.
   - Re-export from `src/lib.rs`.

2. **Core book query and amend APIs**
   - Implement queue position.
   - Implement `amend_size_with_policy`.
   - Add focused tests.

3. **Core simulator mutation APIs**
   - Implement `match_taker_order`.
   - Implement `apply_taker_order`.
   - Implement `submit_limit_order`.
   - Add fill/order-policy tests.

4. **Replay and snapshot helpers**
   - Implement `apply_op`, `apply_op_with_policy`.
   - Implement `apply_snapshot_with_policy`.
   - Add strict/tolerant/synthetic snapshot tests.

5. **Dwellir/examples/docs**
   - Migrate Dwellir to core `BookOp`.
   - Add reasoned decode API.
   - Update examples and bench.
   - Update README and `docs/api.md`.
   - Run `cargo test`, `cargo test --features dwellir`, and example compile checks.


## 7. Orchestration checklist

- [x] Item 1 — Core types, queue-position, and amend priority APIs in `src/types.rs`, `src/book.rs`, `src/lib.rs`, plus focused tests. Completed by agent `621587C7-70D3-40BD-938B-0C33883F72F9`; `cargo test` and `cargo test --features dwellir` passed.
- [x] Item 2 — Deterministic taker matching and `submit_limit_order` policies, with fill/order-policy tests. Completed by agent `B2CA40E7-7483-444C-A57D-E34EB3ED69EC`; `cargo test`, focused book tests, and `cargo test --features dwellir` passed.
- [x] Item 3 — Core `BookOp`, reasoned metadata, replay policy/outcomes, synthetic ID helpers, and snapshot policy helpers. Completed by agent `467BC239-6858-4399-8496-CA46885B9A8E`; `cargo test` and `cargo test --features dwellir` passed.
- [x] Item 4 — Dwellir reasoned decoding plus examples/bench migration to core replay helpers. Completed by agent `A75EC4B6-556B-456F-87E7-C28F8B6E052F`; `cargo test --features dwellir`, examples, and benches compile checks passed.
- [x] Item 5 — Public docs/readme updates and final compatibility verification (`cargo test`, dwellir tests/examples as feasible). Completed by agent `B6E98154-4CF2-4FB8-AA07-F60903FE243B`; `cargo fmt`, `cargo test`, `cargo test --features dwellir`, `cargo check --features dwellir --examples --benches`, and `cargo check --features dwellir --all-targets` passed.

> 💡 Continue this plan conversation with ask_oracle(chat_id: "counterfactual-apis-5EC94B", new_chat: false)