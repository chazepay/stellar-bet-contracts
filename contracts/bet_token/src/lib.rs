//! BET Token Contract
//!
//! SEP-41 compatible governance and rewards token for the StellarBet platform.
//! Used for: protocol governance voting, fee discounts, LP reward boosts.
//! Fixed supply. Distributed to active bettors and liquidity providers.

#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    Address, Env, Symbol, String,
};

// ─── Storage Keys ────────────────────────────────────────────────────────────

const ADMIN_KEY: Symbol = symbol_short!("ADMIN");
const TOTAL_SUPPLY: Symbol = symbol_short!("SUPPLY");
const NAME_KEY: Symbol = symbol_short!("NAME");
const SYMBOL_KEY: Symbol = symbol_short!("SYM");
const DECIMALS_KEY: Symbol = symbol_short!("DEC");

fn balance_key(addr: &Address) -> (Symbol, Address) {
    (symbol_short!("BAL"), addr.clone())
}

fn allowance_key(owner: &Address, spender: &Address) -> (Symbol, Address, Address) {
    (symbol_short!("ALLOW"), owner.clone(), spender.clone())
}

// ─── Contract ────────────────────────────────────────────────────────────────

#[contract]
pub struct BetToken;

#[contractimpl]
impl BetToken {
    /// Initialize the BET token.
    /// initial_supply: in base units (1 BET = 10^7 units, same as XLM stroops)
    pub fn initialize(
        env: Env,
        admin: Address,
        initial_supply: i128,
        name: String,
        symbol: String,
    ) {
        if env.storage().instance().has(&ADMIN_KEY) {
            panic!("already initialized");
        }
        admin.require_auth();
        assert!(initial_supply > 0, "supply must be positive");

        env.storage().instance().set(&ADMIN_KEY, &admin);
        env.storage().instance().set(&TOTAL_SUPPLY, &initial_supply);
        env.storage().instance().set(&NAME_KEY, &name);
        env.storage().instance().set(&SYMBOL_KEY, &symbol);
        env.storage().instance().set(&DECIMALS_KEY, &7u32);

        // Mint all to admin initially — admin distributes to reward pools
        env.storage()
            .persistent()
            .set(&balance_key(&admin), &initial_supply);

        env.events()
            .publish((symbol_short!("MINT"),), (admin, initial_supply));
    }

    // ─── SEP-41 Interface ────────────────────────────────────────────────

    pub fn name(env: Env) -> String {
        env.storage().instance().get(&NAME_KEY).unwrap()
    }

    pub fn symbol(env: Env) -> String {
        env.storage().instance().get(&SYMBOL_KEY).unwrap()
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage().instance().get(&DECIMALS_KEY).unwrap_or(7)
    }

    pub fn total_supply(env: Env) -> i128 {
        env.storage().instance().get(&TOTAL_SUPPLY).unwrap_or(0)
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&balance_key(&id))
            .unwrap_or(0)
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        assert!(amount > 0, "amount must be positive");

        let from_balance: i128 = env
            .storage()
            .persistent()
            .get(&balance_key(&from))
            .unwrap_or(0);

        assert!(from_balance >= amount, "insufficient balance");

        let to_balance: i128 = env
            .storage()
            .persistent()
            .get(&balance_key(&to))
            .unwrap_or(0);

        env.storage()
            .persistent()
            .set(&balance_key(&from), &(from_balance - amount));
        env.storage()
            .persistent()
            .set(&balance_key(&to), &(to_balance + amount));

        env.events()
            .publish((symbol_short!("XFER"),), (from, to, amount));
    }

    pub fn approve(env: Env, from: Address, spender: Address, amount: i128, expiration_ledger: u32) {
        from.require_auth();
        env.storage()
            .persistent()
            .set(&allowance_key(&from, &spender), &amount);
        env.events()
            .publish((symbol_short!("APPROVE"),), (from, spender, amount));
    }

    pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&allowance_key(&from, &spender))
            .unwrap_or(0)
    }

    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();

        let current_allowance: i128 = env
            .storage()
            .persistent()
            .get(&allowance_key(&from, &spender))
            .unwrap_or(0);

        assert!(current_allowance >= amount, "insufficient allowance");

        let from_balance: i128 = env
            .storage()
            .persistent()
            .get(&balance_key(&from))
            .unwrap_or(0);
        assert!(from_balance >= amount, "insufficient balance");

        let to_balance: i128 = env
            .storage()
            .persistent()
            .get(&balance_key(&to))
            .unwrap_or(0);

        env.storage()
            .persistent()
            .set(&allowance_key(&from, &spender), &(current_allowance - amount));
        env.storage()
            .persistent()
            .set(&balance_key(&from), &(from_balance - amount));
        env.storage()
            .persistent()
            .set(&balance_key(&to), &(to_balance + amount));

        env.events()
            .publish((symbol_short!("XFER_F"),), (spender, from, to, amount));
    }

    /// Mint new tokens. Only admin (used for reward distribution).
    pub fn mint(env: Env, to: Address, amount: i128) {
        let admin: Address = env.storage().instance().get(&ADMIN_KEY).unwrap();
        admin.require_auth();
        assert!(amount > 0, "amount must be positive");

        let to_balance: i128 = env
            .storage()
            .persistent()
            .get(&balance_key(&to))
            .unwrap_or(0);

        env.storage()
            .persistent()
            .set(&balance_key(&to), &(to_balance + amount));

        let total: i128 = env.storage().instance().get(&TOTAL_SUPPLY).unwrap_or(0);
        env.storage()
            .instance()
            .set(&TOTAL_SUPPLY, &(total + amount));

        env.events()
            .publish((symbol_short!("MINT"),), (to, amount));
    }

    /// Burn tokens. Any holder can burn their own tokens.
    pub fn burn(env: Env, from: Address, amount: i128) {
        from.require_auth();

        let from_balance: i128 = env
            .storage()
            .persistent()
            .get(&balance_key(&from))
            .unwrap_or(0);
        assert!(from_balance >= amount, "insufficient balance");

        env.storage()
            .persistent()
            .set(&balance_key(&from), &(from_balance - amount));

        let total: i128 = env.storage().instance().get(&TOTAL_SUPPLY).unwrap_or(0);
        env.storage()
            .instance()
            .set(&TOTAL_SUPPLY, &(total - amount));

        env.events()
            .publish((symbol_short!("BURN"),), (from, amount));
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env, String};

    #[test]
    fn test_token_basics() {
        let env = Env::default();
        let contract_id = env.register_contract(None, BetToken);
        let client = BetTokenClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        let supply = 1_000_000_000_000i128; // 100,000 BET

        env.mock_all_auths();
        client.initialize(
            &admin,
            &supply,
            &String::from_str(&env, "StellarBet Token"),
            &String::from_str(&env, "BET"),
        );

        assert_eq!(client.total_supply(), supply);
        assert_eq!(client.balance(&admin), supply);
        assert_eq!(client.decimals(), 7);

        // Transfer 1000 BET to user
        client.transfer(&admin, &user, &10_000_000_000i128);
        assert_eq!(client.balance(&user), 10_000_000_000i128);
        assert_eq!(client.balance(&admin), supply - 10_000_000_000i128);
    }

    #[test]
    fn test_burn() {
        let env = Env::default();
        let contract_id = env.register_contract(None, BetToken);
        let client = BetTokenClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        env.mock_all_auths();
        client.initialize(
            &admin,
            &1_000_000_000_000i128,
            &String::from_str(&env, "StellarBet Token"),
            &String::from_str(&env, "BET"),
        );

        client.burn(&admin, &1_000_000_000i128);
        assert_eq!(
            client.total_supply(),
            1_000_000_000_000i128 - 1_000_000_000i128
        );
    }
}
