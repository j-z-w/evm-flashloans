# evm-flashloans

Exploring EVM Flash loans on L2's for funsies.

## Layout

- `bot/`: Rust off-chain bot (data ingestion, pathfinding, execution decisions)
- `contracts/`: Foundry Solidity project (flash-loan/swap execution contracts)
- `docs/ours/`: internal project docs
- `docs/third-party/`: local external docs clones

## Prereqs

- Rust stable toolchain
- Foundry (`forge`, `cast`, `anvil`, `chisel`)
- Base RPC provider (Alchemy/Infura)

## Environment

1. Copy `.env.example` to `.env`
2. Fill in your Base RPC URLs and burner key values

## Commands

### Rust bot

```bash
cargo check -p evm_flashloans_l2_arb
```

### Shadow Route Discovery (No Transaction Sends)

Runs one Base route (`WETH -> USDC` on V2, then `USDC -> WETH` on V3) and logs
`would_trade` / `would_skip` decisions as JSON lines. The V3 leg is quoted via
Uniswap QuoterV2 `eth_call` (no transaction broadcast).

```bash
cargo run -p evm_flashloans_l2_arb --bin shadow_route
```

Optional short run:

```bash
SHADOW_MAX_BLOCKS=10 cargo run -p evm_flashloans_l2_arb --bin shadow_route
```

Additional shadow logging controls:

- `SHADOW_SUMMARY_EVERY_BLOCKS` (default `25`): emit summary JSON every N blocks.
- `SHADOW_VERBOSE_BLOCK_LOGS` (default `false`): emit extra per-block diagnostics to stderr.

### Foundry contracts

```bash
cd contracts
forge build
forge test -vv
```

Fork test strictness:

- `REQUIRE_FORK_TESTS=true` forces `contracts/test/BalancerFlashLoanSimple.t.sol` to fail fast if `BASE_RPC_HTTPS_URL` is missing.

### Local Base fork simulation

```bash
anvil --fork-url "$BASE_RPC_HTTPS_URL"
```

Then in another shell:

```bash
cd contracts
forge test -vv
```

## Notes

Blah blah blah something something can't be bothered to write more.
