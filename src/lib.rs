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

pub use book::{DepthIter, OrderBook, OrdersAtLevel, WalletIter};
pub use types::{BookError, Order, OrderId, Price, Qty, Side, Ts, WalletId};
