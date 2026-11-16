# Thesis momentum (experimental)

This example watches callouts for pump.fun tokens with a direct SOL/token pair. It selects the bonding curve or migrated PumpSwap pool, simulates a quote, and sets a 5% minimum-output limit. Cashback curves are skipped.

## Configuration

Register the running machine's public IP with Rungst. Set these environment variables before running; this example does not load a `.env` file.

| Variable | Default | Purpose |
| --- | --- | --- |
| `MOMENTUM_KEYPAIR` | Required | Path to a Solana JSON keypair, including in observation mode |
| `THESIS_COUNT` | `3` | Minimum callout count |
| `MAX_MCAP_USD` | `40000` | Maximum market cap in USD |

## Run

This example is not currently included in the root Cargo workspace. Add it to the workspace before running these commands from the repository root:

```sh
cargo run -p thesis-momentum                 # observe
cargo run -p thesis-momentum -- --simulation # simulate a buy
cargo run -p thesis-momentum -- --buy        # trade with real funds
```

Live mode buys with 0.01 SOL and sells based on market-cap changes or after ten minutes. The wallet also needs SOL for fees and rent. This is an experimental demonstration, not a production trading strategy.
