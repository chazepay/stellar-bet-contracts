# Stellar Wave — Pre-drafted Issues

Copy each section below to create issues in the GitHub UI.
Tag each with `stellar-wave` and set points in the Drips Wave dashboard.

---

## Issue 1 — Wire native XLM transfers in BettingPool (Medium, 300pts)

**Title:** `Wire actual XLM transfer calls in BettingPool.place_bet and claim_payout`

The `place_bet` and `claim_payout` functions currently contain stubbed token
transfer comments. Wire them to the Stellar native asset client so XLM moves
on-chain when bets are placed and payouts are claimed.

**Requirements:**
- Use `stellar_asset_client` (native XLM) for transfers in `place_bet`
- Use `HouseEscrow.pay_winner` cross-contract call from `claim_payout`
- Add integration tests that verify balances change correctly
- All existing unit tests must still pass

**Acceptance criteria:** A testnet deployment where placing a bet visibly
moves XLM from the bettor's account to the escrow contract.

---

## Issue 2 — Add cross-contract call from OddsOracle to BettingPool (Medium, 250pts)

**Title:** `Implement cross-contract settlement call in OddsOracle`

When oracle quorum is reached in `OddsOracle.report_result`, the contract
should directly call `BettingPool.settle_market` via a cross-contract
invocation rather than relying on the backend to poll.

**Requirements:**
- Add `betting_pool` as a workspace dependency in `odds_oracle/Cargo.toml`
- Uncomment and implement the `pool_client.settle_market()` call
- Handle the case where the cross-contract call fails (emit error event, don't panic)
- Write a test that verifies settlement fires from oracle quorum

---

## Issue 3 — Implement BET token reward distribution (Medium, 350pts)

**Title:** `Implement BET token rewards for bettors and liquidity providers`

The `BetToken` contract exists but rewards are not distributed. Add a
`RewardDistributor` module (or extend `HouseEscrow`) that:

- Mints BET tokens to bettors proportional to their stake volume (weekly epoch)
- Mints BET tokens to LPs proportional to their pool share
- Admin can set reward rates per epoch
- Includes an `epoch_end` function callable by admin to snapshot and distribute

---

## Issue 4 — Add bet cancellation window enforcement (Low, 150pts)

**Title:** `Enforce minimum stake time before cancellation is allowed`

Currently `cancel_bet` allows cancellation at any time before market close.
Add a minimum hold time (e.g. 100 ledgers ≈ 8 minutes) before a bet can
be cancelled. This prevents MEV-style cancellation abuse.

**Requirements:**
- Add `MIN_HOLD_LEDGERS` constant (configurable by admin)
- Enforce in `cancel_bet`: `current_ledger - created_ledger >= MIN_HOLD_LEDGERS`
- Update existing cancel test to account for the hold period
- Emit a clear error message if the hold period hasn't elapsed

---

## Issue 5 — Gas optimization: batch claim_payout for multiple bets (High, 500pts)

**Title:** `Add batch_claim_payout to settle multiple winning bets in one transaction`

Add a `batch_claim_payout(bet_ids: Vec<u64>) -> Vec<i128>` function that
processes up to 10 bets in a single transaction, reducing Soroban fee overhead
for active users with multiple open bets on the same settled market.

**Requirements:**
- Max 10 bets per batch (enforce with assertion)
- All bets in a batch must belong to the caller (auth check)
- Partial success: process valid bets, skip invalid ones, return array of payouts
- Benchmark: measure fee vs N individual calls

---

## Issue 6 — Security: add re-entrancy guard pattern (High, 400pts)

**Title:** `Implement re-entrancy guard for BettingPool state-mutating functions`

Soroban contracts can be called recursively via cross-contract calls. Add a
re-entrancy guard (lock flag in instance storage) to `place_bet`, `claim_payout`,
and `cancel_bet` to prevent any exploit path that re-enters during a payout.

**Requirements:**
- Use a boolean `LOCKED` key in instance storage
- Set to `true` at the start of each guarded function, `false` at the end
- If `LOCKED` is already `true` on entry, panic with `"re-entrant call"`
- Write a test that verifies the guard works (mock a re-entrant call attempt)
