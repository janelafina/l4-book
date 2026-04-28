use l4_book::{BookError, Order, OrderBook, Side, WalletId};

fn w(n: u8) -> WalletId {
    let mut x = [0u8; 20];
    x[0] = n;
    WalletId(x)
}

fn o(id: u64, wallet: WalletId, side: Side, price: u64, qty: u64, ts: u64) -> Order {
    Order {
        id,
        wallet,
        side,
        price,
        qty,
        ts,
    }
}

#[test]
fn add_and_best_prices() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 5, 1)).unwrap();
    b.add(o(2, w(1), Side::Bid, 102, 3, 2)).unwrap();
    b.add(o(3, w(2), Side::Ask, 110, 7, 3)).unwrap();
    b.add(o(4, w(2), Side::Ask, 108, 2, 4)).unwrap();

    assert_eq!(b.best_bid(), Some(102));
    assert_eq!(b.best_ask(), Some(108));
    assert_eq!(b.len(), 4);

    let bids: Vec<_> = b.depth(Side::Bid).collect();
    assert_eq!(bids, vec![(102, 3, 1), (100, 5, 1)]);
    let asks: Vec<_> = b.depth(Side::Ask).collect();
    assert_eq!(asks, vec![(108, 2, 1), (110, 7, 1)]);

    b.assert_invariants();
}

#[test]
fn fifo_time_priority_at_level() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 5, 10)).unwrap();
    b.add(o(2, w(2), Side::Bid, 100, 3, 11)).unwrap();
    b.add(o(3, w(3), Side::Bid, 100, 1, 12)).unwrap();

    let ids: Vec<_> = b.orders_at(Side::Bid, 100).map(|o| o.id).collect();
    assert_eq!(ids, vec![1, 2, 3]);

    b.remove(2).unwrap();
    let ids: Vec<_> = b.orders_at(Side::Bid, 100).map(|o| o.id).collect();
    assert_eq!(ids, vec![1, 3]);

    b.assert_invariants();
}

#[test]
fn update_size_partial_fill() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 10, 1)).unwrap();
    b.update_size(1, 3).unwrap();

    assert_eq!(b.get(1).unwrap().qty, 3);
    assert_eq!(b.depth(Side::Bid).next(), Some((100, 3, 1)));

    // non-decreasing update rejected
    assert_eq!(
        b.update_size(1, 3),
        Err(BookError::NonDecreasingSize {
            current: 3,
            proposed: 3
        }),
    );
    assert_eq!(
        b.update_size(1, 5),
        Err(BookError::NonDecreasingSize {
            current: 3,
            proposed: 5
        }),
    );

    // zero removes
    b.update_size(1, 0).unwrap();
    assert!(b.get(1).is_none());
    assert!(b.is_empty());
    b.assert_invariants();
}

#[test]
fn amend_size_either_direction() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 10, 1)).unwrap();
    b.amend_size(1, 25).unwrap();
    assert_eq!(b.get(1).unwrap().qty, 25);
    assert_eq!(b.depth(Side::Bid).next(), Some((100, 25, 1)));

    b.amend_size(1, 5).unwrap();
    assert_eq!(b.depth(Side::Bid).next(), Some((100, 5, 1)));

    b.amend_size(1, 0).unwrap();
    assert!(b.is_empty());
    b.assert_invariants();
}

#[test]
fn remove_empty_level_is_pruned() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 5, 1)).unwrap();
    b.add(o(2, w(1), Side::Bid, 101, 2, 2)).unwrap();
    b.remove(1).unwrap();
    assert_eq!(b.depth(Side::Bid).collect::<Vec<_>>(), vec![(101, 2, 1)]);
    assert_eq!(b.best_bid(), Some(101));
    b.assert_invariants();
}

#[test]
fn duplicate_and_unknown_errors() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 5, 1)).unwrap();
    assert_eq!(
        b.add(o(1, w(2), Side::Ask, 99, 1, 2)),
        Err(BookError::DuplicateOrderId(1))
    );
    assert_eq!(b.remove(999), Err(BookError::UnknownOrderId(999)));
    assert_eq!(
        b.add(o(2, w(1), Side::Bid, 100, 0, 1)),
        Err(BookError::ZeroQty)
    );
    b.assert_invariants();
}

#[test]
fn wallet_attribution() {
    let mut b = OrderBook::new();
    let alice = w(1);
    let bob = w(2);
    b.add(o(1, alice, Side::Bid, 100, 5, 1)).unwrap();
    b.add(o(2, alice, Side::Ask, 110, 2, 2)).unwrap();
    b.add(o(3, bob, Side::Bid, 99, 3, 3)).unwrap();

    let mut alice_ids: Vec<_> = b.orders_by_wallet(alice).map(|o| o.id).collect();
    alice_ids.sort();
    assert_eq!(alice_ids, vec![1, 2]);

    let bob_ids: Vec<_> = b.orders_by_wallet(bob).map(|o| o.id).collect();
    assert_eq!(bob_ids, vec![3]);

    // after removing all of alice's orders her entry is gone.
    b.remove(1).unwrap();
    b.remove(2).unwrap();
    assert_eq!(b.orders_by_wallet(alice).count(), 0);
    b.assert_invariants();
}

#[test]
fn snapshot_replaces_state() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 5, 1)).unwrap();
    b.apply_snapshot([
        o(10, w(3), Side::Bid, 200, 1, 1),
        o(11, w(3), Side::Ask, 210, 2, 2),
    ])
    .unwrap();

    assert!(b.get(1).is_none());
    assert_eq!(b.best_bid(), Some(200));
    assert_eq!(b.best_ask(), Some(210));
    assert_eq!(b.len(), 2);
    b.assert_invariants();
}

#[test]
fn free_list_reuses_slots() {
    let mut b = OrderBook::new();
    for i in 0..100 {
        b.add(o(i, w(1), Side::Bid, 100 + i, 1, i)).unwrap();
    }
    assert_eq!(b.slab_len(), 100);
    for i in 0..100 {
        b.remove(i).unwrap();
    }
    // Reinserting should recycle the freed slots rather than grow the slab.
    for i in 1000..1050 {
        b.add(o(i, w(2), Side::Ask, 100 + (i % 10), 1, i)).unwrap();
    }
    assert_eq!(b.slab_len(), 100, "free-list reuse not occurring");
    b.assert_invariants();
}

#[test]
fn wallet_hex_parsing() {
    let w = WalletId::from_hex("0xf9109ada2f73c62e9889b45453065f0d99260a2d").unwrap();
    assert_eq!(
        format!("{w:?}"),
        "0xf9109ada2f73c62e9889b45453065f0d99260a2d"
    );
    assert!(WalletId::from_hex("0x1234").is_err());
    assert!(WalletId::from_hex("zzzz9ada2f73c62e9889b45453065f0d99260a2d").is_err());
}

#[test]
fn top_n_levels_unaggregated_preserves_price_and_fifo_priority() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 5, 1)).unwrap();
    b.add(o(2, w(2), Side::Bid, 101, 3, 2)).unwrap();
    b.add(o(3, w(3), Side::Bid, 101, 2, 3)).unwrap();
    b.add(o(4, w(4), Side::Bid, 99, 4, 4)).unwrap();
    b.add(o(5, w(5), Side::Ask, 105, 1, 5)).unwrap();
    b.add(o(6, w(6), Side::Ask, 105, 7, 6)).unwrap();
    b.add(o(7, w(7), Side::Ask, 106, 8, 7)).unwrap();
    b.add(o(8, w(8), Side::Ask, 107, 9, 8)).unwrap();

    let top = b.top_n_levels(2);

    assert_eq!(
        top.bids.iter().map(|l| l.price).collect::<Vec<_>>(),
        vec![101, 100]
    );
    assert_eq!(
        top.bids[0].orders.iter().map(|o| o.id).collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert_eq!(
        top.bids[0]
            .orders
            .iter()
            .map(|o| o.wallet)
            .collect::<Vec<_>>(),
        vec![w(2), w(3)]
    );
    assert_eq!(
        top.bids[0].orders.iter().map(|o| o.qty).collect::<Vec<_>>(),
        vec![3, 2]
    );

    assert_eq!(
        top.asks.iter().map(|l| l.price).collect::<Vec<_>>(),
        vec![105, 106]
    );
    assert_eq!(
        top.asks[0].orders.iter().map(|o| o.id).collect::<Vec<_>>(),
        vec![5, 6]
    );
}

#[test]
fn top_n_levels_aggregated_returns_named_depth() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 5, 1)).unwrap();
    b.add(o(2, w(2), Side::Bid, 101, 3, 2)).unwrap();
    b.add(o(3, w(3), Side::Bid, 101, 2, 3)).unwrap();
    b.add(o(4, w(4), Side::Ask, 105, 1, 4)).unwrap();
    b.add(o(5, w(5), Side::Ask, 105, 7, 5)).unwrap();
    b.add(o(6, w(6), Side::Ask, 106, 8, 6)).unwrap();

    let top = b.top_n_levels_aggregated(1);

    assert_eq!(top.bids.len(), 1);
    assert_eq!(top.bids[0].price, 101);
    assert_eq!(top.bids[0].total_qty, 5);
    assert_eq!(top.bids[0].order_count, 2);
    assert_eq!(top.asks.len(), 1);
    assert_eq!(top.asks[0].price, 105);
    assert_eq!(top.asks[0].total_qty, 8);
    assert_eq!(top.asks[0].order_count, 2);
}

#[test]
fn best_bid_ask_returns_both_sides() {
    let mut b = OrderBook::new();
    assert_eq!(b.best_bid_ask(), (None, None));

    b.add(o(1, w(1), Side::Bid, 100, 5, 1)).unwrap();
    assert_eq!(b.best_bid_ask(), (Some(100), None));

    b.add(o(2, w(2), Side::Ask, 105, 2, 2)).unwrap();
    assert_eq!(b.best_bid_ask(), (Some(100), Some(105)));
}

#[test]
fn slippage_buy_walks_asks_against_limit() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Ask, 101, 2, 1)).unwrap();
    b.add(o(2, w(2), Side::Ask, 103, 3, 2)).unwrap();
    b.add(o(3, w(3), Side::Ask, 105, 5, 3)).unwrap();

    let s = b.estimate_slippage(Side::Bid, 4, 103);

    assert!(s.is_complete());
    assert_eq!(s.reference_price, Some(101));
    assert_eq!(s.filled_qty, 4);
    assert_eq!(s.unfilled_qty, 0);
    assert_eq!(s.filled_notional, 408);
    assert_eq!(s.average_price, Some(102.0));
    assert_eq!(s.slippage, Some(1.0));
    assert_eq!(s.slippage_notional, Some(4));
    assert!((s.slippage_pct.unwrap() - (1.0 / 101.0 * 100.0)).abs() < 1e-12);
    assert!(!s.limit_stopped);
    assert!(!s.exhausted_book);
}

#[test]
fn slippage_sell_walks_bids_and_reports_limit_stop() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 99, 4, 1)).unwrap();
    b.add(o(2, w(2), Side::Bid, 97, 5, 2)).unwrap();

    let s = b.estimate_slippage(Side::Ask, 5, 98);

    assert!(!s.is_complete());
    assert_eq!(s.reference_price, Some(99));
    assert_eq!(s.filled_qty, 4);
    assert_eq!(s.unfilled_qty, 1);
    assert_eq!(s.filled_notional, 396);
    assert_eq!(s.average_price, Some(99.0));
    assert_eq!(s.slippage, Some(0.0));
    assert_eq!(s.slippage_notional, Some(0));
    assert_eq!(s.slippage_pct, Some(0.0));
    assert!(s.limit_stopped);
    assert!(!s.exhausted_book);
}

#[test]
fn slippage_reports_exhausted_book_separately_from_limit_stop() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Ask, 101, 2, 1)).unwrap();
    b.add(o(2, w(2), Side::Ask, 103, 3, 2)).unwrap();

    let s = b.estimate_slippage(Side::Bid, 10, 200);

    assert!(!s.is_complete());
    assert_eq!(s.filled_qty, 5);
    assert_eq!(s.unfilled_qty, 5);
    assert_eq!(s.filled_notional, 511);
    assert_eq!(s.average_price, Some(102.2));
    assert!((s.slippage.unwrap() - 1.2).abs() < 1e-12);
    assert_eq!(s.slippage_notional, Some(6));
    assert!(!s.limit_stopped);
    assert!(s.exhausted_book);
}

#[test]
fn slippage_no_fill_when_limit_is_not_marketable() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Ask, 101, 2, 1)).unwrap();

    let s = b.estimate_slippage(Side::Bid, 2, 100);

    assert!(!s.is_complete());
    assert_eq!(s.reference_price, Some(101));
    assert_eq!(s.filled_qty, 0);
    assert_eq!(s.unfilled_qty, 2);
    assert_eq!(s.filled_notional, 0);
    assert_eq!(s.average_price, None);
    assert_eq!(s.slippage, None);
    assert_eq!(s.slippage_notional, None);
    assert_eq!(s.slippage_pct, None);
    assert!(s.limit_stopped);
    assert!(!s.exhausted_book);
}
