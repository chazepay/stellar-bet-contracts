# StellarBet Contracts — Build & Deploy Targets

STELLAR_CLI ?= stellar
NETWORK      ?= testnet
ADMIN_ID     ?= admin

.PHONY: all build test clean deploy-all fmt

all: build

# ── Build all contracts to WASM ──────────────────────────────────────────────
build:
	cargo build --target wasm32-unknown-unknown --release
	@echo "Built WASM artifacts in target/wasm32-unknown-unknown/release/"

# ── Run all tests ─────────────────────────────────────────────────────────────
test:
	cargo test -- --nocapture

# ── Format code ───────────────────────────────────────────────────────────────
fmt:
	cargo fmt --all

# ── Clean build artifacts ─────────────────────────────────────────────────────
clean:
	cargo clean

# ── Generate contract bindings ────────────────────────────────────────────────
bindings:
	$(STELLAR_CLI) contract bindings typescript \
		--wasm target/wasm32-unknown-unknown/release/betting_pool.wasm \
		--output-dir bindings/betting_pool --overwrite
	$(STELLAR_CLI) contract bindings typescript \
		--wasm target/wasm32-unknown-unknown/release/odds_oracle.wasm \
		--output-dir bindings/odds_oracle --overwrite
	$(STELLAR_CLI) contract bindings typescript \
		--wasm target/wasm32-unknown-unknown/release/house_escrow.wasm \
		--output-dir bindings/house_escrow --overwrite
	$(STELLAR_CLI) contract bindings typescript \
		--wasm target/wasm32-unknown-unknown/release/bet_token.wasm \
		--output-dir bindings/bet_token --overwrite

# ── Deploy to testnet ─────────────────────────────────────────────────────────
deploy-token:
	$(STELLAR_CLI) contract deploy \
		--wasm target/wasm32-unknown-unknown/release/bet_token.wasm \
		--source $(ADMIN_ID) --network $(NETWORK)

deploy-escrow:
	$(STELLAR_CLI) contract deploy \
		--wasm target/wasm32-unknown-unknown/release/house_escrow.wasm \
		--source $(ADMIN_ID) --network $(NETWORK)

deploy-oracle:
	$(STELLAR_CLI) contract deploy \
		--wasm target/wasm32-unknown-unknown/release/odds_oracle.wasm \
		--source $(ADMIN_ID) --network $(NETWORK)

deploy-pool:
	$(STELLAR_CLI) contract deploy \
		--wasm target/wasm32-unknown-unknown/release/betting_pool.wasm \
		--source $(ADMIN_ID) --network $(NETWORK)

deploy-all: deploy-token deploy-escrow deploy-oracle deploy-pool
	@echo "All contracts deployed to $(NETWORK)"

# ── Full automated deploy + init + env update ─────────────────────────────────
deploy:
	@chmod +x scripts/deploy.sh
	@./scripts/deploy.sh $(NETWORK)
