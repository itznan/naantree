# Changelog

All notable changes to `naantree` will be documented in this file.

## [0.2.0] - 2026-11-21

### Fixed
- **Windows Process Liveness Check (`is_pid_alive`)**: Fixed `WaitForSingleObject` failure caused by missing `SYNCHRONIZE` access right in `OpenProcess`. Process alive checks and coordinator failover now function correctly on native Windows.

### Added
- **`naantree diff` Boundary Guard**: Added `naantree diff` command (with `--staged` support) to check uncommitted git modifications against the caller agent's active path claims. Warns immediately when edits touch unclaimed files or files claimed by other agents before committing.
- **Cross-Platform Release Matrix & macOS Support**: Added `x86_64-apple-darwin` and `aarch64-apple-darwin` targets to `.github/workflows/release.yml`, generating prebuilt binaries and SHA256 checksums for Apple Silicon and Intel Macs.
- **Standalone Zero-Rust Installers**: Added `install.sh` (POSIX shell for Linux/macOS) and `install.ps1` (PowerShell for Windows) for automated one-line installation without requiring Rust or Cargo.
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
