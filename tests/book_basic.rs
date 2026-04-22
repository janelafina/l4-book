use l4_book::{BookError, Order, OrderBook, Side, WalletId};

fn w(n: u8) -> WalletId {
    let mut x = [0u8; 20];
    x[0] = n;
    WalletId(x)
}

fn o(id: u64, wallet: WalletId, side: Side, price: u64, qty: u64, ts: u64) -> Order {
    Order { id, wallet, side, price, qty, ts }
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
        Err(BookError::NonDecreasingSize { current: 3, proposed: 3 }),
    );
    assert_eq!(
        b.update_size(1, 5),
        Err(BookError::NonDecreasingSize { current: 3, proposed: 5 }),
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
    assert_eq!(b.add(o(1, w(2), Side::Ask, 99, 1, 2)), Err(BookError::DuplicateOrderId(1)));
    assert_eq!(b.remove(999), Err(BookError::UnknownOrderId(999)));
    assert_eq!(b.add(o(2, w(1), Side::Bid, 100, 0, 1)), Err(BookError::ZeroQty));
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
    ]).unwrap();

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
    assert_eq!(format!("{w:?}"), "0xf9109ada2f73c62e9889b45453065f0d99260a2d");
    assert!(WalletId::from_hex("0x1234").is_err());
    assert!(WalletId::from_hex("zzzz9ada2f73c62e9889b45453065f0d99260a2d").is_err());
}
