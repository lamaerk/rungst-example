# Swaps

Build a USDC trade, sign it locally, and submit it through Rungst.

## Setup

Install Rust, then copy `.env.example` to `.env` in this directory. Fill in your API key and the path to a local Solana CLI JSON keypair. The wallet needs SOL for fees. Existing environment variables take precedence over `.env`.

| Variable | Default | Purpose |
| --- | --- | --- |
| `RUNGST_API_KEY` | Required | Rungst API key |
| `RUNGST_WALLET_FILE` | Required | Path to the local keypair JSON file |
| `RUNGST_SOLANA_RPC_URL` | `https://api.mainnet.solana.com` | RPC used for balance and confirmation checks |

## Run

From this directory:

```sh
cargo run -- buy TOKEN_MINT 10
cargo run -- buy TOKEN_MINT 10 --submit
cargo run -- sell TOKEN_MINT --submit
cargo run -- sell TOKEN_MINT 10 --submit
```

The first command only builds a transaction. `--submit` signs and broadcasts a real trade. Omitting the sell amount uses the full associated-token-account balance, which may change before execution.

If submission or confirmation times out, check the transaction signature on chain before trying again.
