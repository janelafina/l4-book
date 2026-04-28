//! Dwellir / Hyperliquid L4 JSONL decoder.
//!
//! Decodes capture files produced by the companion `capture_l4.py` script into
//! venue-agnostic [`Order`] values and [`BookOp`] commands. The core
//! [`OrderBook`](crate::OrderBook) stays oblivious to Hyperliquid's wire format —
//! everything protocol-specific lives here.
//!
//! Fixed-point scaling: prices and sizes arrive as decimal strings. The decoder
//! converts them to `u64` by multiplying by `10^digits`. For BTC perps,
//! `price_digits = 6` and `qty_digits = 8` are safe defaults (Hyperliquid uses
//! fewer; truncation is toward zero).

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

use crate::{Order, OrderId, Qty, Side, WalletId};

/// Venue-neutral book command emitted from a single Dwellir `Updates` message.
#[derive(Clone, Debug)]
pub enum BookOp {
    Add(Order),
    Remove(OrderId),
    /// Partial-fill size reduction. Strictly decreasing per L4 semantics.
    UpdateSize {
        id: OrderId,
        new_qty: Qty,
    },
    /// Amendment; may grow or shrink.
    AmendSize {
        id: OrderId,
        new_qty: Qty,
    },
}

/// One line of the capture file, decoded.
#[derive(Debug)]
pub enum Decoded {
    /// Capture header / subscriptionResponse / unrecognized — nothing to apply.
    Skip,
    Snapshot(Vec<Order>),
    Updates(Vec<BookOp>),
}

/// Whole-file decode result.
pub struct Capture {
    pub snapshot: Vec<Order>,
    /// Outer vec: one entry per `Updates` message, in arrival order.
    /// Inner vec: ops within that update in arrival order.
    pub updates: Vec<Vec<BookOp>>,
    pub stats: CaptureStats,
}

#[derive(Default, Debug, Clone)]
pub struct CaptureStats {
    pub lines: usize,
    pub updates_messages: usize,
    pub total_ops: usize,
    pub adds: usize,
    pub removes: usize,
    pub size_updates: usize,
    pub size_amends: usize,
    /// `new` diffs we couldn't match to an `open` order_status in the same message.
    pub unresolved_new: usize,
}

#[derive(Debug)]
pub enum AdapterError {
    Io(std::io::Error),
    Json(serde_json::Error),
    BadPrice(String),
    BadQty(String),
    BadWallet(String),
    BadSide(String),
    MissingField(&'static str),
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdapterError::Io(e) => write!(f, "io: {e}"),
            AdapterError::Json(e) => write!(f, "json: {e}"),
            AdapterError::BadPrice(s) => write!(f, "bad price {s:?}"),
            AdapterError::BadQty(s) => write!(f, "bad qty {s:?}"),
            AdapterError::BadWallet(s) => write!(f, "bad wallet {s:?}"),
            AdapterError::BadSide(s) => write!(f, "bad side {s:?}"),
            AdapterError::MissingField(f2) => write!(f, "missing field {f2}"),
        }
    }
}

impl std::error::Error for AdapterError {}

impl From<std::io::Error> for AdapterError {
    fn from(e: std::io::Error) -> Self {
        AdapterError::Io(e)
    }
}
impl From<serde_json::Error> for AdapterError {
    fn from(e: serde_json::Error) -> Self {
        AdapterError::Json(e)
    }
}

/// Number of fractional decimal digits preserved when converting wire strings
/// (like `"90057.0"`) to fixed-point `u64`. A value of `6` means multiply by
/// `10^6`.
#[derive(Copy, Clone, Debug)]
pub struct Scales {
    pub price_digits: u32,
    pub qty_digits: u32,
}

impl Scales {
    pub const BTC_DEFAULT: Scales = Scales {
        price_digits: 6,
        qty_digits: 8,
    };
}

/// Decode a single JSONL line. Returns `Decoded::Skip` for the capture header
/// and subscriptionResponse.
pub fn decode_line(line: &str, scales: Scales) -> Result<Decoded, AdapterError> {
    let v: Value = serde_json::from_str(line)?;
    // Capture header written by our own script.
    if v.get("type").and_then(Value::as_str) == Some("capture_header") {
        return Ok(Decoded::Skip);
    }
    // Records wrap the wire message in {recv_ns, seq, kind, msg}.
    let msg = v.get("msg").unwrap_or(&v);

    let channel = msg.get("channel").and_then(Value::as_str);
    match channel {
        Some("subscriptionResponse") => Ok(Decoded::Skip),
        Some("l4Book") => decode_l4(msg, scales),
        _ => Ok(Decoded::Skip),
    }
}

fn decode_l4(msg: &Value, scales: Scales) -> Result<Decoded, AdapterError> {
    let data = msg.get("data").ok_or(AdapterError::MissingField("data"))?;
    if let Some(snap) = data.get("Snapshot") {
        let levels = snap
            .get("levels")
            .ok_or(AdapterError::MissingField("levels"))?;
        let arr = levels
            .as_array()
            .ok_or(AdapterError::MissingField("levels[]"))?;
        let mut out =
            Vec::with_capacity(arr.iter().map(|s| s.as_array().map_or(0, Vec::len)).sum());
        // levels[0] = bids, levels[1] = asks — side is also on each order though.
        for side_arr in arr {
            if let Some(orders) = side_arr.as_array() {
                for o in orders {
                    out.push(parse_order(o, None, scales)?);
                }
            }
        }
        return Ok(Decoded::Snapshot(out));
    }
    if let Some(updates) = data.get("Updates") {
        return Ok(Decoded::Updates(decode_updates(updates, scales)?));
    }
    Ok(Decoded::Skip)
}

fn decode_updates(updates: &Value, scales: Scales) -> Result<Vec<BookOp>, AdapterError> {
    // Stash order_statuses by oid so we can enrich `new` diffs with side/ts.
    let mut status_by_oid: std::collections::HashMap<OrderId, &Value> =
        std::collections::HashMap::new();
    if let Some(arr) = updates.get("order_statuses").and_then(Value::as_array) {
        for s in arr {
            if let Some(order) = s.get("order") {
                if let Some(oid) = order.get("oid").and_then(Value::as_u64) {
                    status_by_oid.insert(oid, s);
                }
            }
        }
    }

    let diffs = match updates.get("book_diffs").and_then(Value::as_array) {
        Some(d) => d,
        None => return Ok(Vec::new()),
    };
    let mut ops = Vec::with_capacity(diffs.len());
    for d in diffs {
        let oid = d
            .get("oid")
            .and_then(Value::as_u64)
            .ok_or(AdapterError::MissingField("book_diff.oid"))?;
        let raw = d
            .get("raw_book_diff")
            .ok_or(AdapterError::MissingField("raw_book_diff"))?;

        // "remove" is the plain string form.
        if raw.as_str() == Some("remove") {
            ops.push(BookOp::Remove(oid));
            continue;
        }
        let raw_obj = raw
            .as_object()
            .ok_or(AdapterError::MissingField("raw_book_diff{}"))?;

        if let Some(new_obj) = raw_obj.get("new") {
            // Build an Order by combining the diff (user, px) with the matching
            // order_status (side, timestamp). Size comes from the `new` payload.
            let sz_str = new_obj
                .get("sz")
                .and_then(Value::as_str)
                .ok_or(AdapterError::MissingField("new.sz"))?;
            let px_str = d
                .get("px")
                .and_then(Value::as_str)
                .ok_or(AdapterError::MissingField("book_diff.px"))?;
            let user_str = d
                .get("user")
                .and_then(Value::as_str)
                .ok_or(AdapterError::MissingField("book_diff.user"))?;
            let Some(status) = status_by_oid.get(&oid) else {
                // `new` without matching order_status — skip; adapter caller can
                // count these via stats to detect capture gaps.
                continue;
            };
            let order_obj = status
                .get("order")
                .ok_or(AdapterError::MissingField("order"))?;
            let side = parse_side(order_obj.get("side").and_then(Value::as_str).unwrap_or(""))?;
            let ts = order_obj
                .get("timestamp")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let wallet = WalletId::from_hex(user_str)
                .map_err(|_| AdapterError::BadWallet(user_str.into()))?;
            let price = parse_fixed(px_str, scales.price_digits)
                .ok_or_else(|| AdapterError::BadPrice(px_str.into()))?;
            let qty = parse_fixed(sz_str, scales.qty_digits)
                .ok_or_else(|| AdapterError::BadQty(sz_str.into()))?;
            ops.push(BookOp::Add(Order {
                id: oid,
                wallet,
                side,
                price,
                qty,
                ts,
            }));
            continue;
        }
        if let Some(upd) = raw_obj.get("update") {
            let new_sz = upd
                .get("newSz")
                .and_then(Value::as_str)
                .ok_or(AdapterError::MissingField("update.newSz"))?;
            let new_qty = parse_fixed(new_sz, scales.qty_digits)
                .ok_or_else(|| AdapterError::BadQty(new_sz.into()))?;
            ops.push(BookOp::UpdateSize { id: oid, new_qty });
            continue;
        }
        if let Some(m) = raw_obj.get("modified") {
            let sz = m
                .get("sz")
                .and_then(Value::as_str)
                .ok_or(AdapterError::MissingField("modified.sz"))?;
            let new_qty = parse_fixed(sz, scales.qty_digits)
                .ok_or_else(|| AdapterError::BadQty(sz.into()))?;
            ops.push(BookOp::AmendSize { id: oid, new_qty });
            continue;
        }
        // Unknown raw_book_diff shape — skip rather than abort the whole replay.
    }
    Ok(ops)
}

fn parse_order(
    o: &Value,
    default_side: Option<Side>,
    scales: Scales,
) -> Result<Order, AdapterError> {
    let oid = o
        .get("oid")
        .and_then(Value::as_u64)
        .ok_or(AdapterError::MissingField("oid"))?;
    let user_str = o
        .get("user")
        .and_then(Value::as_str)
        .ok_or(AdapterError::MissingField("user"))?;
    let side_str = o.get("side").and_then(Value::as_str);
    let side = match side_str {
        Some(s) => parse_side(s)?,
        None => default_side.ok_or(AdapterError::MissingField("side"))?,
    };
    let px_str = o
        .get("limitPx")
        .and_then(Value::as_str)
        .ok_or(AdapterError::MissingField("limitPx"))?;
    let sz_str = o
        .get("sz")
        .and_then(Value::as_str)
        .ok_or(AdapterError::MissingField("sz"))?;
    let ts = o.get("timestamp").and_then(Value::as_u64).unwrap_or(0);
    let wallet =
        WalletId::from_hex(user_str).map_err(|_| AdapterError::BadWallet(user_str.into()))?;
    let price = parse_fixed(px_str, scales.price_digits)
        .ok_or_else(|| AdapterError::BadPrice(px_str.into()))?;
    let qty = parse_fixed(sz_str, scales.qty_digits)
        .ok_or_else(|| AdapterError::BadQty(sz_str.into()))?;
    Ok(Order {
        id: oid,
        wallet,
        side,
        price,
        qty,
        ts,
    })
}

fn parse_side(s: &str) -> Result<Side, AdapterError> {
    match s {
        "B" => Ok(Side::Bid),
        "A" => Ok(Side::Ask),
        other => Err(AdapterError::BadSide(other.into())),
    }
}

/// Parse a decimal string into a u64 scaled by `10^scale_digits`. Truncates
/// toward zero if the fractional part has more digits than `scale_digits`.
pub fn parse_fixed(s: &str, scale_digits: u32) -> Option<u64> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes[0] == b'-' {
        return None;
    }
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    let int: u64 = int_part.parse().ok()?;
    let scale_mul = 10u64.checked_pow(scale_digits)?;
    let mut out = int.checked_mul(scale_mul)?;
    let mut frac = 0u64;
    let mut digits = 0u32;
    for &b in frac_part.as_bytes() {
        if digits >= scale_digits {
            break;
        }
        if !b.is_ascii_digit() {
            return None;
        }
        frac = frac * 10 + (b - b'0') as u64;
        digits += 1;
    }
    while digits < scale_digits {
        frac = frac.checked_mul(10)?;
        digits += 1;
    }
    out = out.checked_add(frac)?;
    Some(out)
}

/// Read an entire capture file into memory, pre-decoded.
pub fn load_capture(path: impl AsRef<Path>, scales: Scales) -> Result<Capture, AdapterError> {
    let file = File::open(path.as_ref())?;
    // Capture files are large (>1 GB) — wide buffer amortizes syscalls.
    let reader = BufReader::with_capacity(1 << 20, file);

    let mut snapshot = Vec::new();
    let mut updates: Vec<Vec<BookOp>> = Vec::new();
    let mut stats = CaptureStats::default();

    for line in reader.lines() {
        let line = line?;
        if line.is_empty() {
            continue;
        }
        stats.lines += 1;
        match decode_line(&line, scales)? {
            Decoded::Skip => {}
            Decoded::Snapshot(mut orders) => {
                snapshot.append(&mut orders);
            }
            Decoded::Updates(ops) => {
                for op in &ops {
                    match op {
                        BookOp::Add(_) => stats.adds += 1,
                        BookOp::Remove(_) => stats.removes += 1,
                        BookOp::UpdateSize { .. } => stats.size_updates += 1,
                        BookOp::AmendSize { .. } => stats.size_amends += 1,
                    }
                }
                stats.total_ops += ops.len();
                stats.updates_messages += 1;
                updates.push(ops);
            }
        }
    }

    Ok(Capture {
        snapshot,
        updates,
        stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_point_parser() {
        assert_eq!(parse_fixed("0", 6), Some(0));
        assert_eq!(parse_fixed("1", 6), Some(1_000_000));
        assert_eq!(parse_fixed("1.5", 6), Some(1_500_000));
        assert_eq!(parse_fixed("90057", 6), Some(90_057_000_000));
        assert_eq!(parse_fixed("75974.0", 6), Some(75_974_000_000));
        assert_eq!(parse_fixed("0.01073", 8), Some(1_073_000));
        // truncation of excess precision.
        assert_eq!(parse_fixed("1.1234567890", 6), Some(1_123_456));
        assert_eq!(parse_fixed("-1", 6), None);
        assert_eq!(parse_fixed("abc", 6), None);
    }

    #[test]
    fn decode_snapshot_line() {
        let line = r#"{"recv_ns":0,"seq":0,"kind":"Snapshot","msg":{"channel":"l4Book","data":{"Snapshot":{"coin":"BTC","height":1,"levels":[[{"user":"0xf9109ada2f73c62e9889b45453065f0d99260a2d","coin":"BTC","side":"B","limitPx":"100","sz":"0.5","oid":1,"timestamp":10}],[{"user":"0x13558be785661958932ceac35ba20de187275a42","coin":"BTC","side":"A","limitPx":"101","sz":"0.25","oid":2,"timestamp":11}]]}}}}"#;
        let dec = decode_line(line, Scales::BTC_DEFAULT).unwrap();
        match dec {
            Decoded::Snapshot(orders) => {
                assert_eq!(orders.len(), 2);
                assert_eq!(orders[0].id, 1);
                assert_eq!(orders[0].side, Side::Bid);
                assert_eq!(orders[0].price, 100_000_000);
                assert_eq!(orders[0].qty, 50_000_000);
                assert_eq!(orders[1].id, 2);
                assert_eq!(orders[1].side, Side::Ask);
            }
            _ => panic!("expected Snapshot"),
        }
    }

    #[test]
    fn decode_updates_line() {
        // One new (paired with open), one remove, one update, one modified.
        let line = r#"{"recv_ns":0,"seq":1,"kind":"Updates","msg":{"channel":"l4Book","data":{"Updates":{"time":1,"height":2,"order_statuses":[{"time":"t","user":"0xbc927e87d072dfac3693846a83fa6922cc6c5f2a","status":"open","order":{"user":null,"coin":"BTC","side":"B","limitPx":"90056.0","sz":"0.00014","oid":100,"timestamp":123}}],"book_diffs":[{"user":"0xbc927e87d072dfac3693846a83fa6922cc6c5f2a","oid":100,"px":"90056.0","coin":"BTC","raw_book_diff":{"new":{"sz":"0.00014"}}},{"user":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","oid":7,"px":"90055.0","coin":"BTC","raw_book_diff":"remove"},{"user":"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","oid":8,"px":"84.371","coin":"SOL","raw_book_diff":{"update":{"origSz":"108.65","newSz":"107.5"}}},{"user":"0xcccccccccccccccccccccccccccccccccccccccc","oid":9,"px":"1","coin":"X","raw_book_diff":{"modified":{"sz":"2"}}}]}}}}"#;
        let dec = decode_line(line, Scales::BTC_DEFAULT).unwrap();
        match dec {
            Decoded::Updates(ops) => {
                assert_eq!(ops.len(), 4);
                match &ops[0] {
                    BookOp::Add(o) => {
                        assert_eq!(o.id, 100);
                        assert_eq!(o.side, Side::Bid);
                        assert_eq!(o.qty, 14_000); // 0.00014 * 1e8
                    }
                    _ => panic!("expected Add"),
                }
                assert!(matches!(ops[1], BookOp::Remove(7)));
                match &ops[2] {
                    BookOp::UpdateSize { id: 8, new_qty } => assert_eq!(*new_qty, 10_750_000_000),
                    _ => panic!("expected UpdateSize"),
                }
                match &ops[3] {
                    BookOp::AmendSize { id: 9, new_qty } => assert_eq!(*new_qty, 200_000_000),
                    _ => panic!("expected AmendSize"),
                }
            }
            _ => panic!("expected Updates"),
        }
    }
}
