## Contracts Workspace

Foundry project used for local fork simulation and Base deployment.

## Quickstart

```bash
forge build
forge test -vv
```

## Deploy With Owner/Operator Split

Set these environment variables before running the deploy script:

```bash
export BALANCER_OWNER=0xYourMultisigAddress
export BALANCER_OPERATOR=0xYourBotSignerAddress
export BALANCER_VAULT=0xBA12222222228d8Ba445958a75a0704d566BF2C8
```

Run:

```bash
forge script script/DeployBalancerFlashLoanSimple.s.sol:DeployBalancerFlashLoanSimpleScript --rpc-url "$BASE_RPC_HTTPS_URL" --broadcast
```

## Base Fork Simulation

In terminal 1:

```bash
anvil --fork-url "$BASE_RPC_HTTPS_URL"
```

In terminal 2:

```bash
forge test -vv
```

## Notes

- RPC endpoint values come from environment variables in `foundry.toml`.
- Keep private keys in `.env` only (burner wallet only).
