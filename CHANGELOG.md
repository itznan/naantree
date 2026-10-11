# Changelog

All notable changes to `naantree` will be documented in this file.

## [Unreleased] - 2026-10-11

### Added
- **Dynamic Role Assignment & Single Coordinator Invariant**:
  - `join` reaps stale agents under lock before identity and role allocation.
  - Role is determined strictly by coordinator liveness (not ID number).
  - Enforced single live coordinator invariant in `join`, `resume`, and `reap`.
  - Added `--force` flag for resuming live agent IDs.
  - Superseded coordinator IDs are automatically demoted to worker upon resume if another coordinator is live.
  - Added `naantree coordinator claim` to claim coordinator duties when current coordinator is stale.
  - Added `naantree wait --for coordinator` to block until an active coordinator is present.
- **Robust Multi-Shell Identity Resolution**:
  - Resolution precedence: `--as`, `NAANTREE_AGENT`, `NAANTREE_SESSION` / session files keyed by session ID, parent PID, grandparent PID, or TTY.
  - `join` outputs exact `--as agent-N` prefix to use on every command.
- **Liveness & Heartbeat Refinement**:
  - Every command automatically refreshes caller heartbeat under lock.
  - Default agent stale timeout lowered to 90s (configurable via `NAANTREE_STALE_TIMEOUT_MS`).
  - Next command from any worker triggers dead coordinator reaping and automatic failover.
  - `status` displays per-agent heartbeat ages and coordinator liveness state.
- **Comprehensive Doctor Invariant Checks**:
  - `naantree doctor` verifies: multiple live coordinators, lack of live coordinator, ghost agents (un-reaped or never resumed), dead recorded PIDs, and claims held by stale agents.
  - Exits non-zero on invariant violations.
- **Quality Gate Baseline & Environment Tasks**:
  - `naantree gate set` executes a baseline check immediately to detect environment issues before tasks begin.
  - Added `task add --kind env` for prerequisite environment tasks that gate code tasks until completed.
  - `task add` warns when `--paths` globs overlap with existing tasks.
- **Skill Updates**:
  - Rewrote START, anti-patterns, and failure sections in `SKILL.md`.
  - Added ready-to-paste user-prompt template that does not hardcode role/IDs.
