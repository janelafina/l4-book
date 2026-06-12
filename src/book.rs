use std::collections::btree_map;
use std::collections::BTreeMap;

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::level::{Level, OrderNode};
use crate::types::{
    AmendPriorityPolicy, BookError, BookOp, DuplicateAddPolicy, FillDelta, LimitOrderPolicy,
    MissingOrderPolicy, NonDecreasingUpdatePolicy, Order, OrderId, Price, Qty, QueuePosition,
    ReplayApplyOutcome, ReplayPolicy, Side, SnapshotOutcome, SnapshotPolicy,
    SubmitLimitOrderOutcome, SubmitRejectReason, TakerMatch, ToleratedReplayReason, Ts, WalletId,
    is_synthetic_order_id,
};

/// A named aggregate price level snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregatedLevel {
    pub price: Price,
    pub total_qty: Qty,
    pub order_count: u64,
}

/// Top-of-book aggregate depth for both sides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregatedTopLevels {
    /// Bid levels in best-to-worse order (highest price first).
    pub bids: Vec<AggregatedLevel>,
    /// Ask levels in best-to-worse order (lowest price first).
    pub asks: Vec<AggregatedLevel>,
}

/// Per-order data in an unaggregated level snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelOrder {
    pub id: OrderId,
    pub wallet: WalletId,
    pub qty: Qty,
    pub ts: Ts,
}

/// One price level containing its resting orders in FIFO order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnaggregatedLevel {
    pub price: Price,
    pub orders: Vec<LevelOrder>,
}

/// Top-of-book unaggregated depth for both sides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnaggregatedTopLevels {
    /// Bid levels in best-to-worse order (highest price first).
    pub bids: Vec<UnaggregatedLevel>,
    /// Ask levels in best-to-worse order (lowest price first).
    pub asks: Vec<UnaggregatedLevel>,
}

/// Slippage estimate for a hypothetical taker order.
///
/// `taker_side` follows normal order side semantics: `Side::Bid` is a buy that
/// consumes asks, and `Side::Ask` is a sell that consumes bids. Slippage is the
/// adverse average-price move versus the current best opposite quote: buy
/// slippage is `average_price - best_ask`; sell slippage is
/// `best_bid - average_price`.
#[derive(Clone, Debug, PartialEq)]
pub struct SlippageEstimate {
    pub taker_side: Side,
    pub requested_qty: Qty,
    pub filled_qty: Qty,
    pub unfilled_qty: Qty,
    /// Sum of `execution_price * filled_qty` across walked levels.
    pub filled_notional: u128,
    pub limit_price: Price,
    pub reference_price: Option<Price>,
    pub average_price: Option<f64>,
    /// Adverse average-price move versus `reference_price`.
    pub slippage: Option<f64>,
    /// Exact adverse extra notional versus filling all executed quantity at
    /// `reference_price`.
    pub slippage_notional: Option<u128>,
    pub slippage_pct: Option<f64>,
    /// True when the next available resting price would violate `limit_price`.
    pub limit_stopped: bool,
    /// True when the acceptable opposite book was exhausted before filling.
    pub exhausted_book: bool,
}

impl SlippageEstimate {
    pub fn is_complete(&self) -> bool {
        self.unfilled_qty == 0
    }
}

/// A limit order book with per-order wallet attribution.
///
/// Storage layout:
/// * `slab` is a slot arena holding every resting order. Indices into `slab`
///   are stable until the order is removed.
/// * `free_list` tracks slots that have been freed and can be reused.
/// * `order_index` maps `OrderId` -> slab slot for O(1) lookup.
/// * `bids` / `asks` are `BTreeMap<Price, Level>`; each `Level` owns a
///   doubly-linked list through slab nodes preserving FIFO time priority.
/// * `wallet_index` maps `WalletId` -> set of orders the wallet currently has
///   resting on the book; this is the L4 attribution surface.
#[derive(Clone)]
pub struct OrderBook {
    slab: Vec<Option<OrderNode>>,
    free_list: Vec<usize>,
    order_index: HashMap<OrderId, usize>,
    bids: BTreeMap<Price, Level>,
    asks: BTreeMap<Price, Level>,
    wallet_index: HashMap<WalletId, HashSet<OrderId>>,
}

impl Default for OrderBook {
    fn default() -> Self {
        Self::new()
    }
}

impl OrderBook {
    pub fn new() -> Self {
        Self {
            slab: Vec::new(),
            free_list: Vec::new(),
            order_index: HashMap::default(),
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            wallet_index: HashMap::default(),
        }
    }

    pub fn with_capacity(orders: usize) -> Self {
        let mut b = Self::new();
        b.slab.reserve(orders);
        b.order_index.reserve(orders);
        b
    }

    pub fn len(&self) -> usize {
        self.order_index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order_index.is_empty()
    }

    /// Total slab capacity (live + freed). Exposed for benchmarking and tests
    /// that want to check the free-list is actually reusing slots rather than
    /// letting the slab grow unboundedly.
    pub fn slab_len(&self) -> usize {
        self.slab.len()
    }

    /// Replace all book state with the provided orders.
    pub fn apply_snapshot<I: IntoIterator<Item = Order>>(
        &mut self,
        orders: I,
    ) -> Result<(), BookError> {
        self.apply_snapshot_with_policy(orders, SnapshotPolicy::Replace)?;
        Ok(())
    }

    /// Apply a snapshot using an explicit replacement policy.
    ///
    /// [`SnapshotPolicy::Replace`] preserves the historical behavior of
    /// [`apply_snapshot`](Self::apply_snapshot): clear the book, then insert the
    /// supplied orders. [`SnapshotPolicy::PreserveSynthetic`] replaces venue
    /// state while re-adding currently live high-bit synthetic orders after the
    /// snapshot orders.
    pub fn apply_snapshot_with_policy<I: IntoIterator<Item = Order>>(
        &mut self,
        orders: I,
        policy: SnapshotPolicy,
    ) -> Result<SnapshotOutcome, BookError> {
        let orders: Vec<Order> = orders.into_iter().collect();
        let preserved = match policy {
            SnapshotPolicy::Replace => Vec::new(),
            SnapshotPolicy::PreserveSynthetic => self.synthetic_orders_in_book_order(),
        };
        if policy == SnapshotPolicy::PreserveSynthetic {
            validate_preserve_synthetic_snapshot(&orders, &preserved)?;
        }

        self.clear();
        let mut inserted = 0;
        for order in orders {
            self.add(order)?;
            inserted += 1;
        }
        let mut preserved_count = 0;
        for order in preserved {
            self.add(order)?;
            preserved_count += 1;
        }
        Ok(SnapshotOutcome {
            inserted,
            preserved: preserved_count,
        })
    }

    pub fn clear(&mut self) {
        self.slab.clear();
        self.free_list.clear();
        self.order_index.clear();
        self.bids.clear();
        self.asks.clear();
        self.wallet_index.clear();
    }

    /// Apply a venue-neutral book operation with strict error behavior.
    pub fn apply_op(&mut self, op: BookOp) -> Result<(), BookError> {
        match op {
            BookOp::Add(order) => self.add(order),
            BookOp::Remove(id) => self.remove(id).map(|_| ()),
            BookOp::UpdateSize { id, new_qty } => self.update_size(id, new_qty),
            BookOp::AmendSize { id, new_qty } => self.amend_size(id, new_qty),
        }
    }

    /// Apply a venue-neutral book operation under a replay policy.
    ///
    /// Strict policy mirrors [`apply_op`](Self::apply_op). Tolerant policies can
    /// skip missing/duplicate feed artifacts or coerce a non-decreasing
    /// `UpdateSize` into `AmendSize`, while returning an explicit outcome for
    /// replay accounting.
    pub fn apply_op_with_policy(&mut self, op: BookOp, policy: ReplayPolicy) -> ReplayApplyOutcome {
        match op {
            BookOp::Add(order) if self.order_index.contains_key(&order.id) => {
                if policy.duplicate_add == DuplicateAddPolicy::Skip {
                    ReplayApplyOutcome::Skipped {
                        op,
                        reason: ToleratedReplayReason::DuplicateAdd { id: order.id },
                    }
                } else {
                    ReplayApplyOutcome::Error {
                        op,
                        error: BookError::DuplicateOrderId(order.id),
                    }
                }
            }
            BookOp::Remove(id) if !self.order_index.contains_key(&id) => {
                if policy.missing_order == MissingOrderPolicy::Skip {
                    ReplayApplyOutcome::Skipped {
                        op,
                        reason: ToleratedReplayReason::MissingOrder { id },
                    }
                } else {
                    ReplayApplyOutcome::Error {
                        op,
                        error: BookError::UnknownOrderId(id),
                    }
                }
            }
            BookOp::UpdateSize { id, new_qty } => {
                let Some(slot) = self.order_index.get(&id).copied() else {
                    if policy.missing_order == MissingOrderPolicy::Skip {
                        return ReplayApplyOutcome::Skipped {
                            op,
                            reason: ToleratedReplayReason::MissingOrder { id },
                        };
                    }
                    return ReplayApplyOutcome::Error {
                        op,
                        error: BookError::UnknownOrderId(id),
                    };
                };
                let current = self.slab[slot]
                    .as_ref()
                    .expect("slab slot present for indexed order")
                    .order
                    .qty;
                if new_qty >= current
                    && policy.non_decreasing_update == NonDecreasingUpdatePolicy::TreatAsAmend
                {
                    let applied = BookOp::AmendSize { id, new_qty };
                    match self.apply_op(applied) {
                        Ok(()) => ReplayApplyOutcome::Coerced {
                            original: op,
                            applied,
                            reason: ToleratedReplayReason::NonDecreasingUpdate {
                                id,
                                current,
                                proposed: new_qty,
                            },
                        },
                        Err(error) => ReplayApplyOutcome::Error { op, error },
                    }
                } else {
                    match self.apply_op(op) {
                        Ok(()) => ReplayApplyOutcome::Applied { op },
                        Err(error) => ReplayApplyOutcome::Error { op, error },
                    }
                }
            }
            BookOp::AmendSize { id, .. } if !self.order_index.contains_key(&id) => {
                if policy.missing_order == MissingOrderPolicy::Skip {
                    ReplayApplyOutcome::Skipped {
                        op,
                        reason: ToleratedReplayReason::MissingOrder { id },
                    }
                } else {
                    ReplayApplyOutcome::Error {
                        op,
                        error: BookError::UnknownOrderId(id),
                    }
                }
            }
            _ => match self.apply_op(op) {
                Ok(()) => ReplayApplyOutcome::Applied { op },
                Err(error) => ReplayApplyOutcome::Error { op, error },
            },
        }
    }

    /// Insert a new order. Errors on zero qty or duplicate id.
    pub fn add(&mut self, order: Order) -> Result<(), BookError> {
        if order.qty == 0 {
            return Err(BookError::ZeroQty);
        }
        if self.order_index.contains_key(&order.id) {
            return Err(BookError::DuplicateOrderId(order.id));
        }

        let slot = self.alloc_slot();
        let node = OrderNode::new(order);
        self.slab[slot] = Some(node);
        self.order_index.insert(order.id, slot);
        self.wallet_index
            .entry(order.wallet)
            .or_default()
            .insert(order.id);

        let levels = match order.side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        };
        let level = levels.entry(order.price).or_insert_with(Level::new);
        level.order_count += 1;
        level.total_qty += order.qty;
        link_tail(&mut self.slab, level, slot);
        Ok(())
    }

    /// Remove and return the order with the given id.
    pub fn remove(&mut self, id: OrderId) -> Result<Order, BookError> {
        let slot = self
            .order_index
            .remove(&id)
            .ok_or(BookError::UnknownOrderId(id))?;
        let node = self.slab[slot]
            .take()
            .expect("slab slot present for indexed order");
        self.free_list.push(slot);

        let order = node.order;
        let levels = match order.side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        };
        let remove_level = {
            let level = levels
                .get_mut(&order.price)
                .expect("level present for resting order");
            level.order_count -= 1;
            level.total_qty -= order.qty;
            unlink(&mut self.slab, level, node.prev, node.next);
            level.order_count == 0
        };
        if remove_level {
            levels.remove(&order.price);
        }

        if let Some(set) = self.wallet_index.get_mut(&order.wallet) {
            set.remove(&id);
            if set.is_empty() {
                self.wallet_index.remove(&order.wallet);
            }
        }
        Ok(order)
    }

    /// Reduce the size of a resting order (partial-fill semantics). `new_qty`
    /// must be strictly less than the current qty. If `new_qty == 0` the order
    /// is removed. Preserves queue position.
    pub fn update_size(&mut self, id: OrderId, new_qty: Qty) -> Result<(), BookError> {
        let slot = *self
            .order_index
            .get(&id)
            .ok_or(BookError::UnknownOrderId(id))?;
        let (current, side, price) = {
            let node = self.slab[slot]
                .as_ref()
                .expect("slab slot present for indexed order");
            (node.order.qty, node.order.side, node.order.price)
        };
        if new_qty >= current {
            return Err(BookError::NonDecreasingSize {
                current,
                proposed: new_qty,
            });
        }
        if new_qty == 0 {
            self.remove(id)?;
            return Ok(());
        }
        self.slab[slot]
            .as_mut()
            .expect("slab slot present")
            .order
            .qty = new_qty;
        let levels = match side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        };
        levels.get_mut(&price).expect("level present").total_qty -= current - new_qty;
        Ok(())
    }

    /// Amend the size of a resting order. `new_qty` may be larger or smaller.
    /// Queue position is preserved (adapter can use
    /// [`amend_size_with_policy`](Self::amend_size_with_policy) if the venue's
    /// policy is "amend-up loses priority"). `new_qty == 0` removes the order.
    pub fn amend_size(&mut self, id: OrderId, new_qty: Qty) -> Result<(), BookError> {
        self.amend_size_with_policy(id, new_qty, AmendPriorityPolicy::Preserve)
    }

    /// Amend the size of a resting order using an explicit priority policy.
    ///
    /// [`AmendPriorityPolicy::Preserve`] keeps the current FIFO position for all
    /// size changes and matches [`amend_size`](Self::amend_size). With
    /// [`AmendPriorityPolicy::LosePriorityOnIncrease`], increasing size moves
    /// the order to the tail of its current price level; decreases and equal-size
    /// amendments preserve position. `new_qty == 0` removes the order.
    pub fn amend_size_with_policy(
        &mut self,
        id: OrderId,
        new_qty: Qty,
        policy: AmendPriorityPolicy,
    ) -> Result<(), BookError> {
        if new_qty == 0 {
            self.remove(id)?;
            return Ok(());
        }
        let slot = *self
            .order_index
            .get(&id)
            .ok_or(BookError::UnknownOrderId(id))?;
        let (current, side, price) = {
            let node = self.slab[slot]
                .as_ref()
                .expect("slab slot present for indexed order");
            (node.order.qty, node.order.side, node.order.price)
        };
        if current == new_qty {
            return Ok(());
        }
        self.slab[slot]
            .as_mut()
            .expect("slab slot present")
            .order
            .qty = new_qty;
        let levels = match side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        };
        let level = levels.get_mut(&price).expect("level present");
        if new_qty > current {
            level.total_qty += new_qty - current;
            if policy == AmendPriorityPolicy::LosePriorityOnIncrease {
                move_to_tail(&mut self.slab, level, slot);
            }
        } else {
            level.total_qty -= current - new_qty;
        }
        Ok(())
    }

    /// Return queue-position statistics for a live resting order.
    pub fn queue_position(&self, id: OrderId) -> Result<QueuePosition, BookError> {
        let target_slot = *self
            .order_index
            .get(&id)
            .ok_or(BookError::UnknownOrderId(id))?;
        let target = self.slab[target_slot]
            .as_ref()
            .expect("slab slot present for indexed order")
            .order;
        let levels = match target.side {
            Side::Bid => &self.bids,
            Side::Ask => &self.asks,
        };
        let level = levels
            .get(&target.price)
            .expect("level present for resting order");

        let mut orders_ahead = 0u64;
        let mut qty_ahead = 0u64;
        let mut orders_behind = 0u64;
        let mut qty_behind = 0u64;
        let mut seen_target = false;
        let mut cur = level.head;
        while let Some(slot) = cur {
            let node = self.slab[slot].as_ref().expect("live linked slot");
            if slot == target_slot {
                seen_target = true;
            } else if seen_target {
                orders_behind += 1;
                qty_behind += node.order.qty;
            } else {
                orders_ahead += 1;
                qty_ahead += node.order.qty;
            }
            cur = node.next;
        }
        debug_assert!(seen_target, "indexed order missing from its level list");

        Ok(QueuePosition {
            id: Some(id),
            side: target.side,
            price: target.price,
            orders_ahead,
            qty_ahead,
            orders_behind,
            qty_behind,
            level_order_count: level.order_count,
            level_total_qty: level.total_qty,
        })
    }

    /// Return queue-position statistics for a hypothetical order that would be
    /// appended to the tail of `side`/`price`.
    pub fn queue_position_for_new_order(&self, side: Side, price: Price) -> QueuePosition {
        let levels = match side {
            Side::Bid => &self.bids,
            Side::Ask => &self.asks,
        };
        let (level_order_count, level_total_qty) = levels
            .get(&price)
            .map(|level| (level.order_count, level.total_qty))
            .unwrap_or((0, 0));
        QueuePosition {
            id: None,
            side,
            price,
            orders_ahead: level_order_count,
            qty_ahead: level_total_qty,
            orders_behind: 0,
            qty_behind: 0,
            level_order_count,
            level_total_qty,
        }
    }

    pub fn get(&self, id: OrderId) -> Option<&Order> {
        let slot = *self.order_index.get(&id)?;
        self.slab[slot].as_ref().map(|n| &n.order)
    }

    pub fn best_bid(&self) -> Option<Price> {
        self.bids.keys().next_back().copied()
    }

    pub fn best_ask(&self) -> Option<Price> {
        self.asks.keys().next().copied()
    }

    /// Return best bid and ask together.
    pub fn best_bid_ask(&self) -> (Option<Price>, Option<Price>) {
        (self.best_bid(), self.best_ask())
    }

    /// Return the first `n` levels per side as aggregate snapshots.
    ///
    /// Bids are ordered best-to-worse (descending price); asks are ordered
    /// best-to-worse (ascending price).
    pub fn top_n_levels_aggregated(&self, n: usize) -> AggregatedTopLevels {
        AggregatedTopLevels {
            bids: self.top_n_levels_aggregated_for_side(Side::Bid, n),
            asks: self.top_n_levels_aggregated_for_side(Side::Ask, n),
        }
    }

    /// Return the first `n` levels per side with individual resting orders.
    ///
    /// Orders inside each level preserve FIFO time priority. The level count is
    /// capped, but every order in each returned level is copied into the
    /// snapshot.
    pub fn top_n_levels(&self, n: usize) -> UnaggregatedTopLevels {
        UnaggregatedTopLevels {
            bids: self.top_n_levels_for_side(Side::Bid, n),
            asks: self.top_n_levels_for_side(Side::Ask, n),
        }
    }

    /// Estimate execution and slippage for a hypothetical limit taker order.
    ///
    /// `taker_side` is the side of the incoming order: `Side::Bid` buys from
    /// asks up to `limit_price`, while `Side::Ask` sells into bids down to
    /// `limit_price`. The book is not mutated. Partial fills are represented by
    /// `filled_qty`, `unfilled_qty`, `limit_stopped`, and `exhausted_book`.
    pub fn estimate_slippage(
        &self,
        taker_side: Side,
        qty: Qty,
        limit_price: Price,
    ) -> SlippageEstimate {
        let reference_price = match taker_side {
            Side::Bid => self.best_ask(),
            Side::Ask => self.best_bid(),
        };
        let opposite_side = match taker_side {
            Side::Bid => Side::Ask,
            Side::Ask => Side::Bid,
        };

        let mut remaining = qty;
        let mut notional = 0u128;
        let mut limit_stopped = false;

        if remaining > 0 {
            for (price, level_qty, _) in self.depth(opposite_side) {
                if !limit_allows(taker_side, price, limit_price) {
                    limit_stopped = true;
                    break;
                }

                let take = remaining.min(level_qty);
                notional += price as u128 * take as u128;
                remaining -= take;

                if remaining == 0 {
                    break;
                }
            }
        }

        let filled_qty = qty - remaining;
        let average_price = if filled_qty > 0 {
            Some(notional as f64 / filled_qty as f64)
        } else {
            None
        };
        let slippage = match (average_price, reference_price) {
            (Some(avg), Some(reference)) => Some(match taker_side {
                Side::Bid => avg - reference as f64,
                Side::Ask => reference as f64 - avg,
            }),
            _ => None,
        };
        let slippage_pct = match (slippage, reference_price) {
            (Some(slip), Some(reference)) if reference > 0 => Some(slip / reference as f64 * 100.0),
            _ => None,
        };
        let slippage_notional = if filled_qty > 0 {
            reference_price.map(|reference| {
                let reference_notional = reference as u128 * filled_qty as u128;
                match taker_side {
                    Side::Bid => notional.saturating_sub(reference_notional),
                    Side::Ask => reference_notional.saturating_sub(notional),
                }
            })
        } else {
            None
        };

        SlippageEstimate {
            taker_side,
            requested_qty: qty,
            filled_qty,
            unfilled_qty: remaining,
            filled_notional: notional,
            limit_price,
            reference_price,
            average_price,
            slippage,
            slippage_notional,
            slippage_pct,
            limit_stopped,
            exhausted_book: remaining > 0 && !limit_stopped,
        }
    }

    /// Match a hypothetical taker limit order against the opposite side without
    /// mutating the book.
    ///
    /// The walk is deterministic: asks are consumed from lowest to highest for
    /// a bid taker, bids are consumed from highest to lowest for an ask taker,
    /// and orders inside each price level are consumed in FIFO order.
    pub fn match_taker_order(&self, taker_side: Side, qty: Qty, limit_price: Price) -> TakerMatch {
        let reference_price = match taker_side {
            Side::Bid => self.best_ask(),
            Side::Ask => self.best_bid(),
        };
        let opposite_side = match taker_side {
            Side::Bid => Side::Ask,
            Side::Ask => Side::Bid,
        };

        let mut remaining = qty;
        let mut notional = 0u128;
        let mut fills = Vec::new();
        let mut limit_stopped = false;

        if remaining > 0 {
            for (price, _, _) in self.depth(opposite_side) {
                if !limit_allows(taker_side, price, limit_price) {
                    limit_stopped = true;
                    break;
                }

                for maker in self.orders_at(opposite_side, price) {
                    if remaining == 0 {
                        break;
                    }

                    let filled_qty = remaining.min(maker.qty);
                    let maker_qty_after = maker.qty - filled_qty;
                    fills.push(FillDelta {
                        maker_order_id: maker.id,
                        maker_wallet: maker.wallet,
                        maker_side: maker.side,
                        price: maker.price,
                        filled_qty,
                        maker_qty_before: maker.qty,
                        maker_qty_after,
                        maker_removed: maker_qty_after == 0,
                    });
                    notional += maker.price as u128 * filled_qty as u128;
                    remaining -= filled_qty;
                }

                if remaining == 0 {
                    break;
                }
            }
        }

        let filled_qty = qty - remaining;
        let average_price = if filled_qty > 0 {
            Some(notional as f64 / filled_qty as f64)
        } else {
            None
        };

        TakerMatch {
            taker_side,
            requested_qty: qty,
            filled_qty,
            unfilled_qty: remaining,
            filled_notional: notional,
            limit_price,
            reference_price,
            average_price,
            fills,
            limit_stopped,
            exhausted_book: remaining > 0 && !limit_stopped,
        }
    }

    /// Apply a taker limit order to the book, returning the same deterministic
    /// fill deltas as [`match_taker_order`](Self::match_taker_order).
    pub fn apply_taker_order(
        &mut self,
        taker_side: Side,
        qty: Qty,
        limit_price: Price,
    ) -> Result<TakerMatch, BookError> {
        if qty == 0 {
            return Err(BookError::ZeroQty);
        }
        let taker_match = self.match_taker_order(taker_side, qty, limit_price);
        for fill in &taker_match.fills {
            if fill.maker_removed {
                self.remove(fill.maker_order_id)?;
            } else {
                self.update_size(fill.maker_order_id, fill.maker_qty_after)?;
            }
        }
        Ok(taker_match)
    }

    /// Submit a limit order using a simulator policy.
    ///
    /// This is a local deterministic simulator helper, not a venue-matching
    /// engine. Rejected FOK/PostOnly submissions do not mutate the book.
    pub fn submit_limit_order(
        &mut self,
        order: Order,
        policy: LimitOrderPolicy,
    ) -> Result<SubmitLimitOrderOutcome, BookError> {
        if order.qty == 0 {
            return Err(BookError::ZeroQty);
        }
        if self.order_index.contains_key(&order.id) {
            return Err(BookError::DuplicateOrderId(order.id));
        }

        match policy {
            LimitOrderPolicy::Gtc => {
                let taker_match = self.apply_taker_order(order.side, order.qty, order.price)?;
                let rested_order = if taker_match.unfilled_qty > 0 {
                    let rested = Order {
                        qty: taker_match.unfilled_qty,
                        ..order
                    };
                    self.add(rested)?;
                    Some(rested)
                } else {
                    None
                };
                Ok(SubmitLimitOrderOutcome {
                    policy,
                    rejected: None,
                    taker_match,
                    rested_order,
                })
            }
            LimitOrderPolicy::Ioc => {
                let taker_match = self.apply_taker_order(order.side, order.qty, order.price)?;
                Ok(SubmitLimitOrderOutcome {
                    policy,
                    rejected: None,
                    taker_match,
                    rested_order: None,
                })
            }
            LimitOrderPolicy::Fok => {
                let simulated = self.match_taker_order(order.side, order.qty, order.price);
                if !simulated.is_complete() {
                    return Ok(SubmitLimitOrderOutcome {
                        policy,
                        rejected: Some(SubmitRejectReason::FillOrKillWouldNotFill),
                        taker_match: simulated,
                        rested_order: None,
                    });
                }
                let taker_match = self.apply_taker_order(order.side, order.qty, order.price)?;
                Ok(SubmitLimitOrderOutcome {
                    policy,
                    rejected: None,
                    taker_match,
                    rested_order: None,
                })
            }
            LimitOrderPolicy::PostOnly => {
                let taker_match = self.match_taker_order(order.side, order.qty, order.price);
                if self.limit_order_would_cross(order.side, order.price) {
                    return Ok(SubmitLimitOrderOutcome {
                        policy,
                        rejected: Some(SubmitRejectReason::PostOnlyWouldCross),
                        taker_match,
                        rested_order: None,
                    });
                }
                self.add(order)?;
                Ok(SubmitLimitOrderOutcome {
                    policy,
                    rejected: None,
                    taker_match,
                    rested_order: Some(order),
                })
            }
        }
    }

    /// Iterate levels of a side in priority order (best first), yielding
    /// `(price, total_qty, order_count)`.
    pub fn depth(&self, side: Side) -> DepthIter<'_> {
        DepthIter(match side {
            Side::Bid => DepthIterInner::Desc(self.bids.iter().rev()),
            Side::Ask => DepthIterInner::Asc(self.asks.iter()),
        })
    }

    /// Iterate orders at a single price level in FIFO order. Returns an empty
    /// iterator if the level doesn't exist.
    /// Returns `(total_qty, order_count)` for one price level, or `None` when
    /// no orders rest there. O(log levels); does not touch individual orders.
    pub fn level_summary(&self, side: Side, price: Price) -> Option<(Qty, u64)> {
        let levels = match side {
            Side::Bid => &self.bids,
            Side::Ask => &self.asks,
        };
        levels
            .get(&price)
            .map(|level| (level.total_qty, level.order_count))
    }

    pub fn orders_at(&self, side: Side, price: Price) -> OrdersAtLevel<'_> {
        let levels = match side {
            Side::Bid => &self.bids,
            Side::Ask => &self.asks,
        };
        let head = levels.get(&price).and_then(|l| l.head);
        OrdersAtLevel {
            slab: &self.slab,
            next: head,
        }
    }

    /// Iterate every order belonging to a wallet. Order is unspecified (hash
    /// set iteration); callers that need sorted output can collect and sort.
    pub fn orders_by_wallet(&self, wallet: WalletId) -> WalletIter<'_> {
        let inner = self.wallet_index.get(&wallet).map(|s| s.iter());
        WalletIter { book: self, inner }
    }

    /// Debug-only consistency check. Walks every level's linked list and
    /// verifies the cached `order_count` / `total_qty` match reality.
    #[cfg(debug_assertions)]
    pub fn assert_invariants(&self) {
        for (side, levels) in [(Side::Bid, &self.bids), (Side::Ask, &self.asks)] {
            for (price, level) in levels {
                let mut count = 0u64;
                let mut qty = 0u64;
                let mut cur = level.head;
                let mut last = None;
                while let Some(slot) = cur {
                    let node = self.slab[slot].as_ref().expect("live linked slot");
                    assert_eq!(node.order.side, side);
                    assert_eq!(node.order.price, *price);
                    count += 1;
                    qty += node.order.qty;
                    last = Some(slot);
                    cur = node.next;
                }
                assert_eq!(level.order_count, count, "order_count at price {price}");
                assert_eq!(level.total_qty, qty, "total_qty at price {price}");
                assert_eq!(level.tail, last);
            }
        }
        assert_eq!(
            self.order_index.len(),
            self.slab.iter().filter(|s| s.is_some()).count(),
        );
    }

    fn alloc_slot(&mut self) -> usize {
        if let Some(slot) = self.free_list.pop() {
            slot
        } else {
            self.slab.push(None);
            self.slab.len() - 1
        }
    }

    fn top_n_levels_aggregated_for_side(&self, side: Side, n: usize) -> Vec<AggregatedLevel> {
        self.depth(side)
            .take(n)
            .map(|(price, total_qty, order_count)| AggregatedLevel {
                price,
                total_qty,
                order_count,
            })
            .collect()
    }

    fn top_n_levels_for_side(&self, side: Side, n: usize) -> Vec<UnaggregatedLevel> {
        self.depth(side)
            .take(n)
            .map(|(price, _, _)| UnaggregatedLevel {
                price,
                orders: self
                    .orders_at(side, price)
                    .map(|order| LevelOrder {
                        id: order.id,
                        wallet: order.wallet,
                        qty: order.qty,
                        ts: order.ts,
                    })
                    .collect(),
            })
            .collect()
    }

    fn limit_order_would_cross(&self, side: Side, price: Price) -> bool {
        match side {
            Side::Bid => self.best_ask().is_some_and(|best_ask| best_ask <= price),
            Side::Ask => self.best_bid().is_some_and(|best_bid| best_bid >= price),
        }
    }

    fn synthetic_orders_in_book_order(&self) -> Vec<Order> {
        let mut orders = Vec::new();
        for side in [Side::Bid, Side::Ask] {
            for (price, _, _) in self.depth(side) {
                orders.extend(
                    self.orders_at(side, price)
                        .filter(|order| is_synthetic_order_id(order.id))
                        .copied(),
                );
            }
        }
        orders
    }
}

fn limit_allows(taker_side: Side, resting_price: Price, limit_price: Price) -> bool {
    match taker_side {
        Side::Bid => resting_price <= limit_price,
        Side::Ask => resting_price >= limit_price,
    }
}

fn validate_preserve_synthetic_snapshot(
    orders: &[Order],
    preserved: &[Order],
) -> Result<(), BookError> {
    let preserved_ids: HashSet<OrderId> = preserved.iter().map(|order| order.id).collect();
    let mut seen_snapshot_ids = HashSet::default();
    for order in orders {
        if order.qty == 0 {
            return Err(BookError::ZeroQty);
        }
        if preserved_ids.contains(&order.id) {
            return Err(BookError::DuplicateOrderId(order.id));
        }
        if is_synthetic_order_id(order.id) {
            return Err(BookError::SyntheticOrderIdInSnapshot(order.id));
        }
        if !seen_snapshot_ids.insert(order.id) {
            return Err(BookError::DuplicateOrderId(order.id));
        }
    }
    Ok(())
}

fn link_tail(slab: &mut [Option<OrderNode>], level: &mut Level, slot: usize) {
    if let Some(old_tail) = level.tail {
        slab[old_tail].as_mut().expect("tail live").next = Some(slot);
        slab[slot].as_mut().expect("new node live").prev = Some(old_tail);
    } else {
        level.head = Some(slot);
    }
    level.tail = Some(slot);
}

fn unlink(
    slab: &mut [Option<OrderNode>],
    level: &mut Level,
    prev: Option<usize>,
    next: Option<usize>,
) {
    match prev {
        Some(p) => slab[p].as_mut().expect("prev live").next = next,
        None => level.head = next,
    }
    match next {
        Some(n) => slab[n].as_mut().expect("next live").prev = prev,
        None => level.tail = prev,
    }
}

fn move_to_tail(slab: &mut [Option<OrderNode>], level: &mut Level, slot: usize) {
    if level.tail == Some(slot) {
        return;
    }
    let (prev, next) = {
        let node = slab[slot].as_ref().expect("moving node live");
        (node.prev, node.next)
    };
    unlink(slab, level, prev, next);
    {
        let node = slab[slot].as_mut().expect("moving node live");
        node.prev = None;
        node.next = None;
    }
    link_tail(slab, level, slot);
}

pub struct DepthIter<'a>(DepthIterInner<'a>);

enum DepthIterInner<'a> {
    Asc(btree_map::Iter<'a, Price, Level>),
    Desc(std::iter::Rev<btree_map::Iter<'a, Price, Level>>),
}

impl<'a> Iterator for DepthIter<'a> {
    type Item = (Price, Qty, u64);
    fn next(&mut self) -> Option<Self::Item> {
        let (price, level) = match &mut self.0 {
            DepthIterInner::Asc(it) => it.next()?,
            DepthIterInner::Desc(it) => it.next()?,
        };
        Some((*price, level.total_qty, level.order_count))
    }
}

pub struct OrdersAtLevel<'a> {
    slab: &'a [Option<OrderNode>],
    next: Option<usize>,
}

impl<'a> Iterator for OrdersAtLevel<'a> {
    type Item = &'a Order;
    fn next(&mut self) -> Option<Self::Item> {
        let slot = self.next?;
        let node = self.slab[slot].as_ref()?;
        self.next = node.next;
        Some(&node.order)
    }
}

pub struct WalletIter<'a> {
    book: &'a OrderBook,
    inner: Option<std::collections::hash_set::Iter<'a, OrderId>>,
}

impl<'a> Iterator for WalletIter<'a> {
    type Item = &'a Order;
    fn next(&mut self) -> Option<Self::Item> {
        let id = *self.inner.as_mut()?.next()?;
        self.book.get(id)
    }
}
