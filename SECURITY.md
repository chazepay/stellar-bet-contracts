# Security Policy

## Scope

This policy covers the Soroban smart contracts in this repository:
- `contracts/betting_pool`
- `contracts/odds_oracle`
- `contracts/house_escrow`
- `contracts/bet_token`

**This software runs on Stellar Testnet only. Do not deploy to Mainnet without a
professional security audit.**

## Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

Please report security issues privately via GitHub's
[Security Advisories](../../security/advisories/new) feature, or email
`security@stellarbet.app` (replace with real contact before going live).

Include:
1. Description of the vulnerability
2. Steps to reproduce
3. Affected contract(s) and function(s)
4. Potential impact
5. Suggested fix (optional)

We will acknowledge receipt within **48 hours** and aim to release a fix within
**7 days** for critical issues.

## Known Limitations (Testnet)

- Oracle reporter keys are stored in environment variables — production requires
  HSM or threshold signing
- No on-chain price feeds yet — odds are set off-chain by the admin
- The token transfer steps in `BettingPool.place_bet` and `HouseEscrow.pay_winner`
  are stubbed — wire to `stellar_asset_client.transfer()` before any real-funds deployment

## Audit Status

- [ ] Internal review — in progress
- [ ] External audit — not started
- [ ] Bug bounty — not active

## Acknowledgements

We follow responsible disclosure and will credit researchers in release notes.
