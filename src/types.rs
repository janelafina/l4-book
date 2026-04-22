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

#[derive(Debug, PartialEq, Eq)]
pub enum BookError {
    DuplicateOrderId(OrderId),
    UnknownOrderId(OrderId),
    ZeroQty,
    /// Attempted a size update that doesn't shrink (partial-fill updates must
    /// strictly decrease; amends may go either way, so they use a separate path).
    NonDecreasingSize { current: Qty, proposed: Qty },
    InvalidWalletHex,
}

impl fmt::Display for BookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BookError::DuplicateOrderId(oid) => write!(f, "duplicate order id {oid}"),
            BookError::UnknownOrderId(oid) => write!(f, "unknown order id {oid}"),
            BookError::ZeroQty => write!(f, "order qty must be nonzero"),
            BookError::NonDecreasingSize { current, proposed } => {
                write!(f, "update_size must shrink: current={current} proposed={proposed}")
            }
            BookError::InvalidWalletHex => write!(f, "invalid wallet hex"),
        }
    }
}

impl std::error::Error for BookError {}
