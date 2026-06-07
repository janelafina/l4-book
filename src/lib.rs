//! L4 limit order book.
//!
//! An L4 book tracks every resting order individually (like L3) *and* attributes
//! each one to a wallet address. The core is venue-agnostic: callers translate
//! their wire format into the [`Order`] and command surface exposed here.
//!
//! Prices and sizes are unsigned fixed-point integers (ticks and lots). Decimal
//! handling is the adapter's job.

mod book;
mod level;
mod types;

#[cfg(feature = "dwellir")]
pub mod dwellir;

pub use book::{
    AggregatedLevel, AggregatedTopLevels, DepthIter, LevelOrder, OrderBook, OrdersAtLevel,
    SlippageEstimate, UnaggregatedLevel, UnaggregatedTopLevels, WalletIter,
};
pub use types::{
    AmendPriorityPolicy, BookError, BookOp, DuplicateAddPolicy, FillDelta, LimitOrderPolicy,
    MissingOrderPolicy, NonDecreasingUpdatePolicy, OperationCause, OperationSource, Order, OrderId,
    Price, Qty, QueuePosition, ReasonedBookOp, ReplayApplyOutcome, ReplayPolicy,
    SYNTHETIC_ORDER_ID_FLAG, Side, SimulatorCause, SnapshotOutcome, SnapshotPolicy,
    SubmitLimitOrderOutcome, SubmitRejectReason, TakerMatch, ToleratedReplayReason, Ts,
    VenueDiffKind, WalletId, is_synthetic_order_id, synthetic_order_id,
};
