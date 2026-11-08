//! OddsOracle Contract
//!
//! Authorized oracle that receives market results from trusted off-chain
//! data providers and forwards settlement instructions to the BettingPool.
//! Uses a multi-sig quorum model: N-of-M reporter threshold before settlement.
//! When quorum is reached, BettingPool.settle_market is called via a direct
//! cross-contract invocation rather than relying on the backend to poll.

#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    Address, Env, Map, Symbol, Vec,
};

// ─── Storage Keys ────────────────────────────────────────────────────────────

const ADMIN_KEY: Symbol = symbol_short!("ADMIN");
const POOL_KEY: Symbol = symbol_short!("POOL");
const QUORUM_KEY: Symbol = symbol_short!("QUORUM");

// ─── Data Types ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug)]
pub struct OracleReport {
    pub market_id: u64,
    pub reported_outcome: u32,
    pub reporter: Address,
    pub ledger: u32,
    pub data_source: Symbol, // e.g., "SPORTRADAR", "THEODDS"
    pub external_event_id: soroban_sdk::Bytes,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PendingSettlement {
    pub market_id: u64,
    pub outcome_votes: Map<u32, u32>, // outcome_index -> vote count
    pub reporters: Vec<Address>,
    pub is_settled: bool,
}

fn reporter_key(addr: &Address) -> (Symbol, Address) {
    (symbol_short!("RPTR"), addr.clone())
}

fn pending_key(market_id: u64) -> (Symbol, u64) {
    (symbol_short!("PEND"), market_id)
}

fn report_key(market_id: u64, reporter: &Address) -> (Symbol, u64, Address) {
    (symbol_short!("RPT"), market_id, reporter.clone())
}

// ─── BettingPool client (cross-contract call for settle_market) ──────────────

mod betting_pool {
    use soroban_sdk::{contractclient, Env};

    #[allow(dead_code)]
    #[contractclient(name = "BettingPoolClient")]
    pub trait BettingPool {
        fn settle_market(env: Env, market_id: u64, winning_outcome: u32);
    }
}

// ─── Contract ────────────────────────────────────────────────────────────────

#[contract]
pub struct OddsOracle;

#[contractimpl]
impl OddsOracle {
    /// Initialize the oracle.
    /// quorum: minimum number of reporter agreements required to settle (e.g., 2)
    pub fn initialize(env: Env, admin: Address, pool: Address, quorum: u32) {
        if env.storage().instance().has(&ADMIN_KEY) {
            panic!("already initialized");
        }
        admin.require_auth();
        assert!(quorum >= 1, "quorum must be >= 1");

        env.storage().instance().set(&ADMIN_KEY, &admin);
        env.storage().instance().set(&POOL_KEY, &pool);
        env.storage().instance().set(&QUORUM_KEY, &quorum);
    }

    /// Admin adds a trusted reporter (off-chain oracle service).
    pub fn add_reporter(env: Env, reporter: Address) {
        let admin: Address = env.storage().instance().get(&ADMIN_KEY).unwrap();
        admin.require_auth();
        env.storage()
            .persistent()
            .set(&reporter_key(&reporter), &true);
        env.events()
            .publish((symbol_short!("RPT_ADD"),), reporter);
    }

    /// Admin removes a reporter.
    pub fn remove_reporter(env: Env, reporter: Address) {
        let admin: Address = env.storage().instance().get(&ADMIN_KEY).unwrap();
        admin.require_auth();
        env.storage()
            .persistent()
            .remove(&reporter_key(&reporter));
        env.events()
            .publish((symbol_short!("RPT_RM"),), reporter);
    }

    /// A trusted reporter submits a market result.
    /// Once quorum is reached, BettingPool.settle_market is called directly
    /// via cross-contract invocation and the QUORUM event is emitted.
    pub fn report_result(
        env: Env,
        reporter: Address,
        market_id: u64,
        outcome: u32,
        data_source: Symbol,
        external_event_id: soroban_sdk::Bytes,
    ) {
        reporter.require_auth();

        let is_trusted: bool = env
            .storage()
            .persistent()
            .get(&reporter_key(&reporter))
            .unwrap_or(false);
        assert!(is_trusted, "reporter not authorized");

        // Prevent double-reporting from same reporter for same market
        assert!(
            !env.storage()
                .persistent()
                .has(&report_key(market_id, &reporter)),
            "reporter already submitted for this market"
        );

        env.storage()
            .persistent()
            .set(&report_key(market_id, &reporter), &outcome);

        let mut pending: PendingSettlement = env
            .storage()
            .persistent()
            .get(&pending_key(market_id))
            .unwrap_or(PendingSettlement {
                market_id,
                outcome_votes: Map::new(&env),
                reporters: Vec::new(&env),
                is_settled: false,
            });

        assert!(!pending.is_settled, "market already settled");

        // Tally vote
        let current = pending.outcome_votes.get(outcome).unwrap_or(0);
        pending.outcome_votes.set(outcome, current + 1);
        pending.reporters.push_back(reporter.clone());

        env.events()
            .publish((symbol_short!("REPORTED"),), (market_id, outcome, reporter.clone()));

        let quorum: u32 = env.storage().instance().get(&QUORUM_KEY).unwrap();
        let vote_count = pending.outcome_votes.get(outcome).unwrap_or(0);

        if vote_count >= quorum {
            // Quorum reached — mark settled and persist before the cross-contract call
            // so any re-entrant report_result will hit the `is_settled` guard.
            pending.is_settled = true;
            env.storage()
                .persistent()
                .set(&pending_key(market_id), &pending);

            // Call BettingPool.settle_market on-chain
            let pool: Address = env.storage().instance().get(&POOL_KEY).unwrap();
            let pool_client = betting_pool::BettingPoolClient::new(&env, &pool);
            pool_client.settle_market(&market_id, &outcome);

            env.events()
                .publish((symbol_short!("QUORUM"),), (market_id, outcome, vote_count));
        } else {
            env.storage()
                .persistent()
                .set(&pending_key(market_id), &pending);
        }
    }

    // ─── View Functions ──────────────────────────────────────────────────

    pub fn get_pending(env: Env, market_id: u64) -> PendingSettlement {
        env.storage()
            .persistent()
            .get(&pending_key(market_id))
            .expect("no reports for this market")
    }

    pub fn is_reporter(env: Env, reporter: Address) -> bool {
        env.storage()
            .persistent()
            .get(&reporter_key(&reporter))
            .unwrap_or(false)
    }

    pub fn get_quorum(env: Env) -> u32 {
        env.storage().instance().get(&QUORUM_KEY).unwrap()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{
        testutils::Address as _,
        token::StellarAssetClient as TokenAdmin,
        Env,
    };

    // ─── Minimal BettingPool registration for cross-contract test ────────

    mod mock_pool {
        use soroban_sdk::{contract, contractimpl, symbol_short, Env};

        /// Minimal BettingPool stub — records which markets have been settled
        /// so the quorum test can verify the cross-contract call was made.
        #[contract]
        pub struct MockPool;

        const SETTLED_KEY: soroban_sdk::Symbol = symbol_short!("SETTLED");

        #[contractimpl]
        impl MockPool {
            pub fn settle_market(env: Env, market_id: u64, winning_outcome: u32) {
                // Store the settled outcome so tests can assert it
                env.storage()
                    .persistent()
                    .set(&(symbol_short!("MKT"), market_id), &winning_outcome);
                env.events()
                    .publish((SETTLED_KEY,), (market_id, winning_outcome));
            }

            pub fn get_settled_outcome(env: Env, market_id: u64) -> u32 {
                env.storage()
                    .persistent()
                    .get(&(symbol_short!("MKT"), market_id))
                    .expect("market not settled")
            }
        }
    }

    #[test]
    fn test_initialize_and_reporters() {
        let env = Env::default();
        let contract_id = env.register_contract(None, OddsOracle);
        let client = OddsOracleClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let pool = Address::generate(&env);
        let reporter1 = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin, &pool, &2u32);
        assert_eq!(client.get_quorum(), 2);
        assert!(!client.is_reporter(&reporter1));

        client.add_reporter(&reporter1);
        assert!(client.is_reporter(&reporter1));

        client.remove_reporter(&reporter1);
        assert!(!client.is_reporter(&reporter1));
    }

    #[test]
    fn test_report_result_quorum_settles_pool() {
        let env = Env::default();
        env.mock_all_auths();

        // Deploy the mock BettingPool
        let pool_id = env.register_contract(None, mock_pool::MockPool);
        let pool_client = mock_pool::MockPoolClient::new(&env, &pool_id);

        // Deploy OddsOracle pointing at the mock pool
        let oracle_id = env.register_contract(None, OddsOracle);
        let client = OddsOracleClient::new(&env, &oracle_id);

        let admin = Address::generate(&env);
        let r1 = Address::generate(&env);
        let r2 = Address::generate(&env);

        client.initialize(&admin, &pool_id, &2u32);
        client.add_reporter(&r1);
        client.add_reporter(&r2);

        let ext_id = soroban_sdk::Bytes::from_slice(&env, b"event_12345");
        let source = symbol_short!("SPORTR");

        // First report — quorum not reached yet
        client.report_result(&r1, &0u64, &1u32, &source, &ext_id);
        let pending = client.get_pending(&0u64);
        assert!(!pending.is_settled);

        // Second report — quorum reached, pool.settle_market must be called
        client.report_result(&r2, &0u64, &1u32, &source, &ext_id);
        let pending = client.get_pending(&0u64);
        assert!(pending.is_settled);

        // Verify the cross-contract call actually reached the mock pool
        let settled_outcome = pool_client.get_settled_outcome(&0u64);
        assert_eq!(settled_outcome, 1u32);
    }
}
