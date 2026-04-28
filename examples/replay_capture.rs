//! Sanity-check the full pipeline: parse the capture, apply snapshot + all
//! updates, and print stats. Not a bench — wall-clock only.

use std::time::Instant;

use l4_book::dwellir::{BookOp, Scales, load_capture};
use l4_book::{OrderBook, Side};

fn main() {
    let path = std::env::var("L4_CAPTURE")
        .unwrap_or_else(|_| "benchmark_data/btc_l4_capture.jsonl".to_string());

    let t0 = Instant::now();
    let cap = load_capture(&path, Scales::BTC_DEFAULT).expect("load capture");
    let load_s = t0.elapsed().as_secs_f64();
    println!(
        "loaded: {:.2}s  snapshot={}  updates={}  ops={} (add={} rm={} upd={} amend={})",
        load_s,
        cap.snapshot.len(),
        cap.updates.len(),
        cap.stats.total_ops,
        cap.stats.adds,
        cap.stats.removes,
        cap.stats.size_updates,
        cap.stats.size_amends,
    );

    let mut book = OrderBook::with_capacity(cap.snapshot.len() + 65_536);
    let t1 = Instant::now();
    book.apply_snapshot(cap.snapshot.iter().copied())
        .expect("snapshot");
    let snap_s = t1.elapsed().as_secs_f64();
    println!(
        "snapshot applied: {:.3}s  ({} orders, {:.0} orders/s)",
        snap_s,
        cap.snapshot.len(),
        cap.snapshot.len() as f64 / snap_s,
    );
    #[cfg(debug_assertions)]
    book.assert_invariants();
    println!(
        "  best_bid={:?}  best_ask={:?}  spread={:?}",
        book.best_bid(),
        book.best_ask(),
        book.best_ask()
            .and_then(|a| book.best_bid().map(|b| a.saturating_sub(b)))
    );

    let t2 = Instant::now();
    let mut errs_add = 0u64;
    let mut errs_rm = 0u64;
    let mut errs_upd = 0u64;
    let mut errs_amend = 0u64;
    for ops in &cap.updates {
        for op in ops {
            match op {
                BookOp::Add(o) => {
                    if book.add(*o).is_err() {
                        errs_add += 1
                    }
                }
                BookOp::Remove(id) => {
                    if book.remove(*id).is_err() {
                        errs_rm += 1
                    }
                }
                BookOp::UpdateSize { id, new_qty } => {
                    if book.update_size(*id, *new_qty).is_err() {
                        errs_upd += 1
                    }
                }
                BookOp::AmendSize { id, new_qty } => {
                    if book.amend_size(*id, *new_qty).is_err() {
                        errs_amend += 1
                    }
                }
            }
        }
    }
    let stream_s = t2.elapsed().as_secs_f64();
    println!(
        "stream applied: {:.3}s  ({} ops, {:.0} ops/s)",
        stream_s,
        cap.stats.total_ops,
        cap.stats.total_ops as f64 / stream_s,
    );
    println!(
        "  errors: add={} remove={} update_size={} amend={} (typically zero when stream is consistent)",
        errs_add, errs_rm, errs_upd, errs_amend,
    );
    println!(
        "  final book: len={}  bids_levels={}  asks_levels={}  best_bid={:?}  best_ask={:?}",
        book.len(),
        book.depth(Side::Bid).count(),
        book.depth(Side::Ask).count(),
        book.best_bid(),
        book.best_ask(),
    );
    #[cfg(debug_assertions)]
    book.assert_invariants();
}
