# Momentum bot

Observe Solana momentum or trade through Rungst. The bot uses two IP-authorized streams and signs trades locally. Register the machine's public IP first. Unfiltered streams require a paid plan and consume credits while connected.

## Run

From the repository root:

```sh
cargo run -p momentum          # observe candidates
cargo run -p momentum -- --buy # trade with real funds
```

For live trading, copy `examples/bot/momentum/.env.example` to `.env` in the working directory, or set environment variables. Existing environment variables take precedence. The wallet needs USDC and SOL for fees and rent.

## Environment variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `RUNGST_API_KEY` | Required for live mode | Rungst API key |
| `RUNGST_WALLET_FILE` | Required for live mode | Path to a local Solana JSON keypair |
| `BUY_AMOUNT_USDC` | Required for live mode | USDC amount per buy |
| `RUNGST_SOLANA_RPC_URL` | `https://api.mainnet.solana.com` | RPC for balance and confirmation checks |
| `SLIPPAGE_BPS` | `100` | Trade slippage in basis points |
| `WINDOW_SECS` | `30` | Rolling event window in seconds |
| `MIN_BUYERS` | `5` | Minimum distinct buyer wallets |
| `MIN_BUY_RATIO` | `0.6` | Minimum buy share of priced trades, from 0 to 1 |
| `MIN_NETFLOW_USD` | `100` | Minimum buy volume minus sell volume in USD |
| `MIN_CALLOUTS` | `1` | Minimum callouts in the window |
| `SELL_TIMEOUT_SECS` | `90` | Seconds after buy confirmation before selling |
| `MAX_BUYS_PER_MINT` | `1` | Maximum lifetime buy attempts per mint |
| `BUY_HISTORY_FILE` | `momentum-buy-history.json` | Path to persistent buy counts |
| `MAX_BUYS_PER_RUN` | Unset | Optional limit on confirmed buys per run |

## Strategy and operation

A mint must meet all five window thresholds and have a callout with a positive market cap. Only trades with a trader and positive finite USD amount count; events are not deduplicated. The latest qualifying callout sets the entry reference.

After a confirmed buy, the bot sells when a callout shows a market-cap change of +5% or −40%, or when the timeout expires. These snapshots do not guarantee an execution price or a sale. Only one position is open at a time; the bot refuses to buy a token already held in the associated account.

Buy attempts are counted before submission, including failed or uncertain attempts. Keep the same history file across restarts. After a confirmed sale the bot resumes, unless `MAX_BUYS_PER_RUN` has been reached.

The position is held in memory only. Use one instance and manage open positions before restarting. Ctrl+C requests a sale of the tracked position. Stream or trading errors stop the bot; check any open or uncertain position and transaction signature before retrying.

The bot can rebuild after a confirmed on-chain failure, up to three attempts. An uncertain submission is not rebuilt. Reported P&L is the USDC account balance change across the trade; other transfers affect it, and SOL costs are excluded.
