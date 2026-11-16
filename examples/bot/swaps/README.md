# Momentum trading example

Build a USDC trade, sign locally, submit through Rungst and wait for Solana confirmation. This example does not yet implement a momentum strategy.

Copy `.env.example` to `.env` in this directory and fill in `RUNGST_API_KEY` and `RUNGST_WALLET_FILE` (local Solana CLI JSON keypair). Confirmation uses `https://api.mainnet.solana.com` by default; optionally set `RUNGST_SOLANA_RPC_URL` to use another RPC. You can also set these as environment variables; existing environment variables take precedence. Trading requests use `https://api.rungst.com`.

```sh
cargo run -- buy TOKEN_MINT 10
cargo run -- sell TOKEN_MINT --submit
cargo run -- sell TOKEN_MINT 10 --submit
```

For example:

```sh
cargo run -- buy HkRQ6Qcadi5yuoNpZdA1Q7yVF3r7LdEmAw1aKkSxpump 0.005 --submit
cargo run -- sell HkRQ6Qcadi5yuoNpZdA1Q7yVF3r7LdEmAw1aKkSxpump --submit
```

Omit the sell amount to fetch and sell the full current balance from the wallet’s associated token account. Provide an amount to sell part of the balance. The balance may change before the transaction executes.

The first command builds only.
`--submit` signs and broadcasts a real trade.
 The wallet must have SOL for fees.
 A timeout is an unknown outcome.
 check the signature before creating another trade.
