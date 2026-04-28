use std::collections::btree_map;
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::level::{Level, OrderNode};
use crate::types::{BookError, Order, OrderId, Price, Qty, Side, Ts, WalletId};

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
            order_index: HashMap::new(),
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            wallet_index: HashMap::new(),
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
        self.clear();
        for order in orders {
            self.add(order)?;
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.slab.clear();
        self.free_list.clear();
        self.order_index.clear();
        self.bids.clear();
        self.asks.clear();
        self.wallet_index.clear();
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
    /// Queue position is preserved (adapter can drop-and-re-add if the venue's
    /// policy is "amend-up loses priority"). `new_qty == 0` removes the order.
    pub fn amend_size(&mut self, id: OrderId, new_qty: Qty) -> Result<(), BookError> {
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
        } else {
            level.total_qty -= current - new_qty;
        }
        Ok(())
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
}

fn limit_allows(taker_side: Side, resting_price: Price, limit_price: Price) -> bool {
    match taker_side {
        Side::Bid => resting_price <= limit_price,
        Side::Ask => resting_price >= limit_price,
    }
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
