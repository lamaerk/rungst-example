# Thesis momentum (experimental)

This example is designed for pump.fun tokens with a direct SOL/token pair only. It reads the on-chain bonding curve to choose the curve or migrated PumpSwap pool. It builds Pump instructions locally from the published IDLs and simulates a quote before setting a 5% minimum-output limit. Cashback bonding curves are skipped.

This example listens for callouts and reports a token when it has at least three messages and a market cap of $40,000 or less. Set `THESIS_COUNT` or `MAX_MCAP_USD` to change these limits.

## Pipeline

The code follows a short event-to-action pipeline:

1. **Collect:** `Collector` subscribes to Solana callouts and turns incoming messages into `Callout` values.
2. **Decide:** `Strategy` counts repeated callouts for each pump.fun mint, applies the market-cap limit, and emits a buy candidate. While a position is open, it checks matching callouts for the take-profit or stop-loss thresholds.
3. **Execute:** `main` handles the candidate according to the selected mode: observe only, simulate a buy, or trade live. The executor opens a position, monitors it, and closes it on a threshold or timeout.
4. **Build and protect:** The instruction module reads the mint's on-chain state, picks the bonding curve or migrated PumpSwap pool, simulates the exact swap, and rebuilds it with a 5% minimum-output limit before submission.

Each stage has a narrow role so the event flow can be followed from the WebSocket message to the resulting action without mixing collection, strategy rules, and transaction details.

Register your IP with Rungst and set `MOMENTUM_KEYPAIR` to a Solana JSON keypair file. Run from the repository root:

```sh
cargo run -p thesis-momentum
```

To simulate a buy without submitting it, run:

```sh
cargo run -p thesis-momentum -- --simulation
```

To enable live buying and selling, add `--buy`. This uses real funds.
The example buys with 0.01 SOL and sells based on callout market cap changes or after ten minutes. The wallet also needs SOL for transaction fees and token-account rent.
This is just a demonstration and should not be used for a real trading strategy. Use at your own risk.
