//! BettingPool Contract
//!
//! Core betting contract for the StellarBet prediction market platform.
//! Handles bet placement, pool management, and payout distribution.
//! Bets are settled by the authorized oracle contract.
//! XLM transfers are fully wired — stake flows bettor → escrow on place_bet,
//! escrow → bettor on claim_payout (via HouseEscrow.pay_winner), and
//! escrow → bettor on cancel_bet.

#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    token::Client as TokenClient,
    Address, Env, Symbol, Vec,
};

// ─── Storage Keys ───────────────────────────────────────────────────────────

const ADMIN_KEY:   Symbol = symbol_short!("ADMIN");
const ORACLE_KEY:  Symbol = symbol_short!("ORACLE");
const ESCROW_KEY:  Symbol = symbol_short!("ESCROW");
const TOKEN_KEY:   Symbol = symbol_short!("TOKEN");
const BET_COUNT:   Symbol = symbol_short!("BET_CNT");

// ─── Data Types ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum BetStatus {
    Open,
    Won,
    Lost,
    Cancelled,
    PendingSettlement,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Bet {
    pub id: u64,
    pub bettor: Address,
    pub market_id: u64,
    pub outcome_index: u32,
    pub stake_xlm: i128,        // in stroops (1 XLM = 10_000_000 stroops)
    pub odds_bps: u32,          // odds in basis points, e.g. 25000 = 2.5x
    pub potential_payout: i128, // stake * odds_bps / 10000
    pub status: BetStatus,
    pub created_ledger: u32,
    pub settled_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Market {
    pub id: u64,
    pub description: soroban_sdk::Bytes,
    pub sport: Symbol,
    pub outcome_count: u32,
    pub total_pool: i128,
    pub winning_outcome: i32, // -1 = unsettled, >=0 = index of winner
    pub is_open: bool,
    pub start_ledger: u32,
    pub close_ledger: u32,
}

// ─── Storage Helpers ─────────────────────────────────────────────────────────

fn bet_key(id: u64) -> (Symbol, u64) {
    (symbol_short!("BET"), id)
}

fn market_key(id: u64) -> (Symbol, u64) {
    (symbol_short!("MKT"), id)
}

fn market_count_key() -> Symbol {
    symbol_short!("MKT_CNT")
}

fn user_bets_key(addr: &Address) -> (Symbol, Address) {
    (symbol_short!("UBETS"), addr.clone())
}

// ─── HouseEscrow client (cross-contract call for pay_winner) ─────────────────

mod house_escrow {
    use soroban_sdk::{contractclient, Address, Env};

    #[allow(dead_code)]
    #[contractclient(name = "HouseEscrowClient")]
    pub trait HouseEscrow {
        fn pay_winner(env: Env, winner: Address, gross_amount: i128) -> i128;
    }
}

// ─── Contract ────────────────────────────────────────────────────────────────

#[contract]
pub struct BettingPool;

#[contractimpl]
impl BettingPool {
    /// Initialize the contract.
    /// admin:     contract owner who can manage markets
    /// oracle:    the OddsOracle contract address allowed to settle markets
    /// escrow:    the HouseEscrow contract address that holds liquidity
    /// xlm_token: the native XLM Stellar asset contract address
    pub fn initialize(
        env: Env,
        admin: Address,
        oracle: Address,
        escrow: Address,
        xlm_token: Address,
    ) {
        if env.storage().instance().has(&ADMIN_KEY) {
            panic!("already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&ADMIN_KEY, &admin);
        env.storage().instance().set(&ORACLE_KEY, &oracle);
        env.storage().instance().set(&ESCROW_KEY, &escrow);
        env.storage().instance().set(&TOKEN_KEY, &xlm_token);
        env.storage().instance().set(&BET_COUNT, &0u64);
        env.storage().instance().set(&market_count_key(), &0u64);
    }

    /// Create a new betting market. Only the admin can call this.
    pub fn create_market(
        env: Env,
        description: soroban_sdk::Bytes,
        sport: Symbol,
        outcome_count: u32,
        close_ledger: u32,
    ) -> u64 {
        let admin: Address = env.storage().instance().get(&ADMIN_KEY).unwrap();
        admin.require_auth();

        assert!(outcome_count >= 2 && outcome_count <= 10, "invalid outcome count");
        assert!(
            close_ledger > env.ledger().sequence(),
            "close_ledger must be in the future"
        );

        let mut count: u64 = env.storage().instance().get(&market_count_key()).unwrap();
        let market_id = count;
        count += 1;

        let market = Market {
            id: market_id,
            description,
            sport: sport.clone(),
            outcome_count,
            total_pool: 0,
            winning_outcome: -1,
            is_open: true,
            start_ledger: env.ledger().sequence(),
            close_ledger,
        };

        env.storage().persistent().set(&market_key(market_id), &market);
        env.storage().instance().set(&market_count_key(), &count);

        env.events()
            .publish((symbol_short!("MKT_NEW"),), (market_id, sport.clone()));

        market_id
    }

    /// Place a bet on a market outcome.
    /// Transfers `stake_xlm` stroops from `bettor` to the escrow contract.
    /// Returns the new bet ID.
    pub fn place_bet(
        env: Env,
        bettor: Address,
        market_id: u64,
        outcome_index: u32,
        stake_xlm: i128,
        odds_bps: u32,
    ) -> u64 {
        bettor.require_auth();

        assert!(stake_xlm >= 10_000_000, "minimum stake is 1 XLM");
        assert!(odds_bps >= 10_100, "minimum odds 1.01x");
        assert!(odds_bps <= 500_000, "maximum odds 50x");

        let mut market: Market = env
            .storage()
            .persistent()
            .get(&market_key(market_id))
            .expect("market not found");

        assert!(market.is_open, "market is closed");
        assert!(
            env.ledger().sequence() < market.close_ledger,
            "market betting window closed"
        );
        assert!(
            outcome_index < market.outcome_count,
            "invalid outcome index"
        );

        // Calculate potential payout: stake * odds_bps / 10000
        let potential_payout = stake_xlm
            .checked_mul(odds_bps as i128)
            .expect("overflow")
            / 10_000;

        // Transfer stake from bettor → escrow
        let escrow: Address = env.storage().instance().get(&ESCROW_KEY).unwrap();
        let xlm_token: Address = env.storage().instance().get(&TOKEN_KEY).unwrap();
        let token = TokenClient::new(&env, &xlm_token);
        token.transfer(&bettor, &escrow, &stake_xlm);

        let mut bet_count: u64 = env.storage().instance().get(&BET_COUNT).unwrap();
        let bet_id = bet_count;
        bet_count += 1;

        let bet = Bet {
            id: bet_id,
            bettor: bettor.clone(),
            market_id,
            outcome_index,
            stake_xlm,
            odds_bps,
            potential_payout,
            status: BetStatus::Open,
            created_ledger: env.ledger().sequence(),
            settled_ledger: 0,
        };

        env.storage().persistent().set(&bet_key(bet_id), &bet);
        env.storage().instance().set(&BET_COUNT, &bet_count);

        // Track bets per user
        let mut user_bets: Vec<u64> = env
            .storage()
            .persistent()
            .get(&user_bets_key(&bettor))
            .unwrap_or(Vec::new(&env));
        user_bets.push_back(bet_id);
        env.storage()
            .persistent()
            .set(&user_bets_key(&bettor), &user_bets);

        // Update market pool
        market.total_pool += stake_xlm;
        env.storage().persistent().set(&market_key(market_id), &market);

        env.events().publish(
            (symbol_short!("BET_NEW"),),
            (bet_id, bettor, market_id, outcome_index, stake_xlm),
        );

        bet_id
    }

    /// Settle a market with the winning outcome. Only callable by the oracle.
    pub fn settle_market(env: Env, market_id: u64, winning_outcome: u32) {
        let oracle: Address = env.storage().instance().get(&ORACLE_KEY).unwrap();
        oracle.require_auth();

        let mut market: Market = env
            .storage()
            .persistent()
            .get(&market_key(market_id))
            .expect("market not found");

        assert!(market.winning_outcome == -1, "market already settled");
        assert!(
            winning_outcome < market.outcome_count,
            "invalid winning outcome"
        );

        market.winning_outcome = winning_outcome as i32;
        market.is_open = false;
        env.storage().persistent().set(&market_key(market_id), &market);

        env.events()
            .publish((symbol_short!("MKT_SETL"),), (market_id, winning_outcome));
    }

    /// Claim payout for a settled bet.
    /// Winners: calls HouseEscrow.pay_winner which deducts fee and transfers
    /// net XLM to the bettor. Returns net payout in stroops.
    /// Losers: marks bet as lost, returns 0.
    pub fn claim_payout(env: Env, bet_id: u64) -> i128 {
        let mut bet: Bet = env
            .storage()
            .persistent()
            .get(&bet_key(bet_id))
            .expect("bet not found");

        bet.bettor.require_auth();
        assert!(matches!(bet.status, BetStatus::Open), "bet not claimable");

        let market: Market = env
            .storage()
            .persistent()
            .get(&market_key(bet.market_id))
            .expect("market not found");

        assert!(market.winning_outcome >= 0, "market not settled yet");

        if market.winning_outcome == bet.outcome_index as i32 {
            bet.status = BetStatus::Won;
            bet.settled_ledger = env.ledger().sequence();
            env.storage().persistent().set(&bet_key(bet_id), &bet);

            // Call HouseEscrow.pay_winner — deducts protocol fee, sends net to bettor
            let escrow: Address = env.storage().instance().get(&ESCROW_KEY).unwrap();
            let escrow_client = house_escrow::HouseEscrowClient::new(&env, &escrow);
            let net_payout = escrow_client.pay_winner(&bet.bettor, &bet.potential_payout);

            env.events().publish(
                (symbol_short!("CLAIM"),),
                (bet_id, bet.bettor.clone(), net_payout),
            );

            net_payout
        } else {
            bet.status = BetStatus::Lost;
            bet.settled_ledger = env.ledger().sequence();
            env.storage().persistent().set(&bet_key(bet_id), &bet);

            env.events()
                .publish((symbol_short!("LOST"),), (bet_id, bet.bettor.clone()));

            0
        }
    }

    /// Cancel a bet before the market closes. Refunds stake minus 1% fee.
    /// Transfers refund from escrow back to bettor via native XLM token.
    pub fn cancel_bet(env: Env, bet_id: u64) -> i128 {
        let mut bet: Bet = env
            .storage()
            .persistent()
            .get(&bet_key(bet_id))
            .expect("bet not found");

        bet.bettor.require_auth();
        assert!(matches!(bet.status, BetStatus::Open), "bet not cancellable");

        let market: Market = env
            .storage()
            .persistent()
            .get(&market_key(bet.market_id))
            .expect("market not found");

        assert!(market.is_open, "market already settled, use claim_payout");

        // 1% cancellation fee; remainder refunded
        let fee = bet.stake_xlm / 100;
        let refund = bet.stake_xlm - fee;

        bet.status = BetStatus::Cancelled;
        bet.settled_ledger = env.ledger().sequence();
        env.storage().persistent().set(&bet_key(bet_id), &bet);

        // Transfer refund from escrow → bettor
        let escrow: Address = env.storage().instance().get(&ESCROW_KEY).unwrap();
        let xlm_token: Address = env.storage().instance().get(&TOKEN_KEY).unwrap();
        let token = TokenClient::new(&env, &xlm_token);
        token.transfer(&escrow, &bet.bettor, &refund);

        env.events()
            .publish((symbol_short!("CANCEL"),), (bet_id, refund));

        refund
    }

    // ─── View Functions ───────────────────────────────────────────────────

    pub fn get_bet(env: Env, bet_id: u64) -> Bet {
        env.storage()
            .persistent()
            .get(&bet_key(bet_id))
            .expect("bet not found")
    }

    pub fn get_market(env: Env, market_id: u64) -> Market {
        env.storage()
            .persistent()
            .get(&market_key(market_id))
            .expect("market not found")
    }

    pub fn get_user_bets(env: Env, user: Address) -> Vec<u64> {
        env.storage()
            .persistent()
            .get(&user_bets_key(&user))
            .unwrap_or(Vec::new(&env))
    }

    pub fn get_bet_count(env: Env) -> u64 {
        env.storage().instance().get(&BET_COUNT).unwrap_or(0)
    }

    pub fn get_market_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&market_count_key())
            .unwrap_or(0)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

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
        token_admin.mint(admin, &1_000_000_000_000i128);
        token_id.address()
    }

    #[test]
    fn test_initialize() {
        let env = Env::default();
        let contract_id = env.register_contract(None, BettingPool);
        let client = BettingPoolClient::new(&env, &contract_id);

        let admin   = Address::generate(&env);
        let oracle  = Address::generate(&env);
        let escrow  = Address::generate(&env);
        let xlm     = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin, &oracle, &escrow, &xlm);
        assert_eq!(client.get_bet_count(), 0);
        assert_eq!(client.get_market_count(), 0);
    }

    #[test]
    fn test_create_market() {
        let env = Env::default();
        let contract_id = env.register_contract(None, BettingPool);
        let client = BettingPoolClient::new(&env, &contract_id);

        let admin  = Address::generate(&env);
        let oracle = Address::generate(&env);
        let escrow = Address::generate(&env);
        let xlm    = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin, &oracle, &escrow, &xlm);

        let desc = soroban_sdk::Bytes::from_slice(&env, b"Man Utd vs Arsenal");
        let market_id = client.create_market(&desc, &symbol_short!("SOCCER"), &3u32, &10000u32);
        assert_eq!(market_id, 0);

        let market = client.get_market(&0u64);
        assert!(market.is_open);
        assert_eq!(market.winning_outcome, -1);
    }

    #[test]
    fn test_place_and_settle_bet() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, BettingPool);
        let client = BettingPoolClient::new(&env, &contract_id);

        let admin  = Address::generate(&env);
        let oracle = Address::generate(&env);
        let escrow = Address::generate(&env);
        let bettor = Address::generate(&env);

        let xlm_token = setup_token(&env, &admin);
        let token = TokenClient::new(&env, &xlm_token);

        // Fund bettor with 100 XLM
        let token_admin = TokenAdmin::new(&env, &xlm_token);
        token_admin.mint(&bettor, &1_000_000_000i128);

        client.initialize(&admin, &oracle, &escrow, &xlm_token);

        let desc = soroban_sdk::Bytes::from_slice(&env, b"Test Match");
        let market_id = client.create_market(&desc, &symbol_short!("SOCCER"), &2u32, &10000u32);

        // Place 10 XLM at 2.0x (20000 bps)
        let stake = 100_000_000i128; // 10 XLM
        let bet_id = client.place_bet(&bettor, &market_id, &0u32, &stake, &20000u32);

        assert_eq!(bet_id, 0);
        // Stake should have moved from bettor to escrow
        assert_eq!(token.balance(&bettor), 1_000_000_000 - stake);
        assert_eq!(token.balance(&escrow), stake);

        let bet = client.get_bet(&bet_id);
        assert_eq!(bet.potential_payout, 200_000_000i128); // 2x

        // Settle market — outcome 0 wins
        client.settle_market(&market_id, &0u32);
        assert!(!client.get_market(&market_id).is_open);
    }
}
