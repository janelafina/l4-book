# l4-book

A standalone L4 limit order book in Rust.

An L4 book tracks every resting order individually — the same granularity as L3
— *and* attributes each order to a wallet address. That wallet attribution is
the defining L4 feature: it's what lets you do queue-position tracking, whale
watching, and per-participant microstructure analysis.

The core crate is venue-agnostic: prices and sizes are `u64` fixed-point, and
no wire-format types leak into the public API. An optional `dwellir` feature
wires in a decoder for Dwellir / Hyperliquid L4 JSONL captures, which is what
the benchmark harness and the included capture script use.

## Layout

```
l4-book/
├── Cargo.toml
├── src/
│   ├── lib.rs            module wiring
│   ├── types.rs          Order, Side, WalletId([u8;20]), BookError
│   ├── level.rs          Level + OrderNode (crate-private)
│   ├── book.rs           OrderBook + iterators
│   └── dwellir.rs        Dwellir/Hyperliquid JSONL decoder (feature = "dwellir")
├── tests/book_basic.rs   behavior tests
├── benches/replay.rs     Criterion: snapshot apply + 5 min replay + per-update
├── examples/replay_capture.rs   wall-clock end-to-end sanity run
├── scripts/
│   ├── capture_l4.py     WebSocket capture → JSONL
│   └── requirements.txt
├── docs/api.md           public crate API overview
└── docs/l4-protocol.md   Dwellir L4 wire-format reference (verbatim)
```

## Core API

See [`docs/api.md`](docs/api.md) for a compact reference covering the book data
structure, public types, mutation/query methods, iterator outputs, snapshots,
slippage estimates, counterfactual simulator APIs, replay helpers, and the
optional Dwellir adapter.

```rust
use l4_book::{OrderBook, Order, Side, WalletId};

let mut book = OrderBook::new();
book.add(Order { id, wallet, side, price, qty, ts })?;
book.update_size(id, new_qty)?;   // partial fill; must strictly shrink
book.amend_size(id, new_qty)?;    // may grow or shrink
book.remove(id)?;

book.best_bid();                                 // Option<Price>
book.best_bid_ask();                             // (Option<Price>, Option<Price>)
book.depth(Side::Bid);                           // iter (price, total_qty, order_count)
book.orders_at(Side::Bid, price);                // iter &Order in FIFO order
book.orders_by_wallet(wallet);                   // iter &Order
book.queue_position(id)?;                          // FIFO orders/qty ahead/behind
```

### Counterfactual simulator APIs

The crate includes local deterministic simulator helpers for "what if" analysis.
They walk the current in-memory book by price-time priority; they are not a
venue matcher and do not infer hidden venue state.

```rust
use l4_book::{
    LimitOrderPolicy, Order, ReplayApplyOutcome, ReplayPolicy, Side,
    SnapshotPolicy, synthetic_order_id,
};

// Inspect queue position for live or hypothetical orders.
let pos = book.queue_position(id)?;
let join_tail = book.queue_position_for_new_order(Side::Bid, price);

// Simulate per-maker fills without mutation, or apply them to the book.
let simulated = book.match_taker_order(Side::Bid, 10_000, 50_100);
let applied = book.apply_taker_order(Side::Bid, 10_000, 50_100)?;

// Submit a local synthetic GTC/IOC/FOK/PostOnly limit order.
let sim_id = synthetic_order_id(1).expect("local id must not already use high bit");
let outcome = book.submit_limit_order(
    Order { id: sim_id, wallet, side: Side::Bid, price: 50_100, qty: 10_000, ts },
    LimitOrderPolicy::Gtc,
)?;

// Preserve high-bit synthetic orders across venue snapshots when desired.
let snap = book.apply_snapshot_with_policy(venue_snapshot, SnapshotPolicy::PreserveSynthetic)?;

// Adapter/replay paths can use strict validation or tolerant diagnostics.
match book.apply_op_with_policy(op, ReplayPolicy::DWELLIR_TOLERANT) {
    ReplayApplyOutcome::Applied { .. } => {}
    other => eprintln!("replay anomaly: {other:?}"),
}
```

Synthetic order IDs use the high bit by convention. The namespace is advisory
because `OrderId` is still a plain `u64`; adapters should validate venue IDs
before relying on preserved synthetic orders.

### Top-of-book snapshots and slippage

For UI/API consumers that want owned snapshots instead of iterators:

```rust
let top = book.top_n_levels(5);
// top.bids: best-to-worse bid levels (descending price)
// top.asks: best-to-worse ask levels (ascending price)
// each level has { price, orders }, with FIFO orders containing id, wallet, qty, ts
// note: all orders in each selected price level are copied into the snapshot

let agg = book.top_n_levels_aggregated(5);
// each level has { price, total_qty, order_count }
```

Slippage estimates are taker-side oriented: `Side::Bid` means a buy that
consumes asks up to the limit price, and `Side::Ask` means a sell that consumes
bids down to the limit price. The book is not mutated.

```rust
let est = book.estimate_slippage(Side::Bid, 10_000, 50_100);

est.average_price;   // Option<f64>
est.filled_notional; // exact sum(price * qty) as u128
est.slippage;        // raw average-price move vs current best ask/bid
est.slippage_notional; // exact total slippage notional vs current best ask/bid
est.slippage_pct;    // percent move vs current best ask/bid
est.filled_qty;
est.unfilled_qty;
est.limit_stopped;   // next available price violated the limit
est.exhausted_book;  // acceptable opposite liquidity ran out
est.is_complete();
```

### Design choices

* **Slab storage** — `Vec<Option<OrderNode>>` with a free list: O(1) alloc/free,
  stable slot indices, no reallocation churn after warm-up.
* **Doubly-linked list per price level** preserving FIFO time priority; any
  order unlinks in O(1).
* **`BTreeMap<Price, Level>`** for sorted access, best-price via `next`/`next_back`.
* **`HashMap<OrderId, slot>`** for O(1) lookup by id.
* **`HashMap<WalletId, HashSet<OrderId>>`** is the L4 attribution index.
* **Empty-level pruning** on last-order removal keeps the price tree compact.
* **No venue matcher** — venues match upstream and adapters reconstruct the
  book from snapshot + diffs. The crate also provides local deterministic
  simulator matching for counterfactual analysis (`match_taker_order`,
  `apply_taker_order`, and `submit_limit_order`).

## Tests

```
cargo test
cargo test --features dwellir     # also runs the adapter's unit tests
```

## Benchmark workflow

The benchmarks replay a real Dwellir L4 capture against the book. Capture
data is **not** committed (the file is 1.5 GB for 5 minutes of BTC) — you
generate it yourself.

### 1. Install Python deps (for the capture script)

```bash
python3 -m venv .venv
.venv/bin/pip install -r scripts/requirements.txt
```

### 2. Capture a snapshot + 5 minutes of L4 updates

```bash
export DWELLIR_WS_URL="wss://<your-dwellir-host>/<your-token>/ws"
.venv/bin/python scripts/capture_l4.py \
    --duration 300 \
    --out benchmark_data/btc_l4_capture.jsonl
```

The output is newline-delimited JSON with a metadata header line followed by
`{recv_ns, seq, kind, msg}` records — one per WebSocket frame.

### 3. Sanity-check end-to-end

```bash
cargo run --release --example replay_capture --features dwellir
```

Prints load time, snapshot apply time, full-stream apply time, replay outcome
counts, and the final book shape. By default it uses `ReplayPolicy::DWELLIR_TOLERANT`
so known feed artifacts are counted instead of aborting; set
`L4_REPLAY_STRICT=1` to validate with strict `apply_op`-equivalent behavior.

### Live Dwellir SP500 example

```bash
export DWELLIR_WS_ENDPOINT="wss://<your-dwellir-host>/<your-token>/ws"
cargo run --release --example dwellir_live_sp500 --features dwellir
```

The live example subscribes to `l4Book` for `xyz:SP500`, maintains an in-memory
`OrderBook`, and every 15 seconds for 2 minutes after the snapshot logs top 5
unaggregated levels, top 5 aggregated levels, and buy/sell slippage estimates
for $1k and $100k notional orders.

For deeper feed debugging, use the full-book debug logger:

```bash
export DWELLIR_WS_ENDPOINT="wss://<your-dwellir-host>/<your-token>/ws"
export DWELLIR_DEBUG_UPDATES=5              # optional; defaults to 5
export DWELLIR_DEBUG_LOG=dwellir-debug.json # optional; defaults to dwellir_sp500_debug_log.json
cargo run --release --example dwellir_debug_log_sp500 --features dwellir
```

The debug logger writes newline-separated pretty JSON objects containing the
initial full snapshot, raw + parsed forms for the first `DWELLIR_DEBUG_UPDATES`
update messages, Dwellir-derived operation causes, the full book after each of
those updates, and all apply/decode errors. It stops after 10 accumulated
errors.

### 4. Criterion benchmarks

```bash
cargo bench --features dwellir
```

#### Reference numbers

Run locally on macOS, Apple Silicon, release profile (`lto = "thin"`), against
a 5-minute BTC perp capture (50,798 snapshot orders, 3,897 Updates messages,
367,092 total book ops):

| benchmark | time | throughput |
|---|---|---|
| `snapshot/apply` — load 50,798 orders | 4.1 ms | 12.3 Melem/s |
| `replay/stream_300s` — apply 367k ops | 28.9 ms | 12.7 Melem/s |
| `per_update/mid_update` — single batch | ~730 ns | 13.7 Melem/s |

All three cluster around ~75 ns/op. The hot path is one `HashMap` lookup on
`OrderId` plus one `BTreeMap` access on `Price`.

## Redaction note

The Dwellir endpoint contains a per-account token and is never committed. The
capture script reads it from `$DWELLIR_WS_URL` or `--endpoint`; the capture
JSONL itself also embeds the URL in its header record, which is why
`benchmark_data/` is gitignored.
