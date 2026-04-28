//! Criterion bench: snapshot apply + update-stream replay against a pre-decoded
//! Dwellir L4 capture. The JSON decode cost is paid once up front so we measure
//! only the book's processing speed.
//!
//! Point it at a capture via env var:
//!   L4_CAPTURE=benchmark_data/btc_l4_capture.jsonl cargo bench --features dwellir

use std::time::Instant;

use criterion::{BatchSize, Criterion, Throughput, black_box, criterion_group, criterion_main};
use l4_book::dwellir::{BookOp, Capture, Scales, load_capture};
use l4_book::{Order, OrderBook};

fn default_path() -> std::path::PathBuf {
    // cargo runs benches from the crate dir; the capture lives one level up.
    std::env::var_os("L4_CAPTURE")
        .map(Into::into)
        .unwrap_or_else(|| std::path::PathBuf::from("benchmark_data/btc_l4_capture.jsonl"))
}

fn load() -> Capture {
    let path = default_path();
    eprintln!("loading capture from {}", path.display());
    let t0 = Instant::now();
    let cap = load_capture(&path, Scales::BTC_DEFAULT).expect("load capture");
    eprintln!(
        "loaded in {:.2}s  snapshot_orders={}  updates_msgs={}  ops={} (add={} rm={} upd={} amend={})",
        t0.elapsed().as_secs_f64(),
        cap.snapshot.len(),
        cap.updates.len(),
        cap.stats.total_ops,
        cap.stats.adds,
        cap.stats.removes,
        cap.stats.size_updates,
        cap.stats.size_amends,
    );
    cap
}

fn apply_ops(book: &mut OrderBook, ops: &[BookOp]) {
    for op in ops {
        match op {
            BookOp::Add(o) => {
                let _ = book.add(*o);
            }
            BookOp::Remove(id) => {
                let _ = book.remove(*id);
            }
            BookOp::UpdateSize { id, new_qty } => {
                let _ = book.update_size(*id, *new_qty);
            }
            BookOp::AmendSize { id, new_qty } => {
                let _ = book.amend_size(*id, *new_qty);
            }
        }
    }
}

fn apply_snapshot(book: &mut OrderBook, snapshot: &[Order]) {
    book.apply_snapshot(snapshot.iter().copied())
        .expect("apply snapshot");
}

fn bench_snapshot(c: &mut Criterion, cap: &Capture) {
    let mut g = c.benchmark_group("snapshot");
    g.throughput(Throughput::Elements(cap.snapshot.len() as u64));
    g.sample_size(50);
    g.bench_function("apply", |b| {
        b.iter_batched_ref(
            || OrderBook::with_capacity(cap.snapshot.len() + 65_536),
            |book| {
                apply_snapshot(book, &cap.snapshot);
                black_box(&*book);
            },
            BatchSize::LargeInput,
        );
    });
    g.finish();
}

fn bench_stream_replay(c: &mut Criterion, cap: &Capture) {
    let total_ops = cap.stats.total_ops as u64;
    let mut g = c.benchmark_group("replay");
    g.throughput(Throughput::Elements(total_ops));
    g.sample_size(10);
    g.bench_function("stream_300s", |b| {
        b.iter_batched_ref(
            || {
                let mut book = OrderBook::with_capacity(cap.snapshot.len() + 65_536);
                apply_snapshot(&mut book, &cap.snapshot);
                book
            },
            |book| {
                for ops in &cap.updates {
                    apply_ops(book, ops);
                }
                black_box(&*book);
            },
            BatchSize::LargeInput,
        );
    });
    g.finish();
}

fn bench_per_update(c: &mut Criterion, cap: &Capture) {
    // Pick a representative middle-of-run update (skip warm-up and tail).
    let mid = cap.updates.len() / 2;
    let sample = cap.updates[mid].clone();
    let mut g = c.benchmark_group("per_update");
    g.throughput(Throughput::Elements(sample.len() as u64));
    // Build a pre-warmed book once; each iter applies `sample` then reverts by
    // resetting from a cloned pristine book. Cloning is expensive, so do it in
    // setup (iter_batched_ref).
    let warm = {
        let mut book = OrderBook::with_capacity(cap.snapshot.len() + 65_536);
        apply_snapshot(&mut book, &cap.snapshot);
        // Run up to mid-1 so state mirrors what the middle update will see.
        for ops in &cap.updates[..mid] {
            apply_ops(&mut book, ops);
        }
        book
    };
    g.sample_size(50);
    g.bench_function(format!("mid_update_{}_ops", sample.len()), |b| {
        b.iter_batched_ref(
            || warm_book(&cap.snapshot, &cap.updates[..mid]),
            |book| {
                apply_ops(book, &sample);
                black_box(&*book);
            },
            BatchSize::LargeInput,
        );
    });
    g.finish();
}

fn warm_book(snapshot: &[Order], updates: &[Vec<BookOp>]) -> OrderBook {
    let mut book = OrderBook::with_capacity(snapshot.len() + 65_536);
    apply_snapshot(&mut book, snapshot);
    for ops in updates {
        apply_ops(&mut book, ops);
    }
    book
}

fn bench_all(c: &mut Criterion) {
    let cap = load();
    bench_snapshot(c, &cap);
    bench_stream_replay(c, &cap);
    bench_per_update(c, &cap);
}

criterion_group!(benches, bench_all);
criterion_main!(benches);
