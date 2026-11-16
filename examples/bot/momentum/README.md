# Momentum bot

Observe Solana momentum and optionally execute one trade with Rungst. The bot uses two IP-authorized WebSocket streams and the Rungst trade build/submit API; it signs transactions locally using the same approach as the swaps example. Unfiltered streams require a paid plan and consume credits while connected. Register the machine's public IP first.

A candidate must meet **all** of these conditions for the same mint in the rolling window, measured from receipt of each event:

| Environment variable | Default | Meaning |
| --- | --- | --- |
| `WINDOW_SECS` | `30` | Window in seconds |
| `MIN_BUYERS` | `5` | Distinct buyer wallets |
| `MIN_BUY_RATIO` | `0.6` | Buy transactions divided by priced buy and sell transactions, from 0 to 1 |
| `MIN_NETFLOW_USD` | `100` | Buy USD volume minus sell USD volume |
| `MIN_CALLOUTS` | `1` | Number of callouts |

Only trades with a positive finite USD amount and a trader are counted; missing USD amounts are excluded from all trade metrics. At least one callout with a positive market cap must also be in the window. The latest such callout is the entry reference. Defaults for buyer count and buy ratio are examples; adjust them for your strategy. Each received event is counted once; duplicate stream events are not deduplicated.

```sh
cargo run -p momentum                 # observe candidates only
cargo run -p momentum -- --buy        # place real trades
```

For live mode, copy `.env.example` to `.env` in the working directory or set environment variables. Set `RUNGST_API_KEY`, `RUNGST_WALLET_FILE` (a local Solana CLI JSON keypair), and `BUY_AMOUNT_USDC`. Optionally set `RUNGST_SOLANA_RPC_URL` and `SLIPPAGE_BPS` (default 100). The wallet needs USDC and SOL for fees and rent. The bot refuses to buy if the associated token account already has a balance and sells the recorded amount. After a confirmed sale it resumes looking for candidates, with at most one position open at a time. `MAX_BUYS_PER_MINT` defaults to 1 and limits lifetime buy attempts per mint. Counts persist in `BUY_HISTORY_FILE` (default `momentum-buy-history.json` in the working directory); use the same file across restarts. A buy is counted before submission, including failed or uncertain attempts. The bot tracks its position only while running; use one instance at a time and manage any open position before restarting.

After a confirmed buy, subsequent callouts for the mint trigger a sale at **+5%** or **−40%** relative to the entry callout's market cap. These are market-cap snapshots, not the actual fill price or profit after fees. Callouts can be delayed or absent, so neither threshold guarantees an execution price or a sale. The bot also sells after `SELL_TIMEOUT_SECS` (default 90 seconds), measured from buy confirmation. The bot stops on stream disconnection or trading errors; an open or uncertain position must be managed manually. Check the reported transaction signature on chain before retrying an uncertain submission. Ctrl+C requests a sell of the tracked open position and waits for confirmation before exiting. If the sell fails or its outcome is uncertain, check the position and transaction manually.

The bot makes up to three transaction attempts when an on-chain confirmation explicitly reports failure, building a fresh transaction each time. Network errors, rejected submissions, and uncertain confirmations stop the bot without creating another transaction. Identical submission payloads may be retried separately for transient transport errors.

After a confirmed sell, P&L is shown in green or red on terminals as the USDC associated-token-account balance change from before the buy to after the sell. Other USDC transfers during that period affect the result; SOL fees and rent are not included.

Set `MAX_BUYS_PER_RUN` to a positive integer to limit confirmed buys during one run. After the final position is sold and confirmed, the bot exits. Leave it unset for unlimited buys; this is separate from the persistent per-mint limit.
