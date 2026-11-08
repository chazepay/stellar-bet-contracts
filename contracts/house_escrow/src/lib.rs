//! HouseEscrow Contract
//!
//! Manages the liquidity pool that backs all bet payouts.
//! Liquidity providers deposit XLM and earn a share of protocol fees.
//! The BettingPool contract is the only authorized caller for payouts.
//! XLM transfers are fully wired — provider → escrow on provide_liquidity,
//! escrow → provider on withdraw_liquidity, and escrow → winner on pay_winner.

#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    token::Client as TokenClient,
    Address, Env, Symbol,
};

// ─── Storage Keys ────────────────────────────────────────────────────────────

const ADMIN_KEY: Symbol = symbol_short!("ADMIN");
const POOL_KEY: Symbol = symbol_short!("POOL");     // BettingPool contract address
const TOKEN_KEY: Symbol = symbol_short!("TOKEN");   // Native XLM asset contract address
const TOTAL_LIQ: Symbol = symbol_short!("TOT_LIQ");
const TOTAL_FEES: Symbol = symbol_short!("TOT_FEES");
const FEE_BPS: Symbol = symbol_short!("FEE_BPS");  // Protocol fee in basis points

// ─── Data Types ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug)]
pub struct LiquidityPosition {
    pub provider: Address,
    pub deposited: i128,   // total XLM deposited (stroops)
    pub shares: i128,      // share of the liquidity pool
    pub joined_ledger: u32,
}

fn lp_key(addr: &Address) -> (Symbol, Address) {
    (symbol_short!("LP"), addr.clone())
}

fn total_shares_key() -> Symbol {
    symbol_short!("TOT_SHR")
}

// ─── Contract ────────────────────────────────────────────────────────────────

#[contract]
pub struct HouseEscrow;

#[contractimpl]
impl HouseEscrow {
    /// Initialize the escrow.
    /// fee_bps:   protocol fee charged on each winning payout (e.g., 50 = 0.5%)
    /// xlm_token: the native XLM Stellar asset contract address
    pub fn initialize(env: Env, admin: Address, pool: Address, fee_bps: u32, xlm_token: Address) {
        if env.storage().instance().has(&ADMIN_KEY) {
            panic!("already initialized");
        }
        admin.require_auth();
        assert!(fee_bps <= 1000, "fee cannot exceed 10%");

        env.storage().instance().set(&ADMIN_KEY, &admin);
        env.storage().instance().set(&POOL_KEY, &pool);
        env.storage().instance().set(&TOKEN_KEY, &xlm_token);
        env.storage().instance().set(&FEE_BPS, &fee_bps);
        env.storage().instance().set(&TOTAL_LIQ, &0i128);
        env.storage().instance().set(&TOTAL_FEES, &0i128);
        env.storage().instance().set(&total_shares_key(), &0i128);
    }

    /// Provide XLM liquidity to back the house.
    /// Transfers `amount` stroops from `provider` to this contract.
    /// Returns the number of shares issued to the LP.
    pub fn provide_liquidity(env: Env, provider: Address, amount: i128) -> i128 {
        provider.require_auth();
        assert!(amount >= 100_000_000, "minimum liquidity is 10 XLM");

        // Transfer XLM from provider → this contract
        let xlm_token: Address = env.storage().instance().get(&TOKEN_KEY).unwrap();
        let token = TokenClient::new(&env, &xlm_token);
        token.transfer(&provider, &env.current_contract_address(), &amount);

        let total_liq: i128 = env.storage().instance().get(&TOTAL_LIQ).unwrap_or(0);
        let total_shares: i128 = env
            .storage()
            .instance()
            .get(&total_shares_key())
            .unwrap_or(0);

        // Share calculation: if first depositor, shares = amount; else proportional
        let new_shares = if total_shares == 0 || total_liq == 0 {
            amount
        } else {
            amount
                .checked_mul(total_shares)
                .expect("overflow")
                / total_liq
        };

        let mut position: LiquidityPosition = env
            .storage()
            .persistent()
            .get(&lp_key(&provider))
            .unwrap_or(LiquidityPosition {
                provider: provider.clone(),
                deposited: 0,
                shares: 0,
                joined_ledger: env.ledger().sequence(),
            });

        position.deposited += amount;
        position.shares += new_shares;
        env.storage().persistent().set(&lp_key(&provider), &position);

        env.storage()
            .instance()
            .set(&TOTAL_LIQ, &(total_liq + amount));
        env.storage()
            .instance()
            .set(&total_shares_key(), &(total_shares + new_shares));

        env.events()
            .publish((symbol_short!("LP_DEP"),), (provider, amount, new_shares));

        new_shares
    }

    /// Withdraw liquidity proportional to share count.
    /// Transfers the proportional XLM amount from this contract back to `provider`.
    /// Returns the XLM amount withdrawn in stroops.
    pub fn withdraw_liquidity(env: Env, provider: Address, shares: i128) -> i128 {
        provider.require_auth();

        let mut position: LiquidityPosition = env
            .storage()
            .persistent()
            .get(&lp_key(&provider))
            .expect("no liquidity position found");

        assert!(position.shares >= shares, "insufficient shares");

        let total_liq: i128 = env.storage().instance().get(&TOTAL_LIQ).unwrap_or(0);
        let total_shares: i128 = env
            .storage()
            .instance()
            .get(&total_shares_key())
            .unwrap_or(0);

        // XLM to return = (shares / total_shares) * total_liq
        let withdraw_amount = shares
            .checked_mul(total_liq)
            .expect("overflow")
            / total_shares;

        position.shares -= shares;
        position.deposited = position
            .deposited
            .saturating_sub(withdraw_amount);
        env.storage().persistent().set(&lp_key(&provider), &position);

        env.storage()
            .instance()
            .set(&TOTAL_LIQ, &(total_liq - withdraw_amount));
        env.storage()
            .instance()
            .set(&total_shares_key(), &(total_shares - shares));

        // Transfer XLM from this contract → provider
        let xlm_token: Address = env.storage().instance().get(&TOKEN_KEY).unwrap();
        let token = TokenClient::new(&env, &xlm_token);
        token.transfer(&env.current_contract_address(), &provider, &withdraw_amount);

        env.events()
            .publish((symbol_short!("LP_WDR"),), (provider, withdraw_amount, shares));

        withdraw_amount
    }

    /// Pay out a winning bet. Only callable by the BettingPool contract.
    /// Deducts protocol fee before transfer; fee stays in pool to boost LP value.
    /// Returns the net payout amount in stroops.
    pub fn pay_winner(env: Env, winner: Address, gross_amount: i128) -> i128 {
        let pool: Address = env.storage().instance().get(&POOL_KEY).unwrap();
        pool.require_auth();

        let fee_bps: u32 = env.storage().instance().get(&FEE_BPS).unwrap_or(50);
        let fee = gross_amount * fee_bps as i128 / 10_000;
        let net_amount = gross_amount - fee;

        let total_liq: i128 = env.storage().instance().get(&TOTAL_LIQ).unwrap_or(0);
        assert!(total_liq >= net_amount, "insufficient liquidity for payout");

        // Update liquidity — fee stays in pool, boosting LP share value
        env.storage()
            .instance()
            .set(&TOTAL_LIQ, &(total_liq - net_amount + fee));

        let total_fees: i128 = env.storage().instance().get(&TOTAL_FEES).unwrap_or(0);
        env.storage()
            .instance()
            .set(&TOTAL_FEES, &(total_fees + fee));

        // Transfer net XLM from this contract → winner
        let xlm_token: Address = env.storage().instance().get(&TOKEN_KEY).unwrap();
        let token = TokenClient::new(&env, &xlm_token);
        token.transfer(&env.current_contract_address(), &winner, &net_amount);

        env.events()
            .publish((symbol_short!("PAYOUT"),), (winner, net_amount, fee));

        net_amount
    }

    // ─── View Functions ──────────────────────────────────────────────────

    pub fn get_position(env: Env, provider: Address) -> LiquidityPosition {
        env.storage()
            .persistent()
            .get(&lp_key(&provider))
            .expect("no position")
    }

    pub fn get_total_liquidity(env: Env) -> i128 {
        env.storage().instance().get(&TOTAL_LIQ).unwrap_or(0)
    }

    pub fn get_total_fees(env: Env) -> i128 {
        env.storage().instance().get(&TOTAL_FEES).unwrap_or(0)
    }

    pub fn get_fee_bps(env: Env) -> u32 {
        env.storage().instance().get(&FEE_BPS).unwrap_or(50)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{
        testutils::Address as _,
        token::{Client as TokenClient, StellarAssetClient as TokenAdmin},
        Env,
    };

    fn setup_token(env: &Env, admin: &Address) -> Address {
        let token_id = env.register_stellar_asset_contract_v2(admin.clone());
        let token_admin = TokenAdmin::new(env, &token_id.address());
        // Mint a large supply to admin for distribution in tests
        token_admin.mint(admin, &1_000_000_000_000i128);
        token_id.address()
    }

    #[test]
    fn test_liquidity_round_trip() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, HouseEscrow);
        let client = HouseEscrowClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let pool = Address::generate(&env);
        let lp = Address::generate(&env);

        let xlm_token = setup_token(&env, &admin);
        let token = TokenClient::new(&env, &xlm_token);

        // Fund LP with 100 XLM
        let token_admin = TokenAdmin::new(&env, &xlm_token);
        token_admin.mint(&lp, &1_000_000_000i128); // 100 XLM

        client.initialize(&admin, &pool, &50u32, &xlm_token); // 0.5% fee

        // Deposit 100 XLM
        let deposit = 1_000_000_000i128; // 100 XLM
        let shares = client.provide_liquidity(&lp, &deposit);
        assert_eq!(shares, deposit);
        assert_eq!(client.get_total_liquidity(), deposit);
        assert_eq!(token.balance(&lp), 0);
        assert_eq!(token.balance(&contract_id), deposit);

        // Withdraw all shares
        let withdrawn = client.withdraw_liquidity(&lp, &shares);
        assert_eq!(withdrawn, deposit);
        assert_eq!(client.get_total_liquidity(), 0);
        assert_eq!(token.balance(&lp), deposit);
        assert_eq!(token.balance(&contract_id), 0);
    }

    #[test]
    fn test_fee_collection() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, HouseEscrow);
        let client = HouseEscrowClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let pool = Address::generate(&env);
        let lp = Address::generate(&env);
        let winner = Address::generate(&env);

        let xlm_token = setup_token(&env, &admin);
        let token = TokenClient::new(&env, &xlm_token);

        // Fund LP and seed the escrow contract itself (simulates stake flow from BettingPool)
        let token_admin = TokenAdmin::new(&env, &xlm_token);
        token_admin.mint(&lp, &10_000_000_000i128); // 1000 XLM

        client.initialize(&admin, &pool, &100u32, &xlm_token); // 1% fee

        // LP deposits 1000 XLM
        client.provide_liquidity(&lp, &10_000_000_000i128);

        // Pay out 100 XLM gross → winner gets 99 XLM, 1 XLM stays in pool as fee
        let gross = 1_000_000_000i128; // 100 XLM
        let net = client.pay_winner(&winner, &gross);
        assert_eq!(net, 990_000_000i128);                   // 99 XLM to winner
        assert_eq!(client.get_total_fees(), 10_000_000i128); // 1 XLM in fees
        assert_eq!(token.balance(&winner), net);
        // Pool should be: 1000 XLM deposited - 99 XLM paid out + 1 XLM fee = 902 XLM
        assert_eq!(client.get_total_liquidity(), 9_010_000_000i128);
    }
}
