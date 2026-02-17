# Testing-Phase Strategy (Foundry-First)

## Goal
Validate flash-loan execution safety and correctness in a deterministic Foundry fork environment before adding bot complexity.

## Why this is the smartest move right now
During testing, the biggest risk is mixing multiple moving parts (RPC setup, quote logic, bot scheduling, and contract execution) and then not knowing what failed.

A testing-first approach isolates the core contract behavior first:
- flash-loan callback correctness
- swap adapter correctness
- repayment guarantees
- profit guard enforcement

This gives fast feedback and makes failures easy to debug.

## Immediate Scope (No bot loop yet)
Build and test only:
- Flash-loan executor contract
- DEX swap adapters needed for route legs
- Foundry fork tests with hardcoded route vectors

Do not include yet:
- real-time off-chain route discovery
- automated transaction submission loop
- dynamic production quote orchestration

## Phase Order
1. Implement flash-loan callback flow + internal route execution wiring.
2. Implement adapter calls for planned route families.
3. Write deterministic Base-fork tests with fixed block number and fixed calldata vectors.
4. Prove happy path: borrow -> swap legs -> repay -> profit remains.
5. Prove failure path: slippage/deadline/auth checks revert safely.
6. After stable tests, add lightweight quote generation.
7. Only then introduce a full off-chain bot loop.

## Minimum Acceptance Checks
A strategy is considered valid only if tests prove all of the following:
- Callback can only be invoked by the expected vault.
- Unauthorized initiators cannot execute routes.
- Each leg enforces `minAmountOut`.
- Final repayment is fully covered (principal + fee).
- Net profit check is enforced before completion.
- Expired deadlines and replayed nonces revert.

## Recommended Test Setup
- Use Base mainnet fork via Alchemy RPC.
- Pin a fixed fork block in tests for repeatability.
- Keep token and pool set small at first.
- Start with hardcoded known-good vectors, then expand.

## What to delay until after contract tests are stable
- Route search optimization logic
- Multi-opportunity scheduling
- Production keeper/retry behavior
- Advanced gas bidding policy

## Prerequisites
- Foundry installed and working (`forge`, `anvil`, `cast`)
- Base RPC from Alchemy in `.env`
- Deterministic test configuration (fixed block, stable fixtures)

## Outcome
This approach gets you to a reliable "engine works" milestone quickly. Once that baseline is solid, adding quote/bot infrastructure is safer and significantly faster.