use std::collections::{BTreeMap, HashMap, HashSet};
use std::collections::btree_map;

use crate::level::{Level, OrderNode};
use crate::types::{BookError, Order, OrderId, Price, Qty, Side, WalletId};

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
    pub fn apply_snapshot<I: IntoIterator<Item = Order>>(&mut self, orders: I) -> Result<(), BookError> {
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
        self.wallet_index.entry(order.wallet).or_default().insert(order.id);

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
        let slot = self.order_index.remove(&id).ok_or(BookError::UnknownOrderId(id))?;
        let node = self.slab[slot].take().expect("slab slot present for indexed order");
        self.free_list.push(slot);

        let order = node.order;
        let levels = match order.side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        };
        let remove_level = {
            let level = levels.get_mut(&order.price).expect("level present for resting order");
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
        let slot = *self.order_index.get(&id).ok_or(BookError::UnknownOrderId(id))?;
        let (current, side, price) = {
            let node = self.slab[slot].as_ref().expect("slab slot present for indexed order");
            (node.order.qty, node.order.side, node.order.price)
        };
        if new_qty >= current {
            return Err(BookError::NonDecreasingSize { current, proposed: new_qty });
        }
        if new_qty == 0 {
            self.remove(id)?;
            return Ok(());
        }
        self.slab[slot].as_mut().expect("slab slot present").order.qty = new_qty;
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
        let slot = *self.order_index.get(&id).ok_or(BookError::UnknownOrderId(id))?;
        let (current, side, price) = {
            let node = self.slab[slot].as_ref().expect("slab slot present for indexed order");
            (node.order.qty, node.order.side, node.order.price)
        };
        if current == new_qty {
            return Ok(());
        }
        self.slab[slot].as_mut().expect("slab slot present").order.qty = new_qty;
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
        OrdersAtLevel { slab: &self.slab, next: head }
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
