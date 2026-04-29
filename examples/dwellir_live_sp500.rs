//! Live Dwellir L4 example for `xyz:SP500`.
//!
//! Run with:
//! `DWELLIR_WS_ENDPOINT='wss://<your-dwellir-host>/<token>/ws' cargo run --release --example dwellir_live_sp500 --features dwellir`
//!
//! The example subscribes to Dwellir's `l4Book` feed, maintains an `OrderBook`,
//! and logs top-of-book detail every 15 seconds for 2 minutes after the initial
//! snapshot arrives.

use std::error::Error;
use std::time::{Duration, Instant};

use l4_book::dwellir::{BookOp, Decoded, Scales, decode_line};
use l4_book::{OrderBook, Price, Qty, Side, SlippageEstimate};
use serde_json::json;
use tungstenite::{Message, connect};

const SYMBOL: &str = "xyz:SP500";
const REPORT_EVERY: Duration = Duration::from_secs(15);
const RUN_FOR: Duration = Duration::from_secs(120);
const PRICE_DIGITS: u32 = 6;
const QTY_DIGITS: u32 = 8;
const NOTIONALS_USD: [u64; 2] = [1_000, 100_000];

fn main() -> Result<(), Box<dyn Error>> {
    let endpoint = std::env::var("DWELLIR_WS_ENDPOINT").map_err(
        |_| "DWELLIR_WS_ENDPOINT is required, e.g. wss://<your-dwellir-host>/<token>/ws",
    )?;
    let scales = Scales {
        price_digits: PRICE_DIGITS,
        qty_digits: QTY_DIGITS,
    };

    let subscription = json!({
        "method": "subscribe",
        "subscription": { "type": "l4Book", "coin": SYMBOL },
    });

    eprintln!("connecting to Dwellir endpoint from DWELLIR_WS_ENDPOINT");
    let (mut ws, response) = connect(endpoint.as_str())?;
    eprintln!("connected: HTTP {}", response.status());
    ws.send(Message::Text(subscription.to_string()))?;
    eprintln!("subscribed to l4Book coin={SYMBOL}");

    let mut book = OrderBook::new();
    let mut snapshot_at: Option<Instant> = None;
    let mut next_report: Option<Instant> = None;
    let mut deadline: Option<Instant> = None;
    let mut messages = 0u64;
    let mut updates = 0u64;
    let mut ops_applied = 0u64;
    let mut op_errors = 0u64;

    loop {
        let msg = ws.read()?;
        messages += 1;

        let text = match msg {
            Message::Text(text) => text,
            Message::Binary(bytes) => String::from_utf8(bytes)?,
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
            Message::Close(frame) => {
                eprintln!("websocket closed: {frame:?}");
                break;
            }
        };

        match decode_line(&text, scales)? {
            Decoded::Skip => {}
            Decoded::Snapshot(orders) => {
                let order_count = orders.len();
                book.apply_snapshot(orders)?;
                let now = Instant::now();
                snapshot_at = Some(now);
                next_report = Some(now + REPORT_EVERY);
                deadline = Some(now + RUN_FOR);
                eprintln!("snapshot applied: {order_count} orders; reporting every 15s for 2m");
            }
            Decoded::Updates(batch) => {
                updates += 1;
                let parsed_message = format!("{:#?}", batch.ops);
                for op in batch.ops {
                    let parsed_op = format!("{op:#?}");
                    match apply_op(&mut book, op) {
                        Ok(()) => ops_applied += 1,
                        Err(err) => {
                            op_errors += 1;
                            eprintln!(
                                "book op apply error #{op_errors}: {err}\nparsed op:\n{parsed_op}\nparsed message:\n{parsed_message}\nraw message:\n{text}"
                            );
                        }
                    }
                }
            }
        }

        let Some(end_at) = deadline else {
            continue;
        };
        let now = Instant::now();
        while let Some(report_at) = next_report {
            if now < report_at {
                break;
            }
            let elapsed = snapshot_at.map_or(0.0, |t| now.duration_since(t).as_secs_f64());
            log_report(
                &book,
                scales,
                elapsed,
                messages,
                updates,
                ops_applied,
                op_errors,
            );
            next_report = Some(report_at + REPORT_EVERY);
        }
        if now >= end_at {
            break;
        }
    }

    eprintln!(
        "done: messages={messages} updates={updates} ops_applied={ops_applied} op_errors={op_errors} len={} best_bid={} best_ask={}",
        book.len(),
        fmt_price_opt(book.best_bid(), scales),
        fmt_price_opt(book.best_ask(), scales),
    );
    Ok(())
}

fn apply_op(book: &mut OrderBook, op: BookOp) -> Result<(), l4_book::BookError> {
    match op {
        BookOp::Add(order) => book.add(order),
        BookOp::Remove(id) => book.remove(id).map(|_| ()),
        BookOp::UpdateSize { id, new_qty } => book.update_size(id, new_qty),
        BookOp::AmendSize { id, new_qty } => book.amend_size(id, new_qty),
    }
}

fn log_report(
    book: &OrderBook,
    scales: Scales,
    elapsed_s: f64,
    messages: u64,
    updates: u64,
    ops_applied: u64,
    op_errors: u64,
) {
    eprintln!(
        "\n=== t={elapsed_s:.1}s symbol={SYMBOL} messages={messages} updates={updates} ops_applied={ops_applied} op_errors={op_errors} len={} best_bid={} best_ask={} ===",
        book.len(),
        fmt_price_opt(book.best_bid(), scales),
        fmt_price_opt(book.best_ask(), scales),
    );

    let unaggregated = book.top_n_levels(5);
    eprintln!("unaggregated top 5 bids: {:#?}", unaggregated.bids);
    eprintln!("unaggregated top 5 asks: {:#?}", unaggregated.asks);

    let aggregated = book.top_n_levels_aggregated(5);
    eprintln!("aggregated top 5 bids:");
    for level in &aggregated.bids {
        eprintln!(
            "  px={} qty={} orders={}",
            fmt_scaled(level.price as u128, scales.price_digits),
            fmt_scaled(level.total_qty as u128, scales.qty_digits),
            level.order_count,
        );
    }
    eprintln!("aggregated top 5 asks:");
    for level in &aggregated.asks {
        eprintln!(
            "  px={} qty={} orders={}",
            fmt_scaled(level.price as u128, scales.price_digits),
            fmt_scaled(level.total_qty as u128, scales.qty_digits),
            level.order_count,
        );
    }

    for notional in NOTIONALS_USD {
        log_slippage(book, scales, Side::Bid, notional);
        log_slippage(book, scales, Side::Ask, notional);
    }
}

fn log_slippage(book: &OrderBook, scales: Scales, side: Side, notional_usd: u64) {
    let reference = match side {
        Side::Bid => book.best_ask(),
        Side::Ask => book.best_bid(),
    };
    let Some(reference_price) = reference else {
        eprintln!("slippage {side:?} ${notional_usd}: unavailable; missing opposite quote");
        return;
    };
    let Some(qty) = qty_for_notional(notional_usd, reference_price, scales) else {
        eprintln!("slippage {side:?} ${notional_usd}: unavailable; bad reference price");
        return;
    };
    let limit_price = match side {
        Side::Bid => Price::MAX,
        Side::Ask => 0,
    };
    let estimate = book.estimate_slippage(side, qty, limit_price);
    eprintln!(
        "slippage {} ${}: qty={} filled={} unfilled={} avg_px={} ref_px={} slip_px={} slip_notional={} slip_pct={} complete={} exhausted={}",
        match side {
            Side::Bid => "buy",
            Side::Ask => "sell",
        },
        notional_usd,
        fmt_scaled(qty as u128, scales.qty_digits),
        fmt_scaled(estimate.filled_qty as u128, scales.qty_digits),
        fmt_scaled(estimate.unfilled_qty as u128, scales.qty_digits),
        fmt_avg_price(&estimate, scales),
        estimate
            .reference_price
            .map(|p| fmt_scaled(p as u128, scales.price_digits))
            .unwrap_or_else(|| "n/a".to_string()),
        estimate
            .slippage
            .map(|v| format!("{:.6}", v / scale_f64(scales.price_digits)))
            .unwrap_or_else(|| "n/a".to_string()),
        estimate
            .slippage_notional
            .map(|n| fmt_scaled(n, scales.price_digits + scales.qty_digits))
            .unwrap_or_else(|| "n/a".to_string()),
        estimate
            .slippage_pct
            .map(|p| format!("{p:.6}%"))
            .unwrap_or_else(|| "n/a".to_string()),
        estimate.is_complete(),
        estimate.exhausted_book,
    );
}

fn qty_for_notional(notional_usd: u64, reference_price: Price, scales: Scales) -> Option<Qty> {
    if reference_price == 0 {
        return None;
    }
    let price_scale = 10u128.checked_pow(scales.price_digits)?;
    let qty_scale = 10u128.checked_pow(scales.qty_digits)?;
    let qty = (notional_usd as u128)
        .checked_mul(price_scale)?
        .checked_mul(qty_scale)?
        / reference_price as u128;
    Some(qty.min(Qty::MAX as u128) as Qty)
}

fn fmt_avg_price(estimate: &SlippageEstimate, scales: Scales) -> String {
    estimate
        .average_price
        .map(|p| format!("{:.6}", p / scale_f64(scales.price_digits)))
        .unwrap_or_else(|| "n/a".to_string())
}

fn fmt_price_opt(price: Option<Price>, scales: Scales) -> String {
    price
        .map(|p| fmt_scaled(p as u128, scales.price_digits))
        .unwrap_or_else(|| "n/a".to_string())
}

fn fmt_scaled(value: u128, digits: u32) -> String {
    if digits == 0 {
        return value.to_string();
    }
    let scale = 10u128.pow(digits);
    let int = value / scale;
    let frac = value % scale;
    let mut frac_s = format!("{frac:0width$}", width = digits as usize);
    while frac_s.ends_with('0') {
        frac_s.pop();
    }
    if frac_s.is_empty() {
        int.to_string()
    } else {
        format!("{int}.{frac_s}")
    }
}

fn scale_f64(digits: u32) -> f64 {
    10u64.pow(digits) as f64
}
