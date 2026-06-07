//! Sanity-check the full pipeline: parse the capture, apply snapshot + all
//! updates, and print stats. Not a bench — wall-clock only.

use std::time::Instant;

use l4_book::dwellir::{Scales, load_capture};
use l4_book::{OrderBook, ReplayApplyOutcome, ReplayPolicy, Side};

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

    let strict = std::env::var("L4_REPLAY_STRICT")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);
    let policy = if strict {
        ReplayPolicy::STRICT
    } else {
        ReplayPolicy::DWELLIR_TOLERANT
    };
    println!(
        "replay policy: {}",
        if strict { "strict" } else { "dwellir_tolerant" }
    );

    let t2 = Instant::now();
    let mut applied = 0u64;
    let mut skipped = 0u64;
    let mut coerced = 0u64;
    let mut errors = 0u64;
    let mut anomaly_samples = Vec::new();
    for ops in &cap.updates {
        for op in ops {
            let outcome = book.apply_op_with_policy(*op, policy);
            match &outcome {
                ReplayApplyOutcome::Applied { .. } => applied += 1,
                ReplayApplyOutcome::Skipped { .. } => skipped += 1,
                ReplayApplyOutcome::Coerced { .. } => coerced += 1,
                ReplayApplyOutcome::Error { .. } => errors += 1,
            }
            if anomaly_samples.len() < 5 && !matches!(outcome, ReplayApplyOutcome::Applied { .. }) {
                anomaly_samples.push(format!("{outcome:?}"));
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
        "  replay outcomes: applied={} skipped={} coerced={} errors={} (strict errors should be zero when stream is consistent)",
        applied, skipped, coerced, errors,
    );
    if !anomaly_samples.is_empty() {
        println!("  first replay anomalies:");
        for sample in anomaly_samples {
            println!("    {sample}");
        }
    }
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
