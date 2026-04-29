//! Live Dwellir L4 debug logger for `xyz:SP500`.
//!
//! Run with:
//! `DWELLIR_WS_ENDPOINT='wss://<your-dwellir-host>/<token>/ws' cargo run --release --example dwellir_debug_log_sp500 --features dwellir`
//!
//! Optional environment variables:
//! * `DWELLIR_DEBUG_UPDATES` — number of initial update messages to log with
//!   raw text, parsed ops, and full post-update snapshots. Defaults to `5`.
//! * `DWELLIR_DEBUG_LOG` — output path. Defaults to
//!   `dwellir_sp500_debug_log.json`.
//!
//! The log file is written as newline-separated pretty JSON objects so it stays
//! useful even if the process is interrupted.

use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};

use l4_book::dwellir::{BookOp, Decoded, Scales, decode_line};
use l4_book::{Order, OrderBook, Side};
use serde_json::{Value, json};
use tungstenite::{Message, connect};

const SYMBOL: &str = "xyz:SP500";
const PRICE_DIGITS: u32 = 6;
const QTY_DIGITS: u32 = 8;
const ERROR_LIMIT: usize = 10;

fn main() -> Result<(), Box<dyn Error>> {
    let endpoint = std::env::var("DWELLIR_WS_ENDPOINT").map_err(
        |_| "DWELLIR_WS_ENDPOINT is required, e.g. wss://<your-dwellir-host>/<token>/ws",
    )?;
    let updates_to_log = env_usize("DWELLIR_DEBUG_UPDATES", 5);
    let log_path = std::env::var("DWELLIR_DEBUG_LOG")
        .unwrap_or_else(|_| "dwellir_sp500_debug_log.json".to_string());
    let scales = Scales {
        price_digits: PRICE_DIGITS,
        qty_digits: QTY_DIGITS,
    };

    let mut log = PrettyJsonLog::create(&log_path)?;
    log.write(&json!({
        "event": "start",
        "symbol": SYMBOL,
        "updates_to_log": updates_to_log,
        "error_limit": ERROR_LIMIT,
        "price_digits": PRICE_DIGITS,
        "qty_digits": QTY_DIGITS,
    }))?;

    let subscription = json!({
        "method": "subscribe",
        "subscription": { "type": "l4Book", "coin": SYMBOL },
    });

    eprintln!("logging debug feed events to {log_path}");
    eprintln!("connecting to Dwellir endpoint from DWELLIR_WS_ENDPOINT");
    let (mut ws, response) = connect(endpoint.as_str())?;
    eprintln!("connected: HTTP {}", response.status());
    ws.send(Message::Text(subscription.to_string()))?;
    eprintln!("subscribed to l4Book coin={SYMBOL}");

    let mut book = OrderBook::new();
    let mut messages = 0usize;
    let mut update_messages = 0usize;
    let mut ops_applied = 0usize;
    let mut errors = 0usize;
    let mut collapsed_complete_fills = 0usize;
    let mut dropped_duplicate_removes = 0usize;

    loop {
        let msg = match ws.read() {
            Ok(msg) => msg,
            Err(err) => {
                errors += 1;
                log.write(&json!({
                    "event": "websocket_read_error",
                    "error_index": errors,
                    "error": err.to_string(),
                }))?;
                break;
            }
        };
        messages += 1;

        let text = match msg {
            Message::Text(text) => text,
            Message::Binary(bytes) => match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(err) => {
                    errors += 1;
                    log.write(&json!({
                        "event": "binary_utf8_error",
                        "error_index": errors,
                        "error": err.to_string(),
                    }))?;
                    if errors >= ERROR_LIMIT {
                        break;
                    }
                    continue;
                }
            },
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
            Message::Close(frame) => {
                log.write(&json!({
                    "event": "websocket_closed",
                    "frame": format!("{frame:?}"),
                    "messages": messages,
                    "update_messages": update_messages,
                    "ops_applied": ops_applied,
                    "errors": errors,
                }))?;
                break;
            }
        };

        let decoded = match decode_line(&text, scales) {
            Ok(decoded) => decoded,
            Err(err) => {
                errors += 1;
                log.write(&json!({
                    "event": "decode_error",
                    "error_index": errors,
                    "error": err.to_string(),
                    "raw_message": text,
                }))?;
                if errors >= ERROR_LIMIT {
                    break;
                }
                continue;
            }
        };

        match decoded {
            Decoded::Skip => {}
            Decoded::Snapshot(orders) => {
                let order_count = orders.len();
                match book.apply_snapshot(orders) {
                    Ok(()) => {
                        log_marker(
                            &mut log,
                            format!(
                                "FULL SNAPSHOT INITIAL messages={messages} orders={order_count}"
                            ),
                        )?;
                        log.write(&json!({
                            "event": "full_snapshot",
                            "marker": "FULL SNAPSHOT INITIAL",
                            "messages": messages,
                            "order_count": order_count,
                            "book": book_snapshot(&book, scales),
                        }))?;
                        eprintln!("snapshot applied: {order_count} orders");
                    }
                    Err(err) => {
                        errors += 1;
                        let marker =
                            format!("ERROR snapshot_apply error_index={errors} error={err}");
                        log_marker(&mut log, marker.clone())?;
                        log.write(&json!({
                            "event": "snapshot_apply_error",
                            "marker": marker,
                            "error_index": errors,
                            "error": err.to_string(),
                            "raw_message": text,
                        }))?;
                    }
                }
            }
            Decoded::Updates(batch) => {
                update_messages += 1;
                collapsed_complete_fills += batch.collapsed_complete_fills;
                dropped_duplicate_removes += batch.dropped_duplicate_removes;
                let ops = batch.ops;
                let should_log_update = update_messages <= updates_to_log;
                let parsed_update = ops_to_value(&ops);
                let mut op_results = Vec::with_capacity(ops.len());

                if should_log_update {
                    log_marker(
                        &mut log,
                        format!(
                            "RAW UPDATE update={update_messages} message={messages} ops={} collapsed_complete_fills={} dropped_duplicate_removes={}",
                            ops.len(),
                            batch.collapsed_complete_fills,
                            batch.dropped_duplicate_removes,
                        ),
                    )?;
                    log.write(&json!({
                        "event": "raw_update",
                        "marker": format!("RAW UPDATE update={update_messages}"),
                        "update_index": update_messages,
                        "messages": messages,
                        "raw_update": text,
                        "parsed_update": parsed_update,
                        "collapsed_complete_fills": batch.collapsed_complete_fills,
                        "dropped_duplicate_removes": batch.dropped_duplicate_removes,
                    }))?;
                }

                for (op_index, op) in ops.iter().enumerate() {
                    let parsed_op = book_op_to_value(op);
                    let order_id = op_order_id(op);
                    let op_kind = op_kind(op);
                    match apply_op(&mut book, op.clone()) {
                        Ok(()) => {
                            ops_applied += 1;
                            let marker = format!(
                                "APPLIED BOOK DIFF update={update_messages} op_index={op_index} kind={op_kind} order_id={order_id}"
                            );
                            log.write(&json!({
                                "event": "applied_book_diff",
                                "marker": marker,
                                "update_index": update_messages,
                                "op_index": op_index,
                                "kind": op_kind,
                                "order_id": order_id,
                                "op": parsed_op,
                            }))?;
                            if should_log_update {
                                op_results.push(json!({
                                    "op_index": op_index,
                                    "status": "applied",
                                    "kind": op_kind,
                                    "order_id": order_id,
                                    "op": parsed_op,
                                }));
                            }
                        }
                        Err(err) => {
                            errors += 1;
                            let marker = format!(
                                "ERROR update={update_messages} op_index={op_index} kind={op_kind} order_id={order_id} error={err}"
                            );
                            log_marker(&mut log, marker.clone())?;
                            let snapshot_marker = format!(
                                "FULL SNAPSHOT AS OF ERROR update={update_messages} op_index={op_index} order_id={order_id}"
                            );
                            log_marker(&mut log, snapshot_marker.clone())?;
                            let error_record = json!({
                                "event": "op_apply_error",
                                "marker": marker,
                                "error_index": errors,
                                "update_index": update_messages,
                                "op_index": op_index,
                                "kind": op_kind,
                                "order_id": order_id,
                                "error": err.to_string(),
                                "raw_update": text,
                                "parsed_update": parsed_update,
                                "parsed_op": parsed_op,
                                "snapshot_marker": snapshot_marker,
                                "book_after_failed_op": book_snapshot(&book, scales),
                            });
                            log.write(&error_record)?;

                            if should_log_update {
                                op_results.push(json!({
                                    "op_index": op_index,
                                    "status": "error",
                                    "kind": op_kind,
                                    "order_id": order_id,
                                    "error": err.to_string(),
                                    "op": parsed_op,
                                }));
                            }

                            if errors >= ERROR_LIMIT {
                                break;
                            }
                        }
                    }
                }

                if should_log_update {
                    log.write(&json!({
                        "event": "parsed_update_summary",
                        "marker": format!("PARSED UPDATE SUMMARY update={update_messages}"),
                        "update_index": update_messages,
                        "messages": messages,
                        "op_results": op_results,
                    }))?;
                    let snapshot_marker = format!("FULL SNAPSHOT AS OF UPDATE {update_messages}");
                    log_marker(&mut log, snapshot_marker.clone())?;
                    log.write(&json!({
                        "event": "full_snapshot",
                        "marker": snapshot_marker,
                        "update_index": update_messages,
                        "messages": messages,
                        "book": book_snapshot(&book, scales),
                    }))?;
                }
            }
        }

        if errors >= ERROR_LIMIT {
            break;
        }
    }

    log.write(&json!({
        "event": "done",
        "messages": messages,
        "update_messages": update_messages,
        "ops_applied": ops_applied,
        "errors": errors,
        "collapsed_complete_fills": collapsed_complete_fills,
        "dropped_duplicate_removes": dropped_duplicate_removes,
        "book_len": book.len(),
        "best_bid": book.best_bid().map(|p| fmt_scaled(p as u128, scales.price_digits)),
        "best_ask": book.best_ask().map(|p| fmt_scaled(p as u128, scales.price_digits)),
    }))?;
    log.flush()?;

    eprintln!(
        "done: messages={messages} updates={update_messages} ops_applied={ops_applied} errors={errors} collapsed_complete_fills={collapsed_complete_fills} dropped_duplicate_removes={dropped_duplicate_removes} log={log_path}"
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

fn book_snapshot(book: &OrderBook, scales: Scales) -> Value {
    json!({
        "len": book.len(),
        "best_bid": book.best_bid().map(|p| fmt_scaled(p as u128, scales.price_digits)),
        "best_ask": book.best_ask().map(|p| fmt_scaled(p as u128, scales.price_digits)),
        "bids": side_snapshot(book, Side::Bid, scales),
        "asks": side_snapshot(book, Side::Ask, scales),
    })
}

fn side_snapshot(book: &OrderBook, side: Side, scales: Scales) -> Value {
    let levels: Vec<Value> = book
        .depth(side)
        .map(|(price, total_qty, order_count)| {
            let orders: Vec<Value> = book
                .orders_at(side, price)
                .map(|order| order_to_value(order, scales))
                .collect();
            json!({
                "price": fmt_scaled(price as u128, scales.price_digits),
                "raw_price": price,
                "total_qty": fmt_scaled(total_qty as u128, scales.qty_digits),
                "raw_total_qty": total_qty,
                "order_count": order_count,
                "orders": orders,
            })
        })
        .collect();
    Value::Array(levels)
}

fn order_to_value(order: &Order, scales: Scales) -> Value {
    json!({
        "id": order.id,
        "wallet": format!("{:?}", order.wallet),
        "side": side_name(order.side),
        "price": fmt_scaled(order.price as u128, scales.price_digits),
        "raw_price": order.price,
        "qty": fmt_scaled(order.qty as u128, scales.qty_digits),
        "raw_qty": order.qty,
        "ts": order.ts,
    })
}

fn ops_to_value(ops: &[BookOp]) -> Value {
    Value::Array(ops.iter().map(book_op_to_value).collect())
}

fn book_op_to_value(op: &BookOp) -> Value {
    match op {
        BookOp::Add(order) => json!({
            "type": "add",
            "order": {
                "id": order.id,
                "wallet": format!("{:?}", order.wallet),
                "side": side_name(order.side),
                "raw_price": order.price,
                "raw_qty": order.qty,
                "ts": order.ts,
            },
        }),
        BookOp::Remove(id) => json!({
            "type": "remove",
            "id": id,
        }),
        BookOp::UpdateSize { id, new_qty } => json!({
            "type": "update_size",
            "id": id,
            "raw_new_qty": new_qty,
        }),
        BookOp::AmendSize { id, new_qty } => json!({
            "type": "amend_size",
            "id": id,
            "raw_new_qty": new_qty,
        }),
    }
}

fn op_kind(op: &BookOp) -> &'static str {
    match op {
        BookOp::Add(_) => "add",
        BookOp::Remove(_) => "remove",
        BookOp::UpdateSize { .. } => "update_size",
        BookOp::AmendSize { .. } => "amend_size",
    }
}

fn op_order_id(op: &BookOp) -> u64 {
    match op {
        BookOp::Add(order) => order.id,
        BookOp::Remove(id) => *id,
        BookOp::UpdateSize { id, .. } | BookOp::AmendSize { id, .. } => *id,
    }
}

fn log_marker(log: &mut PrettyJsonLog, marker: String) -> Result<(), Box<dyn Error>> {
    log.write(&json!({
        "event": "grep_marker",
        "marker": marker,
    }))
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Bid => "bid",
        Side::Ask => "ask",
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
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

struct PrettyJsonLog {
    writer: BufWriter<File>,
}

impl PrettyJsonLog {
    fn create(path: &str) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            writer: BufWriter::new(File::create(path)?),
        })
    }

    fn write(&mut self, value: &Value) -> Result<(), Box<dyn Error>> {
        serde_json::to_writer_pretty(&mut self.writer, value)?;
        self.writer.write_all(b"\n\n")?;
        self.writer.flush()?;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Box<dyn Error>> {
        self.writer.flush()?;
        Ok(())
    }
}
