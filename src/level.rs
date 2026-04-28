use crate::types::{Order, Qty};

/// Aggregate state for a single price level. Individual orders are held in the
/// book's slab and stitched into a doubly-linked list by `head`/`tail` so that
/// time priority (FIFO) is preserved and any order can be unlinked in O(1).
#[derive(Debug, Clone, Default)]
pub(crate) struct Level {
    pub order_count: u64,
    pub total_qty: Qty,
    pub head: Option<usize>,
    pub tail: Option<usize>,
}

impl Level {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct OrderNode {
    pub order: Order,
    pub next: Option<usize>,
    pub prev: Option<usize>,
}

impl OrderNode {
    pub fn new(order: Order) -> Self {
        OrderNode {
            order,
            next: None,
            prev: None,
        }
    }
}
