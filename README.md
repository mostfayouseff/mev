# Apex-MEV UltraSafe v12

Production-grade, deterministic, ultra-low-latency multi-hop arbitrage bot on Solana.

## Design Decisions

1. **Deterministic execution** — same pool state → same paths → same trade, every time. No randomness, no AI/ML in the hot path.
2. **Fail-fast pre-simulation** — every opportunity is simulated via `simulateTransaction` against the private RPC before any gas is paid. Unprofitable paths are cancelled immediately.
3. **Lock-free hot path** — pool registry uses `DashMap` (lock-free concurrent map); ingress uses bounded `crossbeam_channel`; config reads use `ArcSwap` (zero lock cost).
4. **Bellman-Ford negative cycle detection** — path finding uses log-space arithmetic to convert multiplicative profit into additive weights, making circular arb = negative cycle detection. Runs O(V×E) deterministically.
5. **Ternary-search input optimizer** — finds the optimal trade size for each path without floating-point instability, using 45 iterations for ~2⁻⁴⁵ precision.
6. **Atomic execution** — all hops are packed into a single VersionedTransaction (v0) with Address Lookup Tables. If any hop fails, the entire transaction reverts.
7. **Jito bundle support** — optional bundle submission with adaptive tip strategy (tip fraction adjusts based on landing rate).
8. **Circuit breaker** — trips on N consecutive losses; blocks all execution until manually reset.
9. **Hot-reload config** — send SIGHUP to reload `.env` without restarting.
10. **Prometheus-ready metrics** — every critical path emits counters, gauges, and histograms on `:9090/metrics`.

---

## Project Structure

```
apex-mev/
├── Cargo.toml                    # Workspace root
├── .env.example                  # Configuration template
├── README.md                     # This file
├── crates/
│   ├── common/                   # Error types, timing helpers, math utils
│   ├── types/                    # Canonical types: PoolState, ArbPath, Hop, TokenMint
│   ├── config/                   # Hot-reloadable BotConfig from env
│   ├── metrics/                  # Prometheus registry + HTTP server
│   ├── ingress/                  # Zero-copy WebSocket parser + eBPF-style filter
│   ├── core/                     # Path finding, registry, scorer, optimizer
│   ├── strategy/                 # Strategy loop, DEX adapters, hooks
│   ├── safety/                   # Pre-simulation, circuit breaker, risk checks
│   ├── execution/                # Transaction builder, rate limiter, submitter
│   └── jito-integration/         # Jito bundle client + adaptive tip strategy
└── tests/                        # Integration + property tests
```

---

## Prerequisites

- **Rust** 1.82+ (install via [rustup](https://rustup.rs))
- **Solana CLI** 1.18+ (`sh -c "$(curl -sSfL https://release.solana.com/stable/install)"`)
- A **private/dedicated RPC endpoint** (Helius, Triton, QuickNode, etc.)
- **Devnet SOL** for testing (`solana airdrop 2 --url devnet`)

---

## Local Development Setup

```bash
# 1. Clone the repository
git clone <repo-url>
cd apex-mev

# 2. Create your .env from the template
cp .env.example .env
# Edit .env: set RPC_URL_PRIVATE, WS_URL_PRIVATE, and PRIVATE_KEY

# 3. Build the workspace
cargo build

# 4. Run tests (unit + integration)
cargo test

# 5. Run with logging
RUST_LOG=info cargo run --release
```

---

## Configuration Reference

| Variable | Default | Description |
|---|---|---|
| `RPC_URL_PRIVATE` | required | Private RPC HTTP endpoint |
| `WS_URL_PRIVATE` | required | Private RPC WebSocket endpoint |
| `PRIVATE_KEY` | required | Base58-encoded 64-byte keypair |
| `EXECUTE_TRADES` | `false` | Submit real transactions |
| `DRY_RUN` | `true` | Log only, never submit |
| `MIN_PROFIT_LAMPORTS` | `10000` | Minimum net profit per trade |
| `MAX_POSITION_SIZE` | `200000000` | Max trade size (lamports) |
| `SLIPPAGE_BPS` | `50` | Slippage tolerance (basis points) |
| `PRIORITY_FEE_LAMPORTS` | `5000` | Priority fee per CU |
| `USE_JITO` | `false` | Enable Jito bundle submission |
| `JITO_TIP_LAMPORTS` | `10000` | Base Jito tip |
| `CIRCUIT_BREAKER_THRESHOLD` | `5` | Losses before circuit trips |
| `MAX_BORROW_RATIO` | `0.8` | Max borrow as fraction of balance |
| `MAX_HOPS` | `6` | Maximum path hops |
| `MAX_CANDIDATE_PATHS` | `512` | Paths evaluated per tick |
| `MAX_POOL_AGE_SLOTS` | `32` | Pool staleness threshold |
| `DRY_RUN` | `true` | Simulate only, never execute |

---

## Testing

```bash
# All tests
cargo test

# Specific crate
cargo test -p core

# Integration tests only
cargo test --test integration_test

# With output
cargo test -- --nocapture

# Property tests (proptest, 256 cases)
PROPTEST_CASES=256 cargo test
```

---

## Devnet Deployment

### Step 1: Generate a fresh keypair
```bash
solana-keygen new --outfile devnet-key.json
# Add the pubkey to your .env as PRIVATE_KEY
# Airdrop devnet SOL:
solana airdrop 2 --keypair devnet-key.json --url devnet
```

### Step 2: Configure .env for devnet
```
RPC_URL_PRIVATE=https://api.devnet.solana.com
WS_URL_PRIVATE=wss://api.devnet.solana.com
DRY_RUN=false
EXECUTE_TRADES=false    # Keep false initially
USE_JITO=false          # Jito not available on devnet
```

### Step 3: Paper-trade mode (recommended first)
```bash
EXECUTE_TRADES=false RUST_LOG=debug cargo run --release
```
Watch the logs — you should see paths being discovered and scored. Verify profit
estimates are reasonable before enabling live trading.

### Step 4: Enable live trading
```bash
# Only after you're satisfied with paper-trade output
EXECUTE_TRADES=true DRY_RUN=false cargo run --release
```

### Step 5: Monitor
```bash
# Prometheus metrics (run Prometheus + Grafana locally)
curl http://localhost:9090/metrics | grep apex_

# Key metrics to watch:
# apex_paths_profitable_total       — arb opportunities found
# apex_trades_confirmed_total       — trades that landed
# apex_profit_lamports_total        — cumulative profit
# apex_circuit_breaker_trips_total  — should stay 0
# apex_sim_failures_total           — high value = RPC issues
```

---

## Mainnet Deployment Roadmap

| Phase | Action |
|---|---|
| 1 | Run on devnet for 48h with paper trading; verify path quality |
| 2 | Enable `EXECUTE_TRADES=true` on devnet; confirm profit matches simulation |
| 3 | Security audit of execution & safety layers |
| 4 | Switch RPC to mainnet private endpoint; increase `MIN_PROFIT_LAMPORTS` |
| 5 | Start with `MAX_POSITION_SIZE=10000000` (0.01 SOL max); scale up over 1 week |
| 6 | Enable `USE_JITO=true`; monitor landing rate |
| 7 | Deploy Prometheus + Grafana; set alerts on `circuit_breaker_trips` |
| 8 | Expand pool list; tune `tip_fraction` for landing rate target >70% |

---

## Architecture Overview

```
WebSocket (private RPC)
      │
      ▼
[Ingress Layer]
  eBPF-style filter → zero-copy parser → PoolUpdateEvent → channel
      │
      ▼
[Core Engine]
  PoolRegistry (DashMap) → PathFinder (Bellman-Ford) → PathScorer → InputOptimizer
      │
      ▼
[Strategy Loop]
  StrategyHooks → Opportunity → channel
      │
      ▼
[Safety Layer]
  CircuitBreaker → RiskChecker → TransactionSimulator → ValidatedOpportunity
      │ (FAIL-FAST: drop if sim fails or profit < MIN)
      ▼
[Execution Layer]
  RateLimiter → TransactionBuilder → Submitter
      ├── Direct RPC (sendTransaction)
      └── Jito Bundle (sendBundle + tip)
      │
      ▼
[Metrics / Prometheus]
  apex_* counters, gauges, histograms → :9090/metrics
```

---

## Security Notes

- Private key is **never logged** — protected by `tracing`'s skip/redact.
- `DRY_RUN=true` by default; requires explicit opt-in to live trading.
- Circuit breaker prevents runaway losses from unexpected market conditions.
- All arithmetic uses `u128` intermediates to prevent overflow.
- No `unwrap()` in library code; all errors are propagated or fail-fast.
- Pre-simulation catches bad transactions before gas is spent.

---

## License

MIT
