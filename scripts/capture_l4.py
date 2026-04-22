"""Capture Hyperliquid L4 order book data from Dwellir for benchmarking.

Writes JSONL where the first line is a capture-metadata header, followed by
one record per websocket message:

    {"recv_ns": <int>, "seq": <int>, "msg": <parsed json>}

`recv_ns` is host-monotonic wall-clock ns at receive time (time.time_ns()).
The benchmark harness replays messages into the order book in order; the
snapshot is always the first non-subscriptionResponse message.

Supply your Dwellir endpoint via `--endpoint` or the `DWELLIR_WS_URL`
environment variable. The Dwellir URL contains a per-account token, so it is
never checked into this repository.
"""

import argparse
import asyncio
import json
import os
import signal
import sys
import time
from pathlib import Path

import websockets


async def capture(endpoint: str, coin: str, duration_s: float, out_path: Path) -> dict:
    sub_msg = {
        "method": "subscribe",
        "subscription": {"type": "l4Book", "coin": coin},
    }

    stats = {
        "messages": 0,
        "subscription_responses": 0,
        "snapshots": 0,
        "updates": 0,
        "other": 0,
        "bytes": 0,
    }

    # 50 MB max frame as the Dwellir docs recommend; snapshots are big.
    async with websockets.connect(
        endpoint,
        max_size=50 * 1024 * 1024,
        ping_interval=20,
        ping_timeout=20,
    ) as ws:
        await ws.send(json.dumps(sub_msg))

        with out_path.open("w", encoding="utf-8") as f:
            header = {
                "type": "capture_header",
                "endpoint": endpoint,
                "coin": coin,
                "subscription": sub_msg,
                "capture_start_ns": time.time_ns(),
                "capture_start_iso": time.strftime(
                    "%Y-%m-%dT%H:%M:%SZ", time.gmtime()
                ),
                "target_duration_s": duration_s,
                "schema_version": 1,
            }
            f.write(json.dumps(header) + "\n")
            f.flush()

            snapshot_received_ns = None
            seq = 0
            deadline = None  # set once we see the snapshot
            last_status_ns = time.monotonic_ns()

            while True:
                # Compute timeout until deadline (if set) so the receive
                # loop exits exactly at the 5-minute mark after snapshot.
                if deadline is not None:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        break
                    try:
                        raw = await asyncio.wait_for(ws.recv(), timeout=remaining)
                    except asyncio.TimeoutError:
                        break
                else:
                    # Before snapshot arrives, allow up to 30s.
                    raw = await asyncio.wait_for(ws.recv(), timeout=30)

                recv_ns = time.time_ns()
                stats["messages"] += 1
                stats["bytes"] += len(raw) if isinstance(raw, (bytes, bytearray, str)) else 0

                try:
                    msg = json.loads(raw)
                except json.JSONDecodeError:
                    msg = {"_raw": raw if isinstance(raw, str) else raw.decode("utf-8", "replace")}

                # Classify.
                channel = msg.get("channel") if isinstance(msg, dict) else None
                data = msg.get("data") if isinstance(msg, dict) else None
                kind = "other"
                if channel == "subscriptionResponse":
                    kind = "subscriptionResponse"
                    stats["subscription_responses"] += 1
                elif channel == "l4Book" and isinstance(data, dict):
                    if "Snapshot" in data:
                        kind = "Snapshot"
                        stats["snapshots"] += 1
                        if snapshot_received_ns is None:
                            snapshot_received_ns = recv_ns
                            # Start the 5-minute timer from the snapshot.
                            deadline = time.monotonic() + duration_s
                    elif "Updates" in data:
                        kind = "Updates"
                        stats["updates"] += 1
                    else:
                        stats["other"] += 1
                else:
                    stats["other"] += 1

                record = {
                    "recv_ns": recv_ns,
                    "seq": seq,
                    "kind": kind,
                    "msg": msg,
                }
                f.write(json.dumps(record, separators=(",", ":")) + "\n")
                seq += 1

                # Periodic flush + progress every ~5s.
                now_m = time.monotonic_ns()
                if now_m - last_status_ns > 5_000_000_000:
                    f.flush()
                    last_status_ns = now_m
                    elapsed = 0.0
                    if deadline is not None:
                        elapsed = duration_s - max(0.0, deadline - time.monotonic())
                    print(
                        f"[{elapsed:6.1f}s] msgs={stats['messages']} "
                        f"snap={stats['snapshots']} upd={stats['updates']} "
                        f"bytes={stats['bytes']:,}",
                        file=sys.stderr,
                        flush=True,
                    )

            f.flush()

        stats["snapshot_received_ns"] = snapshot_received_ns
        stats["capture_end_ns"] = time.time_ns()
        return stats


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--endpoint",
        default=os.environ.get("DWELLIR_WS_URL"),
        help="Dwellir WebSocket URL (defaults to $DWELLIR_WS_URL).",
    )
    ap.add_argument("--coin", default="BTC")
    ap.add_argument(
        "--duration",
        type=float,
        default=300.0,
        help="Seconds of updates to capture after the snapshot (default 300 = 5 min).",
    )
    ap.add_argument(
        "--out",
        type=Path,
        default=Path("benchmark_data/btc_l4_capture.jsonl"),
    )
    args = ap.parse_args()

    if not args.endpoint:
        ap.error(
            "no endpoint provided; pass --endpoint or set DWELLIR_WS_URL "
            "(e.g. wss://<your-dwellir-host>/<token>/ws)"
        )

    args.out.parent.mkdir(parents=True, exist_ok=True)

    # Graceful Ctrl-C: asyncio handles KeyboardInterrupt at the top level.
    def _sigterm(_sig, _frm):
        raise KeyboardInterrupt()

    signal.signal(signal.SIGTERM, _sigterm)

    print(
        f"Connecting to {args.endpoint}\n"
        f"Subscribing to l4Book coin={args.coin}, capturing {args.duration:.0f}s of updates.\n"
        f"Output: {args.out}",
        file=sys.stderr,
    )

    try:
        stats = asyncio.run(capture(args.endpoint, args.coin, args.duration, args.out))
    except KeyboardInterrupt:
        print("Interrupted; partial capture saved.", file=sys.stderr)
        return 130

    size_bytes = args.out.stat().st_size
    print(
        "\nCapture complete.\n"
        f"  file: {args.out} ({size_bytes:,} bytes on disk)\n"
        f"  messages: {stats['messages']}  "
        f"(snapshots={stats['snapshots']}, updates={stats['updates']}, "
        f"subResp={stats['subscription_responses']}, other={stats['other']})\n"
        f"  wire bytes received: {stats['bytes']:,}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
