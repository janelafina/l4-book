use std::fmt;

pub type OrderId = u64;
pub type Price = u64;
pub type Qty = u64;
pub type Ts = u64;

/// 20-byte wallet address (e.g. Ethereum-style). Adapter-level concern how it
/// maps to a venue's representation; the book just stores and indexes bytes.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct WalletId(pub [u8; 20]);

impl WalletId {
    pub const ZERO: WalletId = WalletId([0; 20]);

    pub fn from_hex(s: &str) -> Result<Self, BookError> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        if s.len() != 40 {
            return Err(BookError::InvalidWalletHex);
        }
        let mut out = [0u8; 20];
        for i in 0..20 {
            let hi = hex_nib(s.as_bytes()[2 * i])?;
            let lo = hex_nib(s.as_bytes()[2 * i + 1])?;
            out[i] = (hi << 4) | lo;
        }
        Ok(WalletId(out))
    }
}

fn hex_nib(b: u8) -> Result<u8, BookError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(BookError::InvalidWalletHex),
    }
}

impl fmt::Debug for WalletId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x")?;
        for byte in self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Bid,
    Ask,
}

/// An order resting on the book. The venue-specific fields (tif, cloid, trigger
/// metadata) are intentionally not modeled here — an adapter can keep them in a
/// sidecar map keyed by `id` if needed.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Order {
    pub id: OrderId,
    pub wallet: WalletId,
    pub side: Side,
    pub price: Price,
    pub qty: Qty,
    pub ts: Ts,
}

/// Venue-neutral mutation operation for a resting-order book.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BookOp {
    Add(Order),
    Remove(OrderId),
    /// Partial-fill semantics: `new_qty` must strictly decrease, with zero
    /// removing the order.
    UpdateSize {
        id: OrderId,
        new_qty: Qty,
    },
    /// Venue/user amend semantics: `new_qty` may increase or decrease, with
    /// zero removing the order.
    AmendSize {
        id: OrderId,
        new_qty: Qty,
    },
}

/// Broad source classification for reasoned book operations.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OperationSource {
    Venue,
    Simulator,
    User,
    Unknown,
}

/// Venue diff category when an adapter can preserve cause metadata.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VenueDiffKind {
    New,
    Remove,
    Update,
    Modified,
    CompleteFillCollapsed,
}

/// Local simulator cause for generated operations.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SimulatorCause {
    TakerFill,
    LimitOrderRest,
    SnapshotPreserve,
}

/// Optional reason/cause metadata for an operation.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// A book operation bundled with reason/cause metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReasonedBookOp {
    pub op: BookOp,
    pub cause: OperationCause,
}

impl ReasonedBookOp {
    pub fn new(op: BookOp, cause: OperationCause) -> Self {
        Self { op, cause }
    }
}

/// High-bit namespace convention for local simulator orders.
pub const SYNTHETIC_ORDER_ID_FLAG: u64 = 1 << 63;

/// Returns true when `id` is in the advisory synthetic-order namespace.
pub fn is_synthetic_order_id(id: OrderId) -> bool {
    id & SYNTHETIC_ORDER_ID_FLAG != 0
}

/// Build a synthetic order id from a local id, returning `None` if `local_id`
/// already occupies the high-bit namespace.
pub fn synthetic_order_id(local_id: u64) -> Option<OrderId> {
    if is_synthetic_order_id(local_id) {
        None
    } else {
        Some(local_id | SYNTHETIC_ORDER_ID_FLAG)
    }
}

/// Queue statistics for a live order, or for a hypothetical new order that
/// would join the tail of a price level.
///
/// `orders_ahead`/`qty_ahead` are zero-based queue-position measures: an order
/// at the head of a level has no orders or quantity ahead of it. For a new
/// order, all current level quantity is ahead and nothing is behind.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// Priority behavior to apply when amending a resting order's size.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AmendPriorityPolicy {
    /// Preserve FIFO queue position regardless of size direction.
    Preserve,
    /// Preserve position on size decreases/equal size; move to the tail of the
    /// same price level when size increases.
    LosePriorityOnIncrease,
}

/// Per-resting-order fill produced by a deterministic taker match.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// Detailed result for a hypothetical or applied taker order.
#[derive(Clone, Debug, PartialEq)]
pub struct TakerMatch {
    pub taker_side: Side,
    pub requested_qty: Qty,
    pub filled_qty: Qty,
    pub unfilled_qty: Qty,
    /// Sum of `execution_price * filled_qty` across maker fills.
    pub filled_notional: u128,
    pub limit_price: Price,
    pub reference_price: Option<Price>,
    pub average_price: Option<f64>,
    pub fills: Vec<FillDelta>,
    /// True when the next available resting price would violate `limit_price`.
    pub limit_stopped: bool,
    /// True when the acceptable opposite book was exhausted before filling.
    pub exhausted_book: bool,
}

impl TakerMatch {
    pub fn is_complete(&self) -> bool {
        self.unfilled_qty == 0
    }
}

/// Time-in-force / crossing behavior for [`OrderBook::submit_limit_order`](crate::OrderBook::submit_limit_order).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LimitOrderPolicy {
    /// Good-till-cancel: take available liquidity, then rest any remainder.
    Gtc,
    /// Immediate-or-cancel: take available liquidity and cancel any remainder.
    Ioc,
    /// Fill-or-kill: only execute if the full quantity is immediately fillable.
    Fok,
    /// Maker-only: reject if the order would immediately cross the book.
    PostOnly,
}

/// Rejection reason for a submitted limit order that performed no mutation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SubmitRejectReason {
    PostOnlyWouldCross,
    FillOrKillWouldNotFill,
}

/// Outcome from [`OrderBook::submit_limit_order`](crate::OrderBook::submit_limit_order).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct SubmitLimitOrderOutcome {
    pub policy: LimitOrderPolicy,
    pub rejected: Option<SubmitRejectReason>,
    pub taker_match: TakerMatch,
    /// The order that was added to the book, if any. For GTC this is the
    /// unfilled remainder; for non-crossing PostOnly this is the full order.
    pub rested_order: Option<Order>,
}

/// Policy for applying a replacement snapshot.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SnapshotPolicy {
    /// Clear all state before inserting snapshot orders.
    Replace,
    /// Replace venue state, then re-add currently live synthetic orders.
    PreserveSynthetic,
}

/// Counts produced by snapshot application.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SnapshotOutcome {
    pub inserted: usize,
    pub preserved: usize,
}

/// Missing-order tolerance for replaying operations.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MissingOrderPolicy {
    Error,
    Skip,
}

/// Duplicate-add tolerance for replaying operations.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DuplicateAddPolicy {
    Error,
    Skip,
}

/// Non-decreasing `UpdateSize` tolerance for replaying operations.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NonDecreasingUpdatePolicy {
    Error,
    TreatAsAmend,
}

/// Replay behavior for applying venue/adapter operations.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ReplayPolicy {
    pub missing_order: MissingOrderPolicy,
    pub duplicate_add: DuplicateAddPolicy,
    pub non_decreasing_update: NonDecreasingUpdatePolicy,
}

impl ReplayPolicy {
    pub const STRICT: ReplayPolicy = ReplayPolicy {
        missing_order: MissingOrderPolicy::Error,
        duplicate_add: DuplicateAddPolicy::Error,
        non_decreasing_update: NonDecreasingUpdatePolicy::Error,
    };

    pub const DWELLIR_TOLERANT: ReplayPolicy = ReplayPolicy {
        missing_order: MissingOrderPolicy::Skip,
        duplicate_add: DuplicateAddPolicy::Skip,
        non_decreasing_update: NonDecreasingUpdatePolicy::TreatAsAmend,
    };
}

/// Why an operation was skipped or coerced under a tolerant replay policy.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ToleratedReplayReason {
    MissingOrder {
        id: OrderId,
    },
    DuplicateAdd {
        id: OrderId,
    },
    NonDecreasingUpdate {
        id: OrderId,
        current: Qty,
        proposed: Qty,
    },
}

/// Result of applying a single operation under a replay policy.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ReplayApplyOutcome {
    Applied {
        op: BookOp,
    },
    Skipped {
        op: BookOp,
        reason: ToleratedReplayReason,
    },
    Coerced {
        original: BookOp,
        applied: BookOp,
        reason: ToleratedReplayReason,
    },
    Error {
        op: BookOp,
        error: BookError,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BookError {
    DuplicateOrderId(OrderId),
    UnknownOrderId(OrderId),
    ZeroQty,
    /// Snapshot inputs are venue state; high-bit synthetic IDs are reserved for
    /// local simulator orders preserved across snapshots.
    SyntheticOrderIdInSnapshot(OrderId),
    /// Attempted a size update that doesn't shrink (partial-fill updates must
    /// strictly decrease; amends may go either way, so they use a separate path).
    NonDecreasingSize {
        current: Qty,
        proposed: Qty,
    },
    InvalidWalletHex,
}

impl fmt::Display for BookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BookError::DuplicateOrderId(oid) => write!(f, "duplicate order id {oid}"),
            BookError::UnknownOrderId(oid) => write!(f, "unknown order id {oid}"),
            BookError::ZeroQty => write!(f, "order qty must be nonzero"),
            BookError::SyntheticOrderIdInSnapshot(oid) => {
                write!(f, "snapshot order id {oid} is in the synthetic namespace")
            }
            BookError::NonDecreasingSize { current, proposed } => {
                write!(
                    f,
                    "update_size must shrink: current={current} proposed={proposed}"
                )
            }
            BookError::InvalidWalletHex => write!(f, "invalid wallet hex"),
        }
    }
}

impl std::error::Error for BookError {}
