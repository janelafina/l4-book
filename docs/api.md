# API Overview

`l4-book` exposes a venue-neutral L4 limit order book. L4 means every resting
order is tracked individually, like L3, and each order is also attributed to a
wallet address.

Prices and quantities are fixed-point integer values. Adapters are responsible
for converting decimal venue values into `u64` ticks/lots before inserting
orders into the core book.

The core book can also run local, deterministic counterfactual simulations:
queue-position queries, taker matching, submit-limit policies, replay helpers,
and snapshot policies. These simulator APIs do **not** claim to reproduce a
venue's private matcher; they deterministically walk the local book state.

## Core Types

```rust
use l4_book::{Order, OrderBook, Side, WalletId};
```

Type aliases:

| Type | Definition | Meaning |
|---|---:|---|
| `OrderId` | `u64` | Unique venue/order identifier |
| `Price` | `u64` | Fixed-point price |
| `Qty` | `u64` | Fixed-point quantity |
| `Ts` | `u64` | Timestamp supplied by the caller/adapter |

Public structs and enums:

| Type | Purpose |
|---|---|
| `Order` | Resting order: `{ id, wallet, side, price, qty, ts }` |
| `Side` | `Side::Bid` or `Side::Ask` |
| `WalletId` | 20-byte wallet address wrapper, with `WalletId::from_hex(...)` |
| `BookError` | Errors from invalid book mutations |
| `OrderBook` | In-memory L4 book with order, price-level, and wallet indexes |

`BookError` variants are:

```rust
DuplicateOrderId(OrderId)
UnknownOrderId(OrderId)
ZeroQty
SyntheticOrderIdInSnapshot(OrderId)
NonDecreasingSize { current: Qty, proposed: Qty }
InvalidWalletHex
```

## Constructing a Book

```rust
let mut book = OrderBook::new();
let mut preallocated = OrderBook::with_capacity(100_000);
```

`OrderBook::with_capacity` reserves storage for the order index and internal
slab. It is useful when applying large snapshots.

## Mutating State

```rust
book.add(order)?;
book.update_size(order_id, new_qty)?;
book.amend_size(order_id, new_qty)?;
book.amend_size_with_policy(order_id, new_qty, AmendPriorityPolicy::LosePriorityOnIncrease)?;
let removed = book.remove(order_id)?;
book.apply_snapshot(snapshot_orders)?;
book.clear();
```

| Method | Return Type | Notes |
|---|---|---|
| `add(Order)` | `Result<(), BookError>` | Inserts a new resting order. Rejects duplicate IDs and zero quantity. |
| `remove(OrderId)` | `Result<Order, BookError>` | Removes and returns an order by ID. |
| `update_size(OrderId, Qty)` | `Result<(), BookError>` | Partial-fill semantics. `new_qty` must strictly shrink; `0` removes the order. Queue position is preserved. |
| `amend_size(OrderId, Qty)` | `Result<(), BookError>` | Amend semantics. Size may grow or shrink; `0` removes the order. Queue position is preserved. |
| `amend_size_with_policy(OrderId, Qty, AmendPriorityPolicy)` | `Result<(), BookError>` | Explicit queue-priority behavior for amends. |
| `apply_snapshot(iter)` | `Result<(), BookError>` | Equivalent to `SnapshotPolicy::Replace`: clears existing state, then adds every order. |
| `clear()` | `()` | Removes all state. |

`AmendPriorityPolicy`:

| Policy | Behavior |
|---|---|
| `Preserve` | Keep FIFO queue position for increases and decreases. This is the default used by `amend_size`. |
| `LosePriorityOnIncrease` | Keep position for decreases/equal size; move the order to the tail of its current price level when size increases. |

## Basic Queries

```rust
let n = book.len();
let empty = book.is_empty();
let order = book.get(order_id);
let best_bid = book.best_bid();
let best_ask = book.best_ask();
let (bid, ask) = book.best_bid_ask();
```

| Method | Return Type |
|---|---|
| `len()` | `usize` |
| `is_empty()` | `bool` |
| `get(OrderId)` | `Option<&Order>` |
| `best_bid()` | `Option<Price>` |
| `best_ask()` | `Option<Price>` |
| `best_bid_ask()` | `(Option<Price>, Option<Price>)` |

## Depth, Order Iteration, and Queue Position

```rust
for (price, total_qty, order_count) in book.depth(Side::Bid) {
    // Bids are best-to-worse: highest price first.
}

for order in book.orders_at(Side::Ask, price) {
    // Orders at the level are FIFO.
}

for order in book.orders_by_wallet(wallet) {
    // Wallet order is unspecified.
}

let live_pos = book.queue_position(order_id)?;
let new_pos = book.queue_position_for_new_order(Side::Bid, price);
```

| Method | Return Type | Ordering |
|---|---|---|
| `depth(Side)` | `DepthIter<'_>` yielding `(Price, Qty, u64)` | Bids descending, asks ascending |
| `orders_at(Side, Price)` | `OrdersAtLevel<'_>` yielding `&Order` | FIFO within the price level |
| `orders_by_wallet(WalletId)` | `WalletIter<'_>` yielding `&Order` | Unspecified hash-set order |
| `queue_position(OrderId)` | `Result<QueuePosition, BookError>` | Counts live order's FIFO position at its level |
| `queue_position_for_new_order(Side, Price)` | `QueuePosition` | Treats the hypothetical order as joining the tail |

`QueuePosition` uses zero-based queue-position semantics: a level-head order has
`orders_ahead == 0` and `qty_ahead == 0`. For a hypothetical new order, all
current level orders/quantity are ahead and nothing is behind.

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
```

## Owned Top-of-Book Snapshots

Use these methods when an API or UI needs owned data instead of borrowed
iterators.

```rust
let top = book.top_n_levels(5);
let agg = book.top_n_levels_aggregated(5);
```

`top_n_levels(n) -> UnaggregatedTopLevels`:

```rust
pub struct UnaggregatedTopLevels {
    pub bids: Vec<UnaggregatedLevel>,
    pub asks: Vec<UnaggregatedLevel>,
}

pub struct UnaggregatedLevel {
    pub price: Price,
    pub orders: Vec<LevelOrder>,
}

pub struct LevelOrder {
    pub id: OrderId,
    pub wallet: WalletId,
    pub qty: Qty,
    pub ts: Ts,
}
```

`top_n_levels_aggregated(n) -> AggregatedTopLevels`:

```rust
pub struct AggregatedTopLevels {
    pub bids: Vec<AggregatedLevel>,
    pub asks: Vec<AggregatedLevel>,
}

pub struct AggregatedLevel {
    pub price: Price,
    pub total_qty: Qty,
    pub order_count: u64,
}
```

Both snapshot shapes return bid levels best-to-worse by descending price and ask
levels best-to-worse by ascending price.

## Slippage Estimation

```rust
let estimate = book.estimate_slippage(Side::Bid, qty, limit_price);
```

`Side::Bid` means a hypothetical buy that consumes asks up to `limit_price`.
`Side::Ask` means a hypothetical sell that consumes bids down to `limit_price`.
The book is not mutated.

`estimate_slippage(...) -> SlippageEstimate`:

```rust
pub struct SlippageEstimate {
    pub taker_side: Side,
    pub requested_qty: Qty,
    pub filled_qty: Qty,
    pub unfilled_qty: Qty,
    pub filled_notional: u128,
    pub limit_price: Price,
    pub reference_price: Option<Price>,
    pub average_price: Option<f64>,
    pub slippage: Option<f64>,
    pub slippage_notional: Option<u128>,
    pub slippage_pct: Option<f64>,
    pub limit_stopped: bool,
    pub exhausted_book: bool,
}
```

`SlippageEstimate::is_complete() -> bool` is true when `unfilled_qty == 0`.

## Deterministic Taker Matching

Use taker matching when you need per-maker fill deltas rather than aggregate
slippage only.

```rust
let simulated = book.match_taker_order(Side::Bid, qty, limit_price);
let applied = book.apply_taker_order(Side::Bid, qty, limit_price)?;
```

| Method | Mutates? | Return Type | Notes |
|---|---:|---|---|
| `match_taker_order(Side, Qty, Price)` | No | `TakerMatch` | Deterministically walks opposite-side orders by price-time priority. |
| `apply_taker_order(Side, Qty, Price)` | Yes | `Result<TakerMatch, BookError>` | Applies the same fills by shrinking/removing maker orders. Rejects zero quantity. |

Matching order:

- Bid takers consume asks from lowest to highest price.
- Ask takers consume bids from highest to lowest price.
- Orders at the same price are consumed FIFO.
- `limit_stopped` is true when the next resting price violates the limit.
- `exhausted_book` is true when acceptable opposite liquidity runs out first.

```rust
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
```

`TakerMatch::is_complete() -> bool` is true when `unfilled_qty == 0`.

## Submit Limit Order Policies

`submit_limit_order` is a local simulator helper for a single incoming limit
order. It is deterministic and operates only on the local `OrderBook` state.

```rust
use l4_book::LimitOrderPolicy;

let outcome = book.submit_limit_order(order, LimitOrderPolicy::Gtc)?;
```

Policies:

| Policy | Behavior |
|---|---|
| `Gtc` | Take available liquidity, then rest any unfilled remainder using the submitted order ID. |
| `Ioc` | Take available liquidity and cancel any unfilled remainder. |
| `Fok` | Simulate first; if the full size is not immediately fillable, reject without mutating. |
| `PostOnly` | Reject without mutating if the order would cross; otherwise rest the full order. |

```rust
pub struct SubmitLimitOrderOutcome {
    pub policy: LimitOrderPolicy,
    pub rejected: Option<SubmitRejectReason>,
    pub taker_match: TakerMatch,
    pub rested_order: Option<Order>,
}

pub enum SubmitRejectReason {
    PostOnlyWouldCross,
    FillOrKillWouldNotFill,
}
```

`submit_limit_order` rejects duplicate IDs and zero quantity with `BookError`
before mutating. Rejected FOK/PostOnly submissions report a `rejected` reason and
do not mutate the book.

## Core Book Operations and Replay Policies

`BookOp` is the venue-neutral mutation command type used by adapters, examples,
benchmarks, and replay helpers.

```rust
pub enum BookOp {
    Add(Order),
    Remove(OrderId),
    UpdateSize { id: OrderId, new_qty: Qty },
    AmendSize { id: OrderId, new_qty: Qty },
}

book.apply_op(BookOp::Remove(order_id))?;
let outcome = book.apply_op_with_policy(op, ReplayPolicy::DWELLIR_TOLERANT);
```

| Method | Return Type | Notes |
|---|---|---|
| `apply_op(BookOp)` | `Result<(), BookError>` | Strict apply helper equivalent to calling `add`/`remove`/`update_size`/`amend_size`. |
| `apply_op_with_policy(BookOp, ReplayPolicy)` | `ReplayApplyOutcome` | Applies, skips, coerces, or reports an error according to replay tolerance. |

Replay policies:

| Policy | Missing order | Duplicate add | Non-decreasing `UpdateSize` |
|---|---|---|---|
| `ReplayPolicy::STRICT` | Error | Error | Error |
| `ReplayPolicy::DWELLIR_TOLERANT` | Skip | Skip | Treat as `AmendSize` |

`ReplayApplyOutcome`:

```rust
Applied { op: BookOp }
Skipped { op: BookOp, reason: ToleratedReplayReason }
Coerced { original: BookOp, applied: BookOp, reason: ToleratedReplayReason }
Error { op: BookOp, error: BookError }
```

`ToleratedReplayReason` identifies the tolerated anomaly:
`MissingOrder { id }`, `DuplicateAdd { id }`, or
`NonDecreasingUpdate { id, current, proposed }`.

Use strict replay when validating captures or adapter behavior. Use tolerant
replay for long-running diagnostics where known feed artifacts should be counted
rather than aborting the run.

## Operation Reason/Cause Metadata

Operation metadata is represented independently from application. The core type
is:

```rust
pub struct ReasonedBookOp {
    pub op: BookOp,
    pub cause: OperationCause,
}
```

`OperationCause` can identify venue, simulator, snapshot, user, or unknown
causes:

```rust
Venue { source: &'static str, diff: VenueDiffKind, status: Option<String> }
Simulator(SimulatorCause)
Snapshot
User
Unknown
```

Venue diff kinds are `New`, `Remove`, `Update`, `Modified`, and
`CompleteFillCollapsed`. Simulator causes are `TakerFill`, `LimitOrderRest`, and
`SnapshotPreserve`. Cause metadata is informational; callers apply the embedded
`BookOp` with `apply_op` or `apply_op_with_policy`.

## Snapshot Policies and Synthetic IDs

The historical `apply_snapshot(iter)` behavior is preserved and remains a full
replace. Use `apply_snapshot_with_policy` when simulator orders should survive a
venue snapshot.

```rust
let outcome = book.apply_snapshot_with_policy(
    venue_snapshot,
    SnapshotPolicy::PreserveSynthetic,
)?;

println!("inserted={} preserved={}", outcome.inserted, outcome.preserved);
```

| Policy | Behavior |
|---|---|
| `SnapshotPolicy::Replace` | Clear all state, then insert snapshot orders. Equivalent to `apply_snapshot`. |
| `SnapshotPolicy::PreserveSynthetic` | Copy currently live synthetic orders, replace venue state, then re-add preserved synthetic orders. |

Synthetic order IDs use the high bit by convention:

```rust
use l4_book::{is_synthetic_order_id, synthetic_order_id, SYNTHETIC_ORDER_ID_FLAG};

let id = synthetic_order_id(42).expect("local id must not already use high bit");
assert!(is_synthetic_order_id(id));
```

`synthetic_order_id(local_id)` returns `None` if `local_id` already has the high
bit set. This namespace is advisory, not enforced by the `OrderId = u64` type.
`PreserveSynthetic` rejects snapshot orders in the synthetic namespace, usually
with `BookError::SyntheticOrderIdInSnapshot`; a snapshot order that collides with
a currently preserved synthetic order can return `BookError::DuplicateOrderId`.
Venue adapters should validate that venue order IDs do not collide before
relying on preserved synthetic orders.

## Optional Dwellir Adapter

Enable the `dwellir` feature to decode Dwellir/Hyperliquid L4 JSONL captures:

```toml
l4-book = { version = "...", features = ["dwellir"] }
```

The adapter exports:

| Type or Function | Purpose |
|---|---|
| `dwellir::Scales` | Fixed-point conversion settings for decimal wire strings |
| `dwellir::decode_line(...)` | Decodes one JSONL line into `Decoded`; strips cause metadata for compatibility |
| `dwellir::decode_line_with_meta(...)` | Decodes one JSONL line into `DecodedWithMeta` with Dwellir-derived causes |
| `dwellir::load_capture(...)` | Loads a capture into snapshot orders and update batches |
| `dwellir::load_capture_with_meta(...)` | Loads a capture while preserving per-operation cause metadata |
| `dwellir::BookOp` | Compatibility re-export of the core `BookOp` type |
| `dwellir::Decoded` | `Skip`, `Snapshot(Vec<Order>)`, or `Updates(UpdateBatch)` |
| `dwellir::DecodedWithMeta` | `Skip`, `Snapshot { orders, cause }`, or `Updates(ReasonedUpdateBatch)` |
| `dwellir::ReasonedUpdateBatch` | Reasoned ops plus `collapsed_complete_fills`, `dropped_duplicate_removes`, and `unresolved_new` counts |
| `dwellir::ReasonedCapture` | Whole-file decode result preserving per-operation cause metadata |
| `dwellir::CaptureStats` | Counts decoded ops and feed quirks |

Typical application loop:

```rust
use l4_book::dwellir::{Decoded, Scales, decode_line};
use l4_book::{OrderBook, ReplayApplyOutcome, ReplayPolicy};

let mut book = OrderBook::new();
let policy = ReplayPolicy::DWELLIR_TOLERANT;

match decode_line(line, Scales::BTC_DEFAULT)? {
    Decoded::Skip => {}
    Decoded::Snapshot(orders) => book.apply_snapshot(orders)?,
    Decoded::Updates(batch) => {
        for op in batch.ops {
            match book.apply_op_with_policy(op, policy) {
                ReplayApplyOutcome::Applied { .. } => {}
                other => eprintln!("replay anomaly: {other:?}"),
            }
        }
    }
}
```

For provenance/debugging, use `decode_line_with_meta` and inspect each
`ReasonedBookOp.cause`. Dwellir metadata can preserve diff/status transitions,
but L4 book diffs alone do not identify individual trade counterparties; use a
trades feed if you need execution-counterparty attribution.
