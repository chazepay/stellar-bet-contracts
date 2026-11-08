#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# StellarBet — Full Contract Deploy Script
#
# Builds all 4 Soroban contracts, deploys them to the Stellar network,
# initializes them in the correct dependency order, and writes the resulting
# contract addresses into both .env files automatically.
#
# Usage:
#   ./scripts/deploy.sh [testnet|mainnet]
#
# Prerequisites:
#   - stellar CLI installed: https://developers.stellar.org/docs/tools/developer-tools
#   - Rust + wasm32 target: rustup target add wasm32-unknown-unknown
#   - A funded Stellar account set up as identity "admin" in stellar CLI:
#       stellar keys generate admin --network testnet
#       stellar keys fund admin --network testnet    (testnet only)
#   - ORACLE_SECRET_KEY exported or set in .env
#
# The script writes addresses to:
#   ../stellar-bet-backend/.env          (server-side)
#   ../stellar-bet-frontend/.env.local   (client-side NEXT_PUBLIC_ vars)
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

NETWORK="${1:-testnet}"
ADMIN_ID="admin"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTRACTS_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BACKEND_ENV="$CONTRACTS_ROOT/../../stellar-bet-backend-main/stellar-bet-backend-main/.env"
FRONTEND_ENV="$CONTRACTS_ROOT/../../stellar-bet-frontend-main/stellar-bet-frontend-main/.env.local"
WASM_DIR="$CONTRACTS_ROOT/target/wasm32-unknown-unknown/release"

log()  { echo -e "\033[32m[deploy]\033[0m $*"; }
warn() { echo -e "\033[33m[warn]\033[0m $*"; }
err()  { echo -e "\033[31m[error]\033[0m $*" >&2; exit 1; }

# ── Preflight checks ──────────────────────────────────────────────────────────

command -v stellar &>/dev/null || err "stellar CLI not found. Install from https://developers.stellar.org/docs/tools/developer-tools"
command -v cargo   &>/dev/null || err "cargo not found. Install Rust from https://rustup.rs"

if ! rustup target list --installed | grep -q "wasm32-unknown-unknown"; then
  warn "wasm32 target not installed — adding it now"
  rustup target add wasm32-unknown-unknown
fi

log "Deploying to network: $NETWORK"
log "Admin identity:       $ADMIN_ID"

# ── Step 1: Build all contracts ───────────────────────────────────────────────

log "Building all contracts…"
cd "$CONTRACTS_ROOT"
cargo build --target wasm32-unknown-unknown --release --quiet
log "Build complete."

# ── Step 2: Get admin address ─────────────────────────────────────────────────

ADMIN_ADDRESS=$(stellar keys address "$ADMIN_ID" --network "$NETWORK" 2>/dev/null) \
  || err "Identity '$ADMIN_ID' not found. Run: stellar keys generate $ADMIN_ID --network $NETWORK"

log "Admin address: $ADMIN_ADDRESS"

# Fund on testnet if balance is low
if [[ "$NETWORK" == "testnet" ]]; then
  log "Ensuring admin account is funded on testnet…"
  stellar keys fund "$ADMIN_ID" --network "$NETWORK" 2>/dev/null || warn "Fund request failed (may already be funded)"
fi

# ── Helper: deploy a contract and return its address ─────────────────────────

deploy_contract() {
  local name="$1"
  local wasm="$WASM_DIR/${name}.wasm"

  [[ -f "$wasm" ]] || err "WASM not found: $wasm"

  log "Deploying $name…"
  local address
  address=$(stellar contract deploy \
    --wasm "$wasm" \
    --source "$ADMIN_ID" \
    --network "$NETWORK" \
    2>&1 | tail -1)

  # Validate the returned address looks like a Stellar contract ID
  if [[ ! "$address" =~ ^C[A-Z0-9]{55}$ ]]; then
    err "Unexpected output deploying $name: $address"
  fi

  echo "$address"
}

# ── Step 3: Deploy all 4 contracts ───────────────────────────────────────────

BET_TOKEN_ID=$(deploy_contract "bet_token")
log "bet_token:    $BET_TOKEN_ID"

HOUSE_ESCROW_ID=$(deploy_contract "house_escrow")
log "house_escrow: $HOUSE_ESCROW_ID"

ODDS_ORACLE_ID=$(deploy_contract "odds_oracle")
log "odds_oracle:  $ODDS_ORACLE_ID"

BETTING_POOL_ID=$(deploy_contract "betting_pool")
log "betting_pool: $BETTING_POOL_ID"

# ── Step 4: Get the native XLM token address ──────────────────────────────────

log "Resolving native XLM token address…"
XLM_TOKEN_ID=$(stellar contract id asset \
  --asset native \
  --network "$NETWORK" \
  --source "$ADMIN_ID" \
  2>/dev/null) || warn "Could not resolve native XLM address — set XLM_TOKEN_ID manually"

log "XLM token:    ${XLM_TOKEN_ID:-<not resolved>}"

# ── Step 5: Initialize contracts in dependency order ─────────────────────────

log "Initializing bet_token…"
stellar contract invoke \
  --id "$BET_TOKEN_ID" \
  --source "$ADMIN_ID" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN_ADDRESS" \
  --decimal 7 \
  --name '"BetToken"' \
  --symbol '"BET"' \
  2>/dev/null || warn "bet_token may already be initialized"

log "Initializing house_escrow…"
stellar contract invoke \
  --id "$HOUSE_ESCROW_ID" \
  --source "$ADMIN_ID" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN_ADDRESS" \
  --xlm_token "${XLM_TOKEN_ID:-$BET_TOKEN_ID}" \
  --fee_bps 200 \
  2>/dev/null || warn "house_escrow may already be initialized"

log "Initializing odds_oracle…"
stellar contract invoke \
  --id "$ODDS_ORACLE_ID" \
  --source "$ADMIN_ID" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN_ADDRESS" \
  --pool "$BETTING_POOL_ID" \
  --quorum 1 \
  2>/dev/null || warn "odds_oracle may already be initialized"

# Add admin as the initial reporter
log "Adding admin as oracle reporter…"
stellar contract invoke \
  --id "$ODDS_ORACLE_ID" \
  --source "$ADMIN_ID" \
  --network "$NETWORK" \
  -- add_reporter \
  --reporter "$ADMIN_ADDRESS" \
  2>/dev/null || warn "Could not add reporter"

log "Initializing betting_pool…"
stellar contract invoke \
  --id "$BETTING_POOL_ID" \
  --source "$ADMIN_ID" \
  --network "$NETWORK" \
  -- initialize \
  --admin "$ADMIN_ADDRESS" \
  --oracle "$ODDS_ORACLE_ID" \
  --escrow "$HOUSE_ESCROW_ID" \
  --xlm_token "${XLM_TOKEN_ID:-$BET_TOKEN_ID}" \
  2>/dev/null || warn "betting_pool may already be initialized"

# ── Step 6: Write addresses to .env files ─────────────────────────────────────

write_env() {
  local file="$1"
  local key="$2"
  local value="$3"

  if [[ ! -f "$file" ]]; then
    warn "Env file not found: $file — skipping"
    return
  fi

  # Update existing key or append
  if grep -q "^${key}=" "$file" 2>/dev/null; then
    # macOS and Linux compatible sed
    sed -i.bak "s|^${key}=.*|${key}=${value}|" "$file" && rm -f "${file}.bak"
  else
    echo "${key}=${value}" >> "$file"
  fi
}

log "Writing contract addresses to backend .env…"
write_env "$BACKEND_ENV" "CONTRACT_BETTING_POOL" "$BETTING_POOL_ID"
write_env "$BACKEND_ENV" "CONTRACT_ODDS_ORACLE"  "$ODDS_ORACLE_ID"
write_env "$BACKEND_ENV" "CONTRACT_HOUSE_ESCROW" "$HOUSE_ESCROW_ID"
write_env "$BACKEND_ENV" "CONTRACT_BET_TOKEN"    "$BET_TOKEN_ID"

log "Writing contract addresses to frontend .env.local…"
write_env "$FRONTEND_ENV" "NEXT_PUBLIC_CONTRACT_BETTING_POOL" "$BETTING_POOL_ID"
write_env "$FRONTEND_ENV" "NEXT_PUBLIC_CONTRACT_HOUSE_ESCROW" "$HOUSE_ESCROW_ID"
write_env "$FRONTEND_ENV" "NEXT_PUBLIC_CONTRACT_BET_TOKEN"    "$BET_TOKEN_ID"

# ── Summary ───────────────────────────────────────────────────────────────────

echo ""
echo "════════════════════════════════════════════════════"
echo "  StellarBet Deploy Complete — $NETWORK"
echo "════════════════════════════════════════════════════"
echo "  bet_token:    $BET_TOKEN_ID"
echo "  house_escrow: $HOUSE_ESCROW_ID"
echo "  odds_oracle:  $ODDS_ORACLE_ID"
echo "  betting_pool: $BETTING_POOL_ID"
echo "────────────────────────────────────────────────────"
echo "  .env files updated:"
echo "    $BACKEND_ENV"
echo "    $FRONTEND_ENV"
echo "════════════════════════════════════════════════════"
echo ""
echo "  Next steps:"
echo "  1. Restart the backend:  npm run dev"
echo "  2. Restart the frontend: npm run dev"
echo "  3. Fund the house escrow pool via /liquidity"
echo ""
