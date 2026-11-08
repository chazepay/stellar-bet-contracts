# stellar-bet-contracts

Soroban smart contracts powering the StellarBet prediction market platform on Stellar.

[![Built on Soroban](https://img.shields.io/badge/Built%20on-Soroban-blueviolet)](https://stellar.org/soroban)
[![Rust](https://img.shields.io/badge/Rust-soroban--sdk%2021.0.0-orange?logo=rust)](https://docs.rs/soroban-sdk)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](./LICENSE)

---

## Table of Contents

- [Overview](#overview)
- [Architecture](#architecture)
- [Contracts](#contracts)
  - [BettingPool](#bettingpool)
  - [OddsOracle](#oddsoracle)
  - [HouseEscrow](#houseescrow)
  - [BetToken](#bettoken)
- [Prerequisites](#prerequisites)
- [Getting Started](#getting-started)
- [Deployment](#deployment)
- [Contract Addresses](#contract-addresses)
- [Data Types](#data-types)
- [Events](#events)
- [Project Structure](#project-structure)
- [Testing](#testing)
- [Security](#security)
- [License](#license)

---

## Overview

StellarBet runs entirely on Soroban. There are four contracts:

| Contract | Crate | Purpose |
|---|---|---|
| `BettingPool` | `contracts/betting_pool` | Bet lifecycle: place, settle, claim, cancel |
| `OddsOracle` | `contracts/odds_oracle` | Multi-reporter quorum oracle for settling markets |
| `HouseEscrow` | `contracts/house_escrow` | Liquidity pool: LPs deposit XLM, earn protocol fees |
| `BetToken` | `contracts/bet_token` | SEP-41 governance and rewards token |

They are deployed independently and wired together at initialization time via contract addresses.

---

## Architecture

```
User
 │
 ├── BettingPool.place_bet(bettor, market_id, outcome_index, stake_xlm, odds_bps)
 │        │
 │        └── stakes accumulate in market.total_pool
 │
Off-chain oracle backend
 │
 ├── OddsOracle.report_result(reporter, market_id, outcome, data_source, external_event_id)
 │        │  N reporters must agree (quorum threshold)
 │        └── emits QUORUM event → backend calls BettingPool.settle_market()
 │
 ├── BettingPool.settle_market(market_id, winning_outcome)   [oracle-only]
 │        └── marks market closed, sets winning_outcome
 │             ↑ called directly by OddsOracle via cross-contract invocation
 │
 └── BettingPool.claim_payout(bet_id)   [bettor]
          └── HouseEscrow.pay_winner(winner, gross_amount)
                   └── deducts fee_bps, transfers net XLM to winner
```

**Authorization model:**
- `BettingPool.create_market` — admin only
- `BettingPool.settle_market` — oracle contract address only
- `OddsOracle.report_result` — whitelisted reporter addresses only
- `HouseEscrow.pay_winner` — BettingPool contract address only
- All user-facing calls (`place_bet`, `claim_payout`, `cancel_bet`, `provide_liquidity`, `withdraw_liquidity`) require the caller's own auth

---

## Contracts

### BettingPool

Core bet lifecycle contract. Stores markets and bets in persistent storage.

**State-changing methods:**

| Method | Auth | Description |
|---|---|---|
| `initialize(admin, oracle, escrow)` | `admin` | One-time setup; sets admin, oracle, and escrow addresses |
| `create_market(description, sport, outcome_count, close_ledger)` | `admin` | Creates a new market; returns `market_id` (u64) |
| `place_bet(bettor, market_id, outcome_index, stake_xlm, odds_bps)` | `bettor` | Places a bet; returns `bet_id` (u64). Min stake 1 XLM (10_000_000 stroops). Odds 1.01x–50x (10100–500000 bps) |
| `settle_market(market_id, winning_outcome)` | `oracle` | Closes the market and records the winning outcome index |
| `claim_payout(bet_id)` | `bettor` | Claims winnings or marks bet as lost; returns payout in stroops |
| `cancel_bet(bet_id)` | `bettor` | Cancels an open bet before market closes; 1% cancellation fee; returns refund in stroops |

**Read-only methods:**

| Method | Returns | Description |
|---|---|---|
| `get_bet(bet_id)` | `Bet` | Bet by ID |
| `get_market(market_id)` | `Market` | Market by ID |
| `get_user_bets(user)` | `Vec<u64>` | All bet IDs for a Stellar address |
| `get_bet_count()` | `u64` | Total bets placed |
| `get_market_count()` | `u64` | Total markets created |

---

### OddsOracle

Multi-reporter quorum oracle. A market is settled only when `quorum` reporters submit the same winning outcome. Prevents any single off-chain source from unilaterally settling a market.

**State-changing methods:**

| Method | Auth | Description |
|---|---|---|
| `initialize(admin, pool, quorum)` | `admin` | One-time setup. `quorum` is the minimum reporter agreements required (≥ 1) |
| `add_reporter(reporter)` | `admin` | Whitelists a reporter address |
| `remove_reporter(reporter)` | `admin` | Removes a reporter from the whitelist |
| `report_result(reporter, market_id, outcome, data_source, external_event_id)` | `reporter` | Submits a result. Each reporter can only report once per market. Emits `QUORUM` when threshold is reached |

**Read-only methods:**

| Method | Returns | Description |
|---|---|---|
| `get_pending(market_id)` | `PendingSettlement` | Current vote tally for a market |
| `is_reporter(reporter)` | `bool` | Whether an address is a trusted reporter |
| `get_quorum()` | `u32` | Current quorum threshold |




### HouseEscrow

Liquidity pool contract. Liquidity providers deposit XLM and receive proportional shares. Shares appreciate as protocol fees accumulate in the pool.

**State-changing methods:**

| Method | Auth | Description |
|---|---|---|
| `initialize(admin, pool, fee_bps)` | `admin` | One-time setup. `fee_bps` is the protocol fee on payouts (max 1000 = 10%) |
| `provide_liquidity(provider, amount)` | `provider` | Deposits XLM (min 10 XLM / 100_000_000 stroops); returns shares issued |
| `withdraw_liquidity(provider, shares)` | `provider` | Burns shares and returns proportional XLM |
| `pay_winner(winner, gross_amount)` | `BettingPool` | Deducts fee, pays net amount to winner; fee stays in pool to boost LP value |

**Read-only methods:**

| Method | Returns | Description |
|---|---|---|
| `get_position(provider)` | `LiquidityPosition` | LP position for an address |
| `get_total_liquidity()` | `i128` | Total XLM in pool (stroops) |
| `get_total_fees()` | `i128` | Cumulative fees collected (stroops) |
| `get_fee_bps()` | `u32` | Current protocol fee in basis points |

**Share pricing:** First depositor's shares equal their deposit amount. Subsequent depositors receive `(amount × total_shares) / total_liquidity` shares. Because fees stay in the pool, each share's redemption value increases over time.

---

### BetToken

SEP-41 compatible governance and rewards token.

**Token details:**
- Name: `StellarBet Token`
- Symbol: `BET`
- Decimals: `7` (same as XLM stroops — 1 BET = 10_000_000 units)
- Supply: set at initialization, mutable by admin for reward distribution

**Methods:**

| Method | Auth | Description |
|---|---|---|
| `initialize(admin, initial_supply, name, symbol)` | `admin` | Mints full supply to admin |
| `transfer(from, to, amount)` | `from` | Transfer tokens |
| `approve(from, spender, amount, expiration_ledger)` | `from` | Set allowance |
| `transfer_from(spender, from, to, amount)` | `spender` | Transfer via allowance |
| `mint(to, amount)` | `admin` | Mint additional tokens for rewards |
| `burn(from, amount)` | `from` | Burn tokens from own balance |
| `name()` / `symbol()` / `decimals()` / `total_supply()` / `balance(id)` / `allowance(from, spender)` | — | SEP-41 read-only interface |

---

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) with the `wasm32-unknown-unknown` target
- [Stellar CLI](https://developers.stellar.org/docs/tools/stellar-cli)

```bash
rustup target add wasm32-unknown-unknown
cargo install stellar-cli --features opt
```

---

## Getting Started

```bash
# Clone
git clone https://github.com/YOUR_ORG/stellar-bet-contracts
cd stellar-bet-contracts

# Build all contracts to WASM
make build

# Run all tests
make test

# Format code
make fmt
```

---

## Deployment

### Automated deploy script (recommended)

The included `scripts/deploy.sh` does everything in one command — builds WASMs, deploys all 4 contracts, initializes them in dependency order, adds admin as oracle reporter, and writes all addresses into both `.env` files automatically.

```bash
# Set up admin identity (testnet)
stellar keys generate admin --network testnet
stellar keys fund admin --network testnet

# Run the full deploy
make deploy NETWORK=testnet ADMIN_ID=admin
```

The script writes addresses to:
- `../stellar-bet-backend/.env`
- `../stellar-bet-frontend/.env.local`

Then restart both servers and you're live.

### Manual deploy

```bash
# Fund an admin keypair on testnet
stellar keys generate admin --network testnet --fund

# Deploy (one at a time or all at once)
make deploy-token   ADMIN_ID=admin NETWORK=testnet
make deploy-escrow  ADMIN_ID=admin NETWORK=testnet
make deploy-oracle  ADMIN_ID=admin NETWORK=testnet
make deploy-pool    ADMIN_ID=admin NETWORK=testnet

# Or all at once
make deploy-all ADMIN_ID=admin NETWORK=testnet
```

After deployment, initialize each contract with the addresses of the others:

```bash
# 1. Initialize BetToken
stellar contract invoke --id <BET_TOKEN_ID> --source admin --network testnet \
  -- initialize --admin <ADMIN_ADDRESS> --initial_supply 1000000000000 \
  --name "StellarBet Token" --symbol "BET"

# 2. Initialize HouseEscrow (fee = 50 bps = 0.5%)
stellar contract invoke --id <HOUSE_ESCROW_ID> --source admin --network testnet \
  -- initialize --admin <ADMIN_ADDRESS> --pool <BETTING_POOL_ID> --fee_bps 50

# 3. Initialize OddsOracle (quorum = 2)
stellar contract invoke --id <ODDS_ORACLE_ID> --source admin --network testnet \
  -- initialize --admin <ADMIN_ADDRESS> --pool <BETTING_POOL_ID> --quorum 2

# 4. Initialize BettingPool
stellar contract invoke --id <BETTING_POOL_ID> --source admin --network testnet \
  -- initialize --admin <ADMIN_ADDRESS> --oracle <ODDS_ORACLE_ID> --escrow <HOUSE_ESCROW_ID>

# 5. Add oracle reporter(s)
stellar contract invoke --id <ODDS_ORACLE_ID> --source admin --network testnet \
  -- add_reporter --reporter <ORACLE_SERVICE_PUBLIC_KEY>
```

### Generate TypeScript Bindings

After deployment, generate typed bindings for the backend and frontend:

```bash
make bindings
# Outputs to: bindings/betting_pool/, bindings/odds_oracle/, bindings/house_escrow/, bindings/bet_token/
```

---

## Contract Addresses

> Fill in after deployment.

| Contract | Testnet Address |
|---|---|
| BettingPool | `CBLLZUILQKPSBVHGYMJN6YCVHTPAXITOLDCB3H3EK2JJQV4NLEHCYMMV` |
| OddsOracle  | `CC6JZ62ZEIJHTHO4ZBNWPLPM5METCMI5Q37XTSIYAFFCYASXEYAV4DW5` |
| HouseEscrow | `CC6ITRWWPWOHOLIISEJQW7KSSIRMPELONRAOT6XEQC677HQM4YZN3H7F` |
| BetToken    | `CDKSPSEGAJLQ2N4CWCRGMRBREPVNMW5AW5NNM52OCIB3RNZV56DIJOVZ` |

---

## Data Types

### `Bet`

| Field | Type | Description |
|---|---|---|
| `id` | `u64` | Unique bet ID |
| `bettor` | `Address` | Stellar address of the bettor |
| `market_id` | `u64` | ID of the market bet on |
| `outcome_index` | `u32` | 0 = home, 1 = away, 2 = draw |
| `stake_xlm` | `i128` | Stake in stroops (1 XLM = 10_000_000) |
| `odds_bps` | `u32` | Decimal odds × 10000 (e.g. 25000 = 2.5x) |
| `potential_payout` | `i128` | `stake_xlm × odds_bps / 10000` |
| `status` | `BetStatus` | `Open`, `Won`, `Lost`, `Cancelled`, `PendingSettlement` |
| `created_ledger` | `u32` | Ledger sequence at placement |
| `settled_ledger` | `u32` | Ledger sequence at settlement (0 if unsettled) |

### `Market`

| Field | Type | Description |
|---|---|---|
| `id` | `u64` | Unique market ID |
| `description` | `Bytes` | Human-readable description |
| `sport` | `Symbol` | Sport key (e.g. `SOCCER`) |
| `outcome_count` | `u32` | Number of possible outcomes (2–10) |
| `total_pool` | `i128` | Sum of all stakes in stroops |
| `winning_outcome` | `i32` | `-1` = unsettled, `≥0` = winning outcome index |
| `is_open` | `bool` | Whether betting is still accepted |
| `start_ledger` | `u32` | Ledger at creation |
| `close_ledger` | `u32` | Ledger after which no new bets are accepted |

### `LiquidityPosition`

| Field | Type | Description |
|---|---|---|
| `provider` | `Address` | LP's Stellar address |
| `deposited` | `i128` | Total XLM deposited in stroops |
| `shares` | `i128` | Current share balance |
| `joined_ledger` | `u32` | Ledger of first deposit |

---

## Events

| Contract | Topic | Payload | Trigger |
|---|---|---|---|
| BettingPool | `MKT_NEW` | `(market_id, sport)` | Market created |
| BettingPool | `BET_NEW` | `(bet_id, bettor, market_id, outcome_index, stake_xlm)` | Bet placed |
| BettingPool | `MKT_SETL` | `(market_id, winning_outcome)` | Market settled |
| BettingPool | `CLAIM` | `(bet_id, bettor, payout)` | Winning bet claimed |
| BettingPool | `LOST` | `(bet_id, bettor)` | Losing bet resolved |
| BettingPool | `CANCEL` | `(bet_id, refund)` | Bet cancelled |
| OddsOracle | `RPT_ADD` | `reporter` | Reporter whitelisted |
| OddsOracle | `RPT_RM` | `reporter` | Reporter removed |
| OddsOracle | `REPORTED` | `(market_id, outcome, reporter)` | Result submitted |
| OddsOracle | `QUORUM` | `(market_id, outcome, vote_count)` | Quorum reached |
| HouseEscrow | `LP_DEP` | `(provider, amount, shares)` | Liquidity deposited |
| HouseEscrow | `LP_WDR` | `(provider, amount, shares)` | Liquidity withdrawn |
| HouseEscrow | `PAYOUT` | `(winner, net_amount, fee)` | Winner paid out |
| BetToken | `MINT` | `(to, amount)` | Tokens minted |
| BetToken | `BURN` | `(from, amount)` | Tokens burned |
| BetToken | `XFER` | `(from, to, amount)` | Transfer |
| BetToken | `XFER_F` | `(spender, from, to, amount)` | Transfer via allowance |
| BetToken | `APPROVE` | `(from, spender, amount)` | Allowance set |

---

## Project Structure

```
contracts/
├── betting_pool/
│   ├── Cargo.toml
│   └── src/lib.rs      # BettingPool contract + unit tests
├── odds_oracle/
│   ├── Cargo.toml
│   └── src/lib.rs      # OddsOracle contract + unit tests
├── house_escrow/
│   ├── Cargo.toml
│   └── src/lib.rs      # HouseEscrow contract + unit tests
└── bet_token/
    ├── Cargo.toml
    └── src/lib.rs      # BetToken (SEP-41) contract + unit tests
Cargo.toml              # Workspace manifest, soroban-sdk = 21.0.0
Makefile                # build, test, fmt, clean, bindings, deploy-*
```

---

## Testing

Tests live alongside the contract source in each `lib.rs` under `#[cfg(test)]`. They use `soroban_sdk::testutils` with `env.mock_all_auths()`.

```bash
# Run all tests with output
make test

# Run a single contract's tests
cargo test -p betting_pool -- --nocapture
cargo test -p odds_oracle -- --nocapture
cargo test -p house_escrow -- --nocapture
cargo test -p bet_token -- --nocapture
```

**What's covered:**
- `BettingPool`: initialize, create market, place + settle + claim bet (full round trip)
- `OddsOracle`: add/remove reporters, single report (no quorum), two reports (quorum reached)
- `HouseEscrow`: liquidity deposit/withdrawal round trip, fee collection on payout
- `BetToken`: transfer, burn, total supply accounting

---

---

## Roadmap to Mainnet

StellarBet is currently deployed on Stellar Testnet. The path to mainnet has three gates:

**1. Security audit**
All four Soroban contracts (BettingPool, OddsOracle, HouseEscrow, BetToken) need a third-party audit before handling real funds. Key areas: re-entrancy in BettingPool, oracle quorum manipulation, LP share price manipulation.

**2. Oracle decentralisation**
The current quorum oracle uses a small set of trusted reporters. Mainnet requires a larger, diverse reporter set with economic incentives (stake-to-report) and on-chain dispute resolution.

**3. Regulatory review**
Sports betting is regulated in most jurisdictions. The platform needs legal review for each target market before enabling real-money play.

Testnet is fully functional and intended for community exploration, developer onboarding, and Stellar Wave program participation. Mainnet launch timeline will be published once audit and legal reviews are complete.

---

## Security

This is **testnet software**. Do not deploy to mainnet or use with real funds until a full security audit is complete. See [SECURITY.md](./SECURITY.md) for responsible disclosure guidelines.

Key areas to review before mainnet:
- The `eventToMarketMap` in the backend oracle scheduler is in-memory only and will not survive restarts

---

## License

MIT
