# Future Operating Playbook (Generalized)

## Purpose
Define a repeatable, low-risk process for moving any EVM arbitrage strategy from idea to production without skipping safety gates.

## Core Principles
1. Safety before speed: if controls and observability are weak, do not scale.
2. One variable at a time: isolate changes so failures are diagnosable.
3. Reversible rollout: every phase must have an immediate stop path.
4. Data first: execution quality is measured, not assumed.
5. Caps before confidence: exposure limits rise only after evidence.

## Deployment Model
1. Use role separation by default.
2. `owner` should be a cold multisig for admin/risk controls.
3. `operator` should be a hot bot signer limited to execution only.
4. Keep on-chain contract paused until preflight checks are complete.

## Strategy Lifecycle
### Phase 0: Design
1. Define exact venue pairings, route types, and constraints.
2. Define failure assumptions and worst-case loss envelope.
3. Write clear go/no-go metrics before any live attempts.

### Phase 1: Deterministic Simulation
1. Validate contracts on fork with fixed vectors and fixed blocks.
2. Prove happy path and failure path behavior.
3. Verify only intended callers and roles can execute privileged actions.

### Phase 2: Shadow Mode (No Sends)
1. Run block-by-block quoting and decision logic live.
2. Log `would_trade` and `would_skip` with reason codes.
3. Track data quality, quote failure rate, and RPC stability.

### Phase 3: Manual Canary
1. Unpause only for tightly controlled canary windows.
2. Execute tiny notional with manual approval per transaction.
3. Cap total attempts and stop immediately on anomalies.

### Phase 4: Low-Frequency Automation
1. Enable automated sends only after canary metrics pass.
2. Keep conservative gas and frequency constraints.
3. Keep kill switch and pause playbook continuously tested.

### Phase 5: Gradual Scale
1. Increase route count and notional caps incrementally.
2. Raise one limit at a time, then observe.
3. Roll back immediately if quality metrics regress.

## Guardrails by Layer
### On-Chain
1. Pause switch.
2. Token allowlist.
3. Per-token max size caps.
4. Fee ceiling checks.
5. Callback sender and payload invariants.
6. Safe token transfer handling and repayment invariants.

### Bot/Off-Chain
1. Minimum expected net profit after gas and loan fee.
2. Maximum gas price and priority fee.
3. Cooldown between attempts.
4. Max daily attempts and max daily realized loss.
5. Strategy-specific reason codes for all skips/rejections.

### Operations
1. Real-time alerting on reverts, RPC failures, drawdown spikes.
2. Signed runbooks for pause, rollback, and key rotation.
3. Daily post-run summary and exception review.

## Data and Quality Standards
1. Quote methods must be execution-accurate where possible.
2. Use integer math for threshold decisions.
3. Mark stale or inconsistent data explicitly and skip.
4. Treat quote errors as expected events, not crashes.
5. Retain structured logs for replay and audit.

## Go/No-Go Gate Template
1. Revert rate below threshold.
2. Net outcome per attempt above threshold.
3. Quote error rate below threshold.
4. RPC health within threshold.
5. No unresolved Sev-1 or Sev-2 incidents.

If any gate fails, return to the previous phase.

## Incident Policy
1. Any unexpected revert cluster triggers immediate pause.
2. Any daily loss breach triggers immediate pause.
3. Any abnormal RPC instability triggers no-send mode.
4. Restart only after root-cause analysis and mitigation is merged.

## Change Management
1. Every strategy change must include tests and rollback plan.
2. Separate infra changes from strategy changes when possible.
3. Never combine high-risk changes in one rollout.
4. Keep baseline-safe config checked in with explicit defaults.

## Minimal Weekly Operating Rhythm
1. Review prior week metrics and incidents.
2. Propose one controlled improvement.
3. Validate in simulation first.
4. Canary only if all gates pass.
5. Record outcomes and next limits.
