use l4_book::{
    AmendPriorityPolicy, BookError, BookOp, FillDelta, LimitOrderPolicy, OperationCause, Order,
    OrderBook, QueuePosition, ReasonedBookOp, ReplayApplyOutcome, ReplayPolicy,
    SYNTHETIC_ORDER_ID_FLAG, Side, SimulatorCause, SnapshotOutcome, SnapshotPolicy,
    SubmitRejectReason, ToleratedReplayReason, WalletId, is_synthetic_order_id, synthetic_order_id,
};

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
fn queue_position_reports_fifo_counts_for_live_orders() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 5, 1)).unwrap();
    b.add(o(2, w(2), Side::Bid, 100, 3, 2)).unwrap();
    b.add(o(3, w(3), Side::Bid, 100, 7, 3)).unwrap();

    assert_eq!(
        b.queue_position(1).unwrap(),
        QueuePosition {
            id: Some(1),
            side: Side::Bid,
            price: 100,
            orders_ahead: 0,
            qty_ahead: 0,
            orders_behind: 2,
            qty_behind: 10,
            level_order_count: 3,
            level_total_qty: 15,
        }
    );
    assert_eq!(
        b.queue_position(2).unwrap(),
        QueuePosition {
            id: Some(2),
            side: Side::Bid,
            price: 100,
            orders_ahead: 1,
            qty_ahead: 5,
            orders_behind: 1,
            qty_behind: 7,
            level_order_count: 3,
            level_total_qty: 15,
        }
    );
    assert_eq!(
        b.queue_position(3).unwrap(),
        QueuePosition {
            id: Some(3),
            side: Side::Bid,
            price: 100,
            orders_ahead: 2,
            qty_ahead: 8,
            orders_behind: 0,
            qty_behind: 0,
            level_order_count: 3,
            level_total_qty: 15,
        }
    );
    assert_eq!(b.queue_position(999), Err(BookError::UnknownOrderId(999)));
    b.assert_invariants();
}

#[test]
fn queue_position_for_new_order_joins_tail() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Ask, 105, 2, 1)).unwrap();
    b.add(o(2, w(2), Side::Ask, 105, 4, 2)).unwrap();

    assert_eq!(
        b.queue_position_for_new_order(Side::Ask, 105),
        QueuePosition {
            id: None,
            side: Side::Ask,
            price: 105,
            orders_ahead: 2,
            qty_ahead: 6,
            orders_behind: 0,
            qty_behind: 0,
            level_order_count: 2,
            level_total_qty: 6,
        }
    );
    assert_eq!(
        b.queue_position_for_new_order(Side::Ask, 106),
        QueuePosition {
            id: None,
            side: Side::Ask,
            price: 106,
            orders_ahead: 0,
            qty_ahead: 0,
            orders_behind: 0,
            qty_behind: 0,
            level_order_count: 0,
            level_total_qty: 0,
        }
    );
    b.assert_invariants();
}

#[test]
fn amend_size_with_policy_moves_amend_up_to_tail() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 5, 1)).unwrap();
    b.add(o(2, w(2), Side::Bid, 100, 3, 2)).unwrap();
    b.add(o(3, w(3), Side::Bid, 100, 7, 3)).unwrap();

    b.amend_size_with_policy(1, 8, AmendPriorityPolicy::LosePriorityOnIncrease)
        .unwrap();

    let ids: Vec<_> = b.orders_at(Side::Bid, 100).map(|o| o.id).collect();
    assert_eq!(ids, vec![2, 3, 1]);
    assert_eq!(b.depth(Side::Bid).next(), Some((100, 18, 3)));
    let pos = b.queue_position(1).unwrap();
    assert_eq!(pos.orders_ahead, 2);
    assert_eq!(pos.qty_ahead, 10);
    assert_eq!(pos.orders_behind, 0);
    assert_eq!(pos.qty_behind, 0);
    b.assert_invariants();
}

#[test]
fn amend_size_with_policy_preserves_priority_on_decrease_and_default_preserves_on_increase() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 5, 1)).unwrap();
    b.add(o(2, w(2), Side::Bid, 100, 3, 2)).unwrap();
    b.add(o(3, w(3), Side::Bid, 100, 7, 3)).unwrap();

    b.amend_size_with_policy(2, 1, AmendPriorityPolicy::LosePriorityOnIncrease)
        .unwrap();
    let ids: Vec<_> = b.orders_at(Side::Bid, 100).map(|o| o.id).collect();
    assert_eq!(ids, vec![1, 2, 3]);
    assert_eq!(b.depth(Side::Bid).next(), Some((100, 13, 3)));

    b.amend_size(1, 8).unwrap();
    let ids: Vec<_> = b.orders_at(Side::Bid, 100).map(|o| o.id).collect();
    assert_eq!(ids, vec![1, 2, 3]);
    assert_eq!(b.depth(Side::Bid).next(), Some((100, 16, 3)));
    b.assert_invariants();
}

#[test]
fn amend_size_with_policy_handles_equal_tail_zero_and_unknown_branches() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 5, 1)).unwrap();
    b.add(o(2, w(2), Side::Bid, 100, 3, 2)).unwrap();

    b.amend_size_with_policy(1, 5, AmendPriorityPolicy::LosePriorityOnIncrease)
        .unwrap();
    let ids: Vec<_> = b.orders_at(Side::Bid, 100).map(|o| o.id).collect();
    assert_eq!(ids, vec![1, 2]);

    b.amend_size_with_policy(2, 4, AmendPriorityPolicy::LosePriorityOnIncrease)
        .unwrap();
    let ids: Vec<_> = b.orders_at(Side::Bid, 100).map(|o| o.id).collect();
    assert_eq!(ids, vec![1, 2], "already-tail amend-up should remain tail");
    assert_eq!(b.depth(Side::Bid).next(), Some((100, 9, 2)));

    b.amend_size_with_policy(1, 0, AmendPriorityPolicy::LosePriorityOnIncrease)
        .unwrap();
    assert!(b.get(1).is_none());
    assert_eq!(b.depth(Side::Bid).next(), Some((100, 4, 1)));
    assert_eq!(
        b.amend_size_with_policy(999, 1, AmendPriorityPolicy::LosePriorityOnIncrease),
        Err(BookError::UnknownOrderId(999))
    );
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

#[test]
fn match_taker_order_returns_fifo_per_order_fills_without_mutating() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Ask, 101, 2, 1)).unwrap();
    b.add(o(2, w(2), Side::Ask, 101, 3, 2)).unwrap();
    b.add(o(3, w(3), Side::Ask, 103, 5, 3)).unwrap();

    let m = b.match_taker_order(Side::Bid, 6, 103);

    assert!(m.is_complete());
    assert_eq!(m.taker_side, Side::Bid);
    assert_eq!(m.requested_qty, 6);
    assert_eq!(m.filled_qty, 6);
    assert_eq!(m.unfilled_qty, 0);
    assert_eq!(m.filled_notional, 2 * 101 + 3 * 101 + 103);
    assert_eq!(m.reference_price, Some(101));
    assert_eq!(m.average_price, Some(608.0 / 6.0));
    assert_eq!(
        m.fills,
        vec![
            FillDelta {
                maker_order_id: 1,
                maker_wallet: w(1),
                maker_side: Side::Ask,
                price: 101,
                filled_qty: 2,
                maker_qty_before: 2,
                maker_qty_after: 0,
                maker_removed: true,
            },
            FillDelta {
                maker_order_id: 2,
                maker_wallet: w(2),
                maker_side: Side::Ask,
                price: 101,
                filled_qty: 3,
                maker_qty_before: 3,
                maker_qty_after: 0,
                maker_removed: true,
            },
            FillDelta {
                maker_order_id: 3,
                maker_wallet: w(3),
                maker_side: Side::Ask,
                price: 103,
                filled_qty: 1,
                maker_qty_before: 5,
                maker_qty_after: 4,
                maker_removed: false,
            },
        ]
    );
    assert!(!m.limit_stopped);
    assert!(!m.exhausted_book);

    // Pure matching did not mutate resting liquidity.
    assert_eq!(
        b.depth(Side::Ask).collect::<Vec<_>>(),
        vec![(101, 5, 2), (103, 5, 1)]
    );

    let s = b.estimate_slippage(Side::Bid, 6, 103);
    assert_eq!(m.filled_qty, s.filled_qty);
    assert_eq!(m.unfilled_qty, s.unfilled_qty);
    assert_eq!(m.filled_notional, s.filled_notional);
    assert_eq!(m.reference_price, s.reference_price);
    assert_eq!(m.average_price, s.average_price);
    assert_eq!(m.limit_stopped, s.limit_stopped);
    assert_eq!(m.exhausted_book, s.exhausted_book);
    b.assert_invariants();
}

#[test]
fn apply_taker_order_removes_full_makers_and_shrinks_partial_maker() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Ask, 101, 2, 1)).unwrap();
    b.add(o(2, w(2), Side::Ask, 101, 3, 2)).unwrap();

    let m = b.apply_taker_order(Side::Bid, 4, 101).unwrap();

    assert_eq!(m.filled_qty, 4);
    assert_eq!(m.unfilled_qty, 0);
    assert_eq!(
        m.fills.iter().map(|f| f.maker_order_id).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(b.get(1).is_none());
    assert_eq!(b.get(2).unwrap().qty, 1);
    assert_eq!(b.depth(Side::Ask).collect::<Vec<_>>(), vec![(101, 1, 1)]);
    assert_eq!(b.orders_by_wallet(w(1)).count(), 0);
    assert_eq!(
        b.orders_by_wallet(w(2)).map(|o| o.id).collect::<Vec<_>>(),
        vec![2]
    );
    b.assert_invariants();
}

#[test]
fn match_taker_order_distinguishes_limit_stop_from_exhausted_book() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 99, 4, 1)).unwrap();
    b.add(o(2, w(2), Side::Bid, 97, 5, 2)).unwrap();
    b.add(o(3, w(3), Side::Ask, 105, 2, 3)).unwrap();

    let stopped = b.match_taker_order(Side::Ask, 5, 98);
    assert_eq!(stopped.filled_qty, 4);
    assert_eq!(stopped.unfilled_qty, 1);
    assert!(stopped.limit_stopped);
    assert!(!stopped.exhausted_book);

    let exhausted = b.match_taker_order(Side::Bid, 10, 200);
    assert_eq!(exhausted.filled_qty, 2);
    assert_eq!(exhausted.unfilled_qty, 8);
    assert!(!exhausted.limit_stopped);
    assert!(exhausted.exhausted_book);
    b.assert_invariants();
}

#[test]
fn match_taker_order_sell_walks_bids_descending_and_fifo() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 101, 2, 1)).unwrap();
    b.add(o(2, w(2), Side::Bid, 101, 3, 2)).unwrap();
    b.add(o(3, w(3), Side::Bid, 99, 4, 3)).unwrap();

    let m = b.match_taker_order(Side::Ask, 6, 99);

    assert_eq!(m.filled_qty, 6);
    assert_eq!(m.unfilled_qty, 0);
    assert_eq!(m.filled_notional, 2 * 101 + 3 * 101 + 99);
    assert_eq!(m.reference_price, Some(101));
    assert_eq!(
        m.fills.iter().map(|f| f.maker_order_id).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(m.fills[2].filled_qty, 1);
    assert_eq!(m.fills[2].maker_qty_before, 4);
    assert_eq!(m.fills[2].maker_qty_after, 3);
    assert!(!m.fills[2].maker_removed);

    // Pure matching did not mutate bid liquidity.
    assert_eq!(
        b.depth(Side::Bid).collect::<Vec<_>>(),
        vec![(101, 5, 2), (99, 4, 1)]
    );
    b.assert_invariants();
}

#[test]
fn submit_limit_order_gtc_matches_then_rests_remainder() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Ask, 100, 2, 1)).unwrap();

    let outcome = b
        .submit_limit_order(o(10, w(9), Side::Bid, 101, 5, 10), LimitOrderPolicy::Gtc)
        .unwrap();

    assert_eq!(outcome.policy, LimitOrderPolicy::Gtc);
    assert_eq!(outcome.rejected, None);
    assert_eq!(outcome.taker_match.filled_qty, 2);
    assert_eq!(outcome.taker_match.unfilled_qty, 3);
    assert_eq!(
        outcome.rested_order,
        Some(o(10, w(9), Side::Bid, 101, 3, 10))
    );
    assert!(b.get(1).is_none());
    assert_eq!(b.get(10), Some(&o(10, w(9), Side::Bid, 101, 3, 10)));
    assert_eq!(b.best_bid_ask(), (Some(101), None));
    b.assert_invariants();
}

#[test]
fn submit_limit_order_gtc_with_no_fill_rests_full_order() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Ask, 100, 2, 1)).unwrap();

    let outcome = b
        .submit_limit_order(o(10, w(9), Side::Bid, 99, 5, 10), LimitOrderPolicy::Gtc)
        .unwrap();

    assert_eq!(outcome.rejected, None);
    assert_eq!(outcome.taker_match.filled_qty, 0);
    assert_eq!(outcome.taker_match.unfilled_qty, 5);
    assert!(outcome.taker_match.limit_stopped);
    assert_eq!(
        outcome.rested_order,
        Some(o(10, w(9), Side::Bid, 99, 5, 10))
    );
    assert_eq!(b.best_bid_ask(), (Some(99), Some(100)));
    assert_eq!(b.get(10), Some(&o(10, w(9), Side::Bid, 99, 5, 10)));
    b.assert_invariants();
}

#[test]
fn submit_limit_order_ioc_matches_and_cancels_remainder() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Ask, 100, 2, 1)).unwrap();

    let outcome = b
        .submit_limit_order(o(10, w(9), Side::Bid, 101, 5, 10), LimitOrderPolicy::Ioc)
        .unwrap();

    assert_eq!(outcome.policy, LimitOrderPolicy::Ioc);
    assert_eq!(outcome.rejected, None);
    assert_eq!(outcome.taker_match.filled_qty, 2);
    assert_eq!(outcome.taker_match.unfilled_qty, 3);
    assert_eq!(outcome.rested_order, None);
    assert!(b.get(1).is_none());
    assert!(b.get(10).is_none());
    assert!(b.is_empty());
    b.assert_invariants();
}

#[test]
fn submit_limit_order_ioc_with_no_fill_cancels_full_order_without_mutation() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Ask, 100, 2, 1)).unwrap();

    let outcome = b
        .submit_limit_order(o(10, w(9), Side::Bid, 99, 5, 10), LimitOrderPolicy::Ioc)
        .unwrap();

    assert_eq!(outcome.rejected, None);
    assert_eq!(outcome.taker_match.filled_qty, 0);
    assert_eq!(outcome.taker_match.unfilled_qty, 5);
    assert_eq!(outcome.rested_order, None);
    assert_eq!(b.len(), 1);
    assert_eq!(b.get(1), Some(&o(1, w(1), Side::Ask, 100, 2, 1)));
    assert!(b.get(10).is_none());
    b.assert_invariants();
}

#[test]
fn submit_limit_order_fok_rejects_without_mutation_when_not_fully_fillable() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Ask, 100, 2, 1)).unwrap();

    let outcome = b
        .submit_limit_order(o(10, w(9), Side::Bid, 101, 5, 10), LimitOrderPolicy::Fok)
        .unwrap();

    assert_eq!(outcome.policy, LimitOrderPolicy::Fok);
    assert_eq!(
        outcome.rejected,
        Some(SubmitRejectReason::FillOrKillWouldNotFill)
    );
    assert_eq!(outcome.taker_match.filled_qty, 2);
    assert_eq!(outcome.taker_match.unfilled_qty, 3);
    assert_eq!(outcome.rested_order, None);
    assert_eq!(b.get(1), Some(&o(1, w(1), Side::Ask, 100, 2, 1)));
    assert!(b.get(10).is_none());
    b.assert_invariants();
}

#[test]
fn submit_limit_order_fok_fills_without_resting_when_fully_fillable() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Ask, 100, 2, 1)).unwrap();
    b.add(o(2, w(2), Side::Ask, 101, 3, 2)).unwrap();

    let outcome = b
        .submit_limit_order(o(10, w(9), Side::Bid, 101, 5, 10), LimitOrderPolicy::Fok)
        .unwrap();

    assert_eq!(outcome.rejected, None);
    assert_eq!(outcome.taker_match.filled_qty, 5);
    assert_eq!(outcome.taker_match.unfilled_qty, 0);
    assert_eq!(outcome.rested_order, None);
    assert!(b.get(1).is_none());
    assert!(b.get(2).is_none());
    assert!(b.get(10).is_none());
    assert!(b.is_empty());
    b.assert_invariants();
}

#[test]
fn submit_limit_order_post_only_rejects_crossing_and_rests_non_crossing() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Ask, 100, 2, 1)).unwrap();

    let rejected = b
        .submit_limit_order(
            o(10, w(9), Side::Bid, 100, 5, 10),
            LimitOrderPolicy::PostOnly,
        )
        .unwrap();

    assert_eq!(
        rejected.rejected,
        Some(SubmitRejectReason::PostOnlyWouldCross)
    );
    assert_eq!(rejected.taker_match.filled_qty, 2);
    assert_eq!(rejected.rested_order, None);
    assert_eq!(b.get(1), Some(&o(1, w(1), Side::Ask, 100, 2, 1)));
    assert!(b.get(10).is_none());

    let rested = b
        .submit_limit_order(
            o(11, w(9), Side::Bid, 99, 5, 11),
            LimitOrderPolicy::PostOnly,
        )
        .unwrap();

    assert_eq!(rested.rejected, None);
    assert_eq!(rested.taker_match.filled_qty, 0);
    assert!(rested.taker_match.limit_stopped);
    assert_eq!(rested.rested_order, Some(o(11, w(9), Side::Bid, 99, 5, 11)));
    assert_eq!(b.get(11), Some(&o(11, w(9), Side::Bid, 99, 5, 11)));
    assert_eq!(b.best_bid_ask(), (Some(99), Some(100)));
    b.assert_invariants();
}

#[test]
fn submit_limit_order_post_only_rests_on_empty_book() {
    let mut b = OrderBook::new();

    let outcome = b
        .submit_limit_order(
            o(10, w(9), Side::Bid, 100, 5, 10),
            LimitOrderPolicy::PostOnly,
        )
        .unwrap();

    assert_eq!(outcome.rejected, None);
    assert_eq!(outcome.taker_match.filled_qty, 0);
    assert_eq!(outcome.taker_match.unfilled_qty, 5);
    assert!(!outcome.taker_match.limit_stopped);
    assert!(outcome.taker_match.exhausted_book);
    assert_eq!(
        outcome.rested_order,
        Some(o(10, w(9), Side::Bid, 100, 5, 10))
    );
    assert_eq!(b.get(10), Some(&o(10, w(9), Side::Bid, 100, 5, 10)));
    b.assert_invariants();
}

#[test]
fn submit_limit_order_rejects_zero_qty_and_duplicate_id_before_mutation() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Ask, 100, 2, 1)).unwrap();

    assert_eq!(
        b.submit_limit_order(o(10, w(9), Side::Bid, 100, 0, 10), LimitOrderPolicy::Gtc),
        Err(BookError::ZeroQty)
    );
    assert_eq!(
        b.apply_taker_order(Side::Bid, 0, 100),
        Err(BookError::ZeroQty)
    );
    assert_eq!(
        b.submit_limit_order(o(1, w(9), Side::Bid, 100, 5, 10), LimitOrderPolicy::Gtc),
        Err(BookError::DuplicateOrderId(1))
    );
    assert_eq!(b.get(1), Some(&o(1, w(1), Side::Ask, 100, 2, 1)));
    b.assert_invariants();
}

#[test]
fn synthetic_order_id_helpers_use_high_bit_namespace() {
    assert_eq!(SYNTHETIC_ORDER_ID_FLAG, 1u64 << 63);
    assert!(!is_synthetic_order_id(42));

    let synthetic = synthetic_order_id(42).unwrap();
    assert_eq!(synthetic, SYNTHETIC_ORDER_ID_FLAG | 42);
    assert!(is_synthetic_order_id(synthetic));
    assert_eq!(synthetic_order_id(synthetic), None);
}

#[test]
fn reasoned_book_op_carries_cause_metadata() {
    let op = BookOp::Add(o(1, w(1), Side::Bid, 100, 5, 1));
    let reasoned = ReasonedBookOp::new(
        op,
        OperationCause::Simulator(SimulatorCause::LimitOrderRest),
    );

    assert_eq!(reasoned.op, op);
    assert_eq!(
        reasoned.cause,
        OperationCause::Simulator(SimulatorCause::LimitOrderRest)
    );
}

#[test]
fn apply_op_strict_applies_all_core_variants_and_reports_errors() {
    let mut b = OrderBook::new();
    let order = o(1, w(1), Side::Bid, 100, 10, 1);

    b.apply_op(BookOp::Add(order)).unwrap();
    assert_eq!(b.get(1), Some(&order));

    b.apply_op(BookOp::UpdateSize { id: 1, new_qty: 4 })
        .unwrap();
    assert_eq!(b.get(1).unwrap().qty, 4);

    b.apply_op(BookOp::AmendSize { id: 1, new_qty: 6 }).unwrap();
    assert_eq!(b.get(1).unwrap().qty, 6);

    assert_eq!(
        b.apply_op(BookOp::UpdateSize { id: 1, new_qty: 6 }),
        Err(BookError::NonDecreasingSize {
            current: 6,
            proposed: 6,
        })
    );
    assert_eq!(
        b.apply_op(BookOp::Add(order)),
        Err(BookError::DuplicateOrderId(1))
    );

    b.apply_op(BookOp::Remove(1)).unwrap();
    assert!(b.is_empty());
    assert_eq!(
        b.apply_op(BookOp::Remove(1)),
        Err(BookError::UnknownOrderId(1))
    );
    b.assert_invariants();
}

#[test]
fn replay_policy_strict_returns_error_outcome_without_mutating() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 4, 1)).unwrap();

    let op = BookOp::UpdateSize { id: 1, new_qty: 5 };
    assert_eq!(
        b.apply_op_with_policy(op, ReplayPolicy::STRICT),
        ReplayApplyOutcome::Error {
            op,
            error: BookError::NonDecreasingSize {
                current: 4,
                proposed: 5,
            },
        }
    );
    assert_eq!(b.get(1).unwrap().qty, 4);
    b.assert_invariants();
}

#[test]
fn replay_policy_tolerant_skips_missing_and_duplicate_ops() {
    let mut b = OrderBook::new();
    let order = o(1, w(1), Side::Ask, 101, 2, 1);
    b.add(order).unwrap();

    assert_eq!(
        b.apply_op_with_policy(BookOp::Remove(999), ReplayPolicy::DWELLIR_TOLERANT),
        ReplayApplyOutcome::Skipped {
            op: BookOp::Remove(999),
            reason: ToleratedReplayReason::MissingOrder { id: 999 },
        }
    );
    assert_eq!(
        b.apply_op_with_policy(
            BookOp::UpdateSize {
                id: 999,
                new_qty: 1
            },
            ReplayPolicy::DWELLIR_TOLERANT,
        ),
        ReplayApplyOutcome::Skipped {
            op: BookOp::UpdateSize {
                id: 999,
                new_qty: 1
            },
            reason: ToleratedReplayReason::MissingOrder { id: 999 },
        }
    );
    assert_eq!(
        b.apply_op_with_policy(BookOp::Add(order), ReplayPolicy::DWELLIR_TOLERANT),
        ReplayApplyOutcome::Skipped {
            op: BookOp::Add(order),
            reason: ToleratedReplayReason::DuplicateAdd { id: 1 },
        }
    );

    assert_eq!(b.len(), 1);
    assert_eq!(b.get(1), Some(&order));
    b.assert_invariants();
}

#[test]
fn replay_policy_tolerant_coerces_non_decreasing_update_to_amend() {
    let mut b = OrderBook::new();
    b.add(o(1, w(1), Side::Bid, 100, 4, 1)).unwrap();
    b.add(o(2, w(2), Side::Bid, 100, 3, 2)).unwrap();

    let original = BookOp::UpdateSize { id: 1, new_qty: 7 };
    let applied = BookOp::AmendSize { id: 1, new_qty: 7 };
    assert_eq!(
        b.apply_op_with_policy(original, ReplayPolicy::DWELLIR_TOLERANT),
        ReplayApplyOutcome::Coerced {
            original,
            applied,
            reason: ToleratedReplayReason::NonDecreasingUpdate {
                id: 1,
                current: 4,
                proposed: 7,
            },
        }
    );

    assert_eq!(b.get(1).unwrap().qty, 7);
    // Coerced amend uses default preserve-priority semantics.
    assert_eq!(
        b.orders_at(Side::Bid, 100)
            .map(|o| o.id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(b.depth(Side::Bid).next(), Some((100, 10, 2)));
    b.assert_invariants();
}

#[test]
fn apply_snapshot_replace_removes_synthetic_orders() {
    let synthetic_id = synthetic_order_id(1).unwrap();
    let mut b = OrderBook::new();
    b.add(o(synthetic_id, w(9), Side::Bid, 99, 5, 1)).unwrap();
    b.add(o(1, w(1), Side::Ask, 101, 2, 1)).unwrap();

    let outcome = b
        .apply_snapshot_with_policy([o(2, w(2), Side::Ask, 102, 3, 2)], SnapshotPolicy::Replace)
        .unwrap();

    assert_eq!(
        outcome,
        SnapshotOutcome {
            inserted: 1,
            preserved: 0
        }
    );
    assert!(b.get(synthetic_id).is_none());
    assert!(b.get(1).is_none());
    assert_eq!(b.get(2), Some(&o(2, w(2), Side::Ask, 102, 3, 2)));
    b.assert_invariants();
}

#[test]
fn apply_snapshot_preserve_synthetic_keeps_local_orders_and_replaces_venue_orders() {
    let syn_bid = synthetic_order_id(1).unwrap();
    let syn_ask = synthetic_order_id(2).unwrap();
    let mut b = OrderBook::new();
    b.add(o(10, w(1), Side::Bid, 100, 2, 1)).unwrap();
    b.add(o(syn_bid, w(9), Side::Bid, 99, 5, 2)).unwrap();
    b.add(o(syn_ask, w(9), Side::Ask, 105, 7, 3)).unwrap();

    let outcome = b
        .apply_snapshot_with_policy(
            [
                o(20, w(2), Side::Bid, 101, 3, 4),
                o(21, w(3), Side::Ask, 106, 4, 5),
            ],
            SnapshotPolicy::PreserveSynthetic,
        )
        .unwrap();

    assert_eq!(
        outcome,
        SnapshotOutcome {
            inserted: 2,
            preserved: 2
        }
    );
    assert!(b.get(10).is_none());
    assert_eq!(b.get(20), Some(&o(20, w(2), Side::Bid, 101, 3, 4)));
    assert_eq!(b.get(21), Some(&o(21, w(3), Side::Ask, 106, 4, 5)));
    assert_eq!(b.get(syn_bid), Some(&o(syn_bid, w(9), Side::Bid, 99, 5, 2)));
    assert_eq!(
        b.get(syn_ask),
        Some(&o(syn_ask, w(9), Side::Ask, 105, 7, 3))
    );
    assert_eq!(
        b.depth(Side::Bid).collect::<Vec<_>>(),
        vec![(101, 3, 1), (99, 5, 1)]
    );
    assert_eq!(
        b.depth(Side::Ask).collect::<Vec<_>>(),
        vec![(105, 7, 1), (106, 4, 1)]
    );
    b.assert_invariants();
}

#[test]
fn apply_snapshot_preserve_synthetic_reports_collision_before_mutation() {
    let synthetic_id = synthetic_order_id(1).unwrap();
    let mut b = OrderBook::new();
    let existing = o(synthetic_id, w(9), Side::Bid, 99, 5, 1);
    b.add(existing).unwrap();
    b.add(o(1, w(1), Side::Ask, 101, 2, 1)).unwrap();

    assert_eq!(
        b.apply_snapshot_with_policy(
            [o(synthetic_id, w(2), Side::Ask, 102, 3, 2)],
            SnapshotPolicy::PreserveSynthetic,
        ),
        Err(BookError::DuplicateOrderId(synthetic_id))
    );

    assert_eq!(b.get(synthetic_id), Some(&existing));
    assert_eq!(b.get(1), Some(&o(1, w(1), Side::Ask, 101, 2, 1)));
    b.assert_invariants();
}

#[test]
fn apply_snapshot_preserve_synthetic_prevalidates_snapshot_before_clearing() {
    let synthetic_id = synthetic_order_id(1).unwrap();
    for (orders, expected) in [
        (vec![o(2, w(2), Side::Ask, 102, 0, 2)], BookError::ZeroQty),
        (
            vec![
                o(2, w(2), Side::Ask, 102, 3, 2),
                o(2, w(3), Side::Bid, 99, 1, 3),
            ],
            BookError::DuplicateOrderId(2),
        ),
        (
            vec![o(
                synthetic_order_id(99).unwrap(),
                w(2),
                Side::Ask,
                102,
                3,
                2,
            )],
            BookError::SyntheticOrderIdInSnapshot(synthetic_order_id(99).unwrap()),
        ),
    ] {
        let mut b = OrderBook::new();
        let existing = o(synthetic_id, w(9), Side::Bid, 99, 5, 1);
        let venue = o(1, w(1), Side::Ask, 101, 2, 1);
        b.add(existing).unwrap();
        b.add(venue).unwrap();

        assert_eq!(
            b.apply_snapshot_with_policy(orders, SnapshotPolicy::PreserveSynthetic),
            Err(expected)
        );
        assert_eq!(b.get(synthetic_id), Some(&existing));
        assert_eq!(b.get(1), Some(&venue));
        b.assert_invariants();
    }
}
