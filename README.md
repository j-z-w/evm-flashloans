# evm-flashloans

Exploring EVM Flash loans on L2's for funsies.

## Layout

- `bot/`: Rust off-chain bot (data ingestion, pathfinding, execution decisions)
- `contracts/`: Foundry Solidity project (flash-loan/swap execution contracts)
- `balancer-docs/`: local external docs clone (ignored from git)

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

### Foundry contracts

```bash
cd contracts
forge build
forge test -vv
```

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
