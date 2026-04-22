<!--
Protocol reference for Dwellir's Hyperliquid L4 WebSocket feed. The body below
is verbatim from Dwellir's public documentation and is included here so the
`scripts/capture_l4.py` script and `src/dwellir.rs` adapter can be reviewed
against the spec they target. Endpoint URL (contains a per-account token) is
NOT included — set it via `DWELLIR_WS_URL` or `--endpoint`.
-->

# L4 Order Book - Individual Order Visibility

The most detailed market data available - individual order visibility with user wallet addresses, order IDs, timestamps, and full order parameters. L4 data enables queue position tracking, whale watching, and advanced market microstructure analysis.

Clone our complete examples: [github.com/dwellir-public/hyperliquid-orderbook-server-code-examples](https://github.com/dwellir-public/hyperliquid-orderbook-server-code-examples)

## How to Subscribe

Send a subscription message to the WebSocket endpoint:

```json
{
  "method": "subscribe",
  "subscription": {
    "type": "l4Book",
    "coin": "BTC"
  }
}
```

### Subscription Parameters

| Parameter | Type   | Required | Description                                                          |
| --------- | ------ | -------- | -------------------------------------------------------------------- |
| `type`    | string | Yes      | Must be `"l4Book"`                                                   |
| `coin`    | string | Yes      | Trading pair symbol (e.g., `"BTC"`, `"ETH"`, `"xyz:MSTR"`, `"@150"`) |

For HIP3 (permissionless perpetuals) markets, use the full coin label format with the `xyz:` prefix. For example, use `"xyz:MSTR"` for the MicroStrategy perpetual, not just `"MSTR"`. Standard perpetuals like BTC and ETH do not require a prefix.

For spot markets, use the `@{index}` format where the index is the spot asset index. For example, use `"@150"` for a specific spot market.

Unlike L2, L4 subscriptions do not support `nLevels` or `nSigFigs` - you receive the complete order book with all individual orders.

The first websocket message after subscribing is a `subscriptionResponse`; the next payload contains either a `Snapshot` or `Updates` wrapper.

## Message Types

L4 subscriptions produce two types of messages:

1. **Initial Snapshot** - Complete order book state when you subscribe
2. **Incremental Updates** - Changes as orders are placed, modified, or filled

### Initial Snapshot Response

When you first subscribe, you receive a complete snapshot of the order book wrapped in a `Snapshot` object:

```json
{
  "channel": "l4Book",
  "data": {
    "Snapshot": {
      "coin": "BTC",
      "height": 854890775,
      "levels": [
        [
          {
            "user": "0xf9109ada2f73c62e9889b45453065f0d99260a2d",
            "coin": "BTC",
            "side": "B",
            "limitPx": "90057",
            "sz": "0.33289",
            "oid": 289682065711,
            "timestamp": 1767878782721,
            "triggerCondition": "N/A",
            "isTrigger": false,
            "triggerPx": "0.0",
            "isPositionTpsl": false,
            "reduceOnly": false,
            "orderType": "Limit",
            "tif": "Alo",
            "cloid": "0x4c4617dbd8b94d358285c5c6d5a43df3"
          }
        ],
        [
          {
            "user": "0x13558be785661958932ceac35ba20de187275a42",
            "coin": "BTC",
            "side": "A",
            "limitPx": "90058",
            "sz": "0.37634",
            "oid": 289682176026,
            "timestamp": 1767878800615,
            "triggerCondition": "N/A",
            "isTrigger": false,
            "triggerPx": "0.0",
            "isPositionTpsl": false,
            "reduceOnly": false,
            "orderType": "Limit",
            "tif": "Alo",
            "cloid": "0x000000000814768000001999b6671c90"
          }
        ]
      ]
    }
  }
}
```

A full BTC order book snapshot can be large. The example above shows one order per side for brevity. Many WebSocket libraries default to a small message limit. If your connection closes immediately after subscribing, increase the maximum message size (for example, `max_size=50 * 1024 * 1024` in Python's `websockets` library).

### Incremental Update Response

After the initial snapshot, you receive incremental updates wrapped in an `Updates` object containing [`order_statuses`](#orderstatus-object-in-order_statuses-array) and [`book_diffs`](#bookdiff-object-in-book_diffs-array).

**Example 1: New Order**

This example shows a new order being added to the book:

```json
{
  "channel": "l4Book",
  "data": {
    "Updates": {
      "time": 1767878802703,
      "height": 854890776,
      "order_statuses": [
        {
          "time": "2026-01-08T13:26:42.703377851",
          "user": "0xbc927e87d072dfac3693846a83fa6922cc6c5f2a",
          "status": "open",
          "order": {
            "user": null,
            "coin": "BTC",
            "side": "B",
            "limitPx": "90056.0",
            "sz": "0.00014",
            "oid": 289682192129,
            "timestamp": 1767878802703,
            "triggerCondition": "N/A",
            "isTrigger": false,
            "triggerPx": "0.0",
            "isPositionTpsl": false,
            "reduceOnly": false,
            "orderType": "Limit",
            "tif": "Alo",
            "cloid": "0xa097c34ee13a42a1afeed2a5ce96b413"
          }
        }
      ],
      "book_diffs": [
        {
          "user": "0xbc927e87d072dfac3693846a83fa6922cc6c5f2a",
          "oid": 289682192129,
          "px": "90056.0",
          "coin": "BTC",
          "raw_book_diff": {
            "new": {
              "sz": "0.00014"
            }
          }
        }
      ]
    }
  }
}
```

**Example 2: Order Size Update (Partial Fill)**

This example shows an existing order being partially filled, with the size decreasing from 108.65 to 107.5:

```json
{
  "channel": "l4Book",
  "data": {
    "Updates": {
      "time": 1767878902834,
      "height": 854890880,
      "order_statuses": [],
      "book_diffs": [
        {
          "user": "0x97991003fd631e2923f40cab2a4fdc35e60dc807",
          "oid": 316542552323,
          "px": "84.371",
          "coin": "SOL",
          "raw_book_diff": {
            "update": {
              "origSz": "108.65",
              "newSz": "107.5"
            }
          }
        }
      ]
    }
  }
}
```

In this update:

- The order at price `84.371` was partially filled
- Original size (`origSz`): `108.65` SOL
- New remaining size (`newSz`): `107.5` SOL

## Response Field Reference

This section provides a detailed breakdown of all fields in L4 messages. For specific field values like order statuses, see [Order Status Values](#order-status-values).

All L4 messages follow this top-level structure:

```typescript
{
  channel: "l4Book",
  data: Snapshot | Updates
}
```

### Message Type 1: Snapshot (Initial State)

**Structure:** `{ channel: "l4Book", data: { Snapshot: {...} } }`

Received once when you first subscribe. Contains the complete order book state.

#### Snapshot Object

| Field    | Type                 | Description                                                                   |
| -------- | -------------------- | ----------------------------------------------------------------------------- |
| `coin`   | `string`             | Trading pair symbol (e.g., `"BTC"`, `"ETH"`, `"xyz:MSTR"`, `"@150"`)          |
| `height` | `number`             | Hyperliquid block height                                                      |
| `levels` | `[Order[], Order[]]` | Two-element array: `[bids, asks]`. Each element is an array of Order objects. |

#### Order Object (in `levels` arrays)

Each order in the `levels[0]` (bids) and `levels[1]` (asks) arrays contains:

| Field              | Type                      | Description                                                                                           |
| ------------------ | ------------------------- | ----------------------------------------------------------------------------------------------------- |
| `user`             | `string`                  | Wallet address of order owner (e.g., `"0xf9109ada..."`)                                               |
| `coin`             | `string`                  | Trading pair (e.g., `"BTC"`)                                                                          |
| `side`             | `"B" \| "A"`              | `"B"` = Bid (buy), `"A"` = Ask (sell)                                                                 |
| `limitPx`          | `string`                  | Limit price (e.g., `"90057"`)                                                                         |
| `sz`               | `string`                  | Remaining order size (e.g., `"0.33289"`)                                                              |
| `oid`              | `number`                  | Unique order ID (e.g., `289682065711`)                                                                |
| `timestamp`        | `number`                  | Order placement time in Unix milliseconds (e.g., `1767878782721`)                                     |
| `triggerCondition` | `string`                  | Trigger condition type, or `"N/A"` if not a trigger order                                             |
| `isTrigger`        | `boolean`                 | Whether this is a trigger/stop order                                                                  |
| `triggerPx`        | `string`                  | Trigger price, or `"0.0"` if not a trigger order                                                      |
| `isPositionTpsl`   | `boolean`                 | Whether this is a position take-profit/stop-loss order                                                |
| `reduceOnly`       | `boolean`                 | Whether order can only reduce position                                                                |
| `orderType`        | `string`                  | Order type: `"Limit"`, `"Market"`, `"Stop Market"`, `"Stop Limit"`, `"Scale"`, `"TWAP"`               |
| `tif`              | `"Gtc" \| "Ioc" \| "Alo"` | Time-in-force: `"Gtc"` (Good til Cancel), `"Ioc"` (Immediate or Cancel), `"Alo"` (Add Liquidity Only) |
| `cloid`            | `string`                  | Client order ID - hex string provided by user (e.g., `"0x4c4617dbd8b94d35..."`)                       |

### Message Type 2: Updates (Incremental Changes)

**Structure:** `{ channel: "l4Book", data: { Updates: {...} } }`

Received continuously after the snapshot. Contains incremental changes to the order book.

#### Updates Object

| Field            | Type            | Description                                                                                                                                                                                       |
| ---------------- | --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `time`           | `number`        | Unix timestamp in milliseconds (e.g., `1767878802703`)                                                                                                                                            |
| `height`         | `number`        | Hyperliquid block height - increments with each update                                                                                                                                            |
| `order_statuses` | `OrderStatus[]` | Array of [order status changes](#orderstatus-object-in-order_statuses-array) (new orders, fills, cancellations, rejections). See [Order Status Values](#order-status-values) for status meanings. |
| `book_diffs`     | `BookDiff[]`    | Array of [order book modifications](#bookdiff-object-in-book_diffs-array) (additions, removals, size changes)                                                                                     |

The `order_statuses` array contains **status transitions** (open, filled, canceled), not per-fill execution data. To get individual fill details (price, size, counterparty), subscribe to the [Trades stream](https://www.dwellir.com/docs/hyperliquid/trades) and correlate using `oid`. See [Understanding Order Data Flow](#understanding-order-data-flow) for details.

#### OrderStatus Object (in `order_statuses` array)

| Field    | Type     | Description                                                                                                                                                              |
| -------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `time`   | `string` | ISO-8601 timestamp with nanosecond precision (e.g., `"2026-01-08T13:26:42.703377851"`)                                                                                   |
| `user`   | `string` | Wallet address of order owner                                                                                                                                            |
| `status` | `string` | Order status: `"open"`, `"filled"`, `"canceled"`, `"badAloPxRejected"`, etc. See [Order Status Values](#order-status-values) for all possible values and their meanings. |
| `order`  | `Order`  | Order object with same structure as [Order Object](#order-object-in-levels-arrays) above (but `user` field may be `null`)                                                |

#### BookDiff Object (in `book_diffs` array)

The `book_diffs` array contains changes to individual orders in the book. Each diff describes what happened to a specific order - whether it was added (`new`), partially filled (`update`), modified (`modified`), or removed (`remove`).

**Common fields present in all diff types:**

| Field           | Type                                                | Description                                                                                                                                                                                                                                                                                                  |
| --------------- | --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `user`          | `string`                                            | Wallet address of order owner (e.g., `"0xbc927e87..."`)                                                                                                                                                                                                                                                      |
| `oid`           | `number`                                            | Unique order ID being modified (e.g., `289682192129`)                                                                                                                                                                                                                                                        |
| `px`            | `string`                                            | Price level where order exists/existed (e.g., `"90056.0"`)                                                                                                                                                                                                                                                   |
| `coin`          | `string`                                            | Trading pair (e.g., `"BTC"`)                                                                                                                                                                                                                                                                                 |
| `raw_book_diff` | `NewDiff \| UpdateDiff \| ModifiedDiff \| "remove"` | Describes what changed. Can be: `{ new: { sz } }` for new orders, `{ update: { origSz, newSz } }` for partial fills, `{ modified: { sz } }` for amendments, or `"remove"` for cancellations/complete fills. See [BookDiff Types](#bookdiff-types-the-raw_book_diff-field) below for detailed specifications. |

### BookDiff Types (the `raw_book_diff` field)

The `raw_book_diff` field indicates what happened to an order. It can take four forms:

| Type             | TypeScript Definition                           | Description                                                                                                                                                                                                                                                                          |
| ---------------- | ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **NewDiff**      | `{ new: { sz: string } }`                       | New order added to the book. The `sz` field contains the order size. Reference the corresponding entry in [`order_statuses`](#orderstatus-object-in-order_statuses-array) with matching `oid` to get full order details (side, tif, etc.).                                           |
| **RemoveDiff**   | `"remove"`                                      | Order completely removed from book. Can occur due to: full fill (check [`order_statuses`](#orderstatus-object-in-order_statuses-array) for [`filled`](#successful-states) status), user cancellation ([`canceled`](#cancellation-states)), system cancellation, or order expiration. |
| **UpdateDiff**   | `{ update: { origSz: string, newSz: string } }` | Order size changed (usually decreased due to partial fill). Contains both the original size (`origSz`) and the new remaining size (`newSz`).                                                                                                                                         |
| **ModifiedDiff** | `{ modified: { sz: string } }`                  | Order size modified (typically from order amendments). The `sz` field contains the new remaining size. Unlike `UpdateDiff`, this only provides the new size without the original.                                                                                                    |

### Order Status Values

The `status` field in `order_statuses` indicates the result of order processing. Understanding these statuses is critical for debugging order placement issues.

| Status                                      | Type         | Description                                                               | Debugging Notes                                                                                                                                                                                                                                                                                                         |
| ------------------------------------------- | ------------ | ------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `open`                                      | Success      | Order successfully placed and resting on the book                         | Most common successful status. Order is live and can be filled.                                                                                                                                                                                                                                                         |
| `filled`                                    | Success      | Order fully executed                                                      | Order matched completely. Check `book_diffs` for removal. **Note:** the `sz` field in the order object may be non-zero even for `filled` status. This reflects remaining size at the time the status was emitted by the Hyperliquid node, not necessarily the final state. Use `book_diffs` for accurate size tracking. |
| `triggered`                                 | Success      | Trigger/stop order activated                                              | Conditional order has been triggered and converted to regular order.                                                                                                                                                                                                                                                    |
| `canceled`                                  | Cancellation | Order canceled by user or system                                          | Standard cancellation. Check if user-initiated or system-triggered.                                                                                                                                                                                                                                                     |
| `reduceOnlyCanceled`                        | Cancellation | Reduce-only order was canceled                                            | Position closed or order would have increased position instead of reducing.                                                                                                                                                                                                                                             |
| `selfTradeCanceled`                         | Cancellation | Order canceled to prevent self-trading                                    | Same user's buy and sell orders would have matched. Exchange prevented self-execution.                                                                                                                                                                                                                                  |
| `marginCanceled`                            | Cancellation | Order canceled due to insufficient margin                                 | User's margin balance insufficient to maintain the order.                                                                                                                                                                                                                                                               |
| `openInterestCapCanceled`                   | Cancellation | Order canceled due to open interest cap reached                           | Market has reached maximum open interest limit. Try again later or use different market.                                                                                                                                                                                                                                |
| `scheduledCancel`                           | Cancellation | Order canceled on a scheduled basis                                       | Order was automatically canceled based on time or condition schedule.                                                                                                                                                                                                                                                   |
| `siblingFilledCanceled`                     | Cancellation | Order canceled because sibling order filled                               | Paired/bracket order canceled when the primary order executed. Common with OCO (One-Cancels-Other) orders.                                                                                                                                                                                                              |
| `badAloPxRejected`                          | Rejection    | Add-liquidity-only order rejected (never reached book)                    | **Most common rejection** (\~70% in production data). ALO order would have crossed spread and executed as taker. Price was too aggressive for maker-only order.                                                                                                                                                         |
| `iocCancelRejected`                         | Rejection    | Immediate-or-cancel order rejected (never reached book)                   | IOC order couldn't fill immediately at specified price. No matching liquidity available.                                                                                                                                                                                                                                |
| `perpMarginRejected`                        | Rejection    | Perpetual futures order rejected (never reached book)                     | Insufficient margin to open the position. Check account balance and leverage.                                                                                                                                                                                                                                           |
| `perpMaxPositionRejected`                   | Rejection    | Perpetual order rejected - exceeds max position size (never reached book) | Order would exceed maximum allowed position size for this market. Reduce order size or close existing positions.                                                                                                                                                                                                        |
| `minTradeNtlRejected`                       | Rejection    | Minimum notional value rejected (never reached book)                      | Order size (price × quantity) below exchange minimum. Increase order size.                                                                                                                                                                                                                                              |
| `reduceOnlyRejected`                        | Rejection    | Reduce-only order rejected (never reached book)                           | Order marked reduce-only but would have increased position or no position exists to reduce.                                                                                                                                                                                                                             |
| `insufficientSpotBalanceRejected`           | Rejection    | Insufficient spot token balance (never reached book)                      | Not enough spot token balance to place order. Deposit more tokens or reduce order size.                                                                                                                                                                                                                                 |
| `oracleRejected`                            | Rejection    | Order rejected due to oracle price issues (never reached book)            | Oracle price feed unavailable or stale. Wait for oracle to update or check market status.                                                                                                                                                                                                                               |
| `positionFlipAtOpenInterestCapRejected`     | Rejection    | Position flip rejected at open interest cap (never reached book)          | Order would flip position direction when market is at OI cap. Close existing position first.                                                                                                                                                                                                                            |
| `positionIncreaseAtOpenInterestCapRejected` | Rejection    | Position increase rejected at open interest cap (never reached book)      | Cannot increase position size when market has reached open interest limit.                                                                                                                                                                                                                                              |
| `tooAggressiveAtOpenInterestCapRejected`    | Rejection    | Order too aggressive at open interest cap (never reached book)            | Order price too aggressive when market near OI cap. Use less aggressive limit price.                                                                                                                                                                                                                                    |

## Understanding Order Data Flow

Hyperliquid separates order data across two channels. Understanding this separation is essential for building correct trading systems.

| Data                                                   | Channel                                                  | What you get                                                     |
| ------------------------------------------------------ | -------------------------------------------------------- | ---------------------------------------------------------------- |
| Order status transitions (open, filled, canceled)      | **L4 Book**                                              | When an order changes state, not how it was filled               |
| Book mutations (new, update, remove)                   | **L4 Book**                                              | Size changes at each price level per order                       |
| Individual fill executions (price, size, counterparty) | **[Trades](https://www.dwellir.com/docs/hyperliquid/trades)** | Each execution with price, size, hash, and both wallet addresses |

**For complete order visibility, subscribe to both channels** and correlate events using the `oid` (order ID) field:

```python
# Subscribe to both channels for full order tracking
await websocket.send(json.dumps({
    "method": "subscribe",
    "subscription": {"type": "l4Book", "coin": "ETH"}
}))
await websocket.send(json.dumps({
    "method": "subscribe",
    "subscription": {"type": "trades", "coin": "ETH"}
}))

# Correlate: l4Book order_statuses use "oid" in the order object,
# trades use "tid" (trade ID) but can be matched to orders by
# tracking which orders are open at each block height.
```

### Why "only 2 events" is normal

A common question is why an order shows only `open` then `filled` with no events in between. This is expected behavior. The L4 Book `order_statuses` array reports **state transitions**, not individual fills.

If an order receives 5 partial fills before completing, the L4 Book reports:

1. `open` - order placed on the book
2. `filled` - order fully executed

The 5 individual fills appear on the **Trades** stream, not in `order_statuses`. The `book_diffs` array tracks the size decreases from partial fills as `update` diffs.

## Order Lifecycle Patterns

Based on production data for ETH, these are the most common order lifecycle patterns on the L4 Book channel:

| Pattern                               | Frequency              | Description                                                                                         |
| ------------------------------------- | ---------------------- | --------------------------------------------------------------------------------------------------- |
| Single rejection event                | \~88% of all orders    | Orders rejected immediately (`badAloPxRejected`, `perpMarginRejected`, etc.). Never reach the book. |
| `open` then `canceled`                | \~98.9% of multi-event | Order placed and later canceled by the user or system                                               |
| `open` then `filled`                  | \~1.1% of multi-event  | Order placed and fully executed (may happen in the same block)                                      |
| `triggered` then `filled`             | Rare                   | Stop/trigger order activated and then filled                                                        |
| `open` then `triggered` then `filled` | Very rare              | Order transitions through triggered state before filling                                            |

Most orders on Hyperliquid are **ALO (Add Liquidity Only)** orders from market makers that get rejected because the price crossed the spread. This is normal high-frequency trading behavior.

## Request Parameters

- `type` (`string, required`): Subscription parameter: Must be `"l4Book"`
- `coin` (`string, required`): Subscription parameter: Trading pair symbol (e.g., `"BTC"`, `"ETH"`, `"xyz:MSTR"`, `"@150"`)

## Request Example

```json
{
  "method": "subscribe",
  "subscription": {
    "type": "l4Book",
    "coin": "BTC"
  }
}
```

## Response Fields

- `channel` (`string, required`): Always `"l4Book"` for this subscription type
- `data.Snapshot` (`object, optional`): Initial full book snapshot after subscribing
- `data.Updates` (`object, optional`): Incremental order book changes after the snapshot

## Successful Response

```json
{
  "channel": "l4Book",
  "data": {
    "Snapshot": {
      "coin": "BTC",
      "height": 927625914,
      "levels": [[], []]
    }
  }
}
```

## Use Cases

### Queue Position Optimization

Understand your place in the order queue:

- **Time priority**: See exactly where your order sits at a price level
- **Fill probability**: Estimate likelihood of execution based on orders ahead
- **Repositioning**: Decide when to cancel and replace for better position

### Whale Wallet Tracking

Monitor large or notable traders:

- **Address tracking**: Follow specific wallet addresses
- **Size alerts**: Trigger on orders above threshold
- **Pattern detection**: Identify accumulation or distribution

### Market Microstructure Research

Analyze order flow dynamics:

- **Order arrival rates**: Study how orders enter the book
- **Cancellation patterns**: Track order lifetime and modification frequency
- **Toxicity analysis**: Measure adverse selection from order flow

### Smart Order Routing

Optimize order execution strategy:

- **Liquidity mapping**: Know exactly what size exists at each level
- **Hidden liquidity**: Detect when large orders are being worked
- **Impact estimation**: Model expected slippage from current book state

## Bandwidth Considerations

L4 data is significantly higher bandwidth than L2:

| Aspect               | L2                    | L4                     |
| -------------------- | --------------------- | ---------------------- |
| Data per level       | 3 fields (px, sz, n)  | 15+ fields per order   |
| Orders visible       | Aggregated count only | Every individual order |
| Update frequency     | Per price level       | Per order change       |
| Typical message size | 1-5 KB                | 10-100+ KB             |

Consider subscribing to L4 only for coins where you need individual order visibility.


