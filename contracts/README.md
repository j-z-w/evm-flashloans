## Contracts Workspace

Foundry project used for local fork simulation and Base deployment.

## Quickstart

```bash
forge build
forge test -vv
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
