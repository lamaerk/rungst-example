# Callouts

Subscribe to live Solana callouts and print their creation time (UTC), source terminal, author handle (or shortened wallet address), token symbol, and thesis.

## Run

1. Install [Rust](https://www.rust-lang.org/tools/install) and register the public IP address of the machine running the example in your Rungst account.
2. From the repository root, run the example with a token mint address:

   ```sh
   cargo run -p callouts -- 9FVtH4mtQt94ZpYR9xzZL8w65oM3G3AfE6G7ZCM3pump
   ```

   Paid plans can omit the address to receive all callouts: `cargo run -p callouts`. The Free Trial requires a mint filter. This example address illustrates the format and may have no activity.
3. Press Ctrl+C to stop.

The WebSocket is authorized by your registered IP; it does not require an API key or an environment file. Each open connection consumes credits, even while idle. The example reconnects and resubscribes after a disconnect; missed callouts are not replayed. Subscription errors are printed and stop the program so they can be corrected.

See the [Rungst callouts documentation](https://docs.rungst.com/streams/callouts) for message fields and subscription details.
