# Trades

Print live trades as `time (UTC) | terminal | @handle | side | token mint | USD amount`. When a handle is unavailable, the trader's shortened wallet address is shown. USD amounts can be unavailable.

Install [Rust](https://www.rust-lang.org/tools/install) and register the running machine's public IP in your Rungst account. From the repository root, run with a trader or token mint filter:

```sh
cargo run -p trades -- --trader 11111111111111111111111111111111
cargo run -p trades -- --mint TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
```

These example addresses illustrate the format and may have no activity. Free Trial subscriptions require a filter; paid plans can omit it with `cargo run -p trades`. Press Ctrl+C to stop.

The connection uses registered IP authorization, consumes credits even when idle, and resubscribes after disconnects. Missed trades are not replayed. See the [trade stream documentation](https://docs.rungst.com/streams/trades) for details.
