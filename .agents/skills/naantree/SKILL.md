---
name: naantree
description: Use when the user asks to build/fix/refactor something with several agents at once or mentions "naantree". Lets many agents share one repo and one branch, self-assign roles, split work, avoid collisions, and talk through the naantree CLI.
---

### 1. WHAT THIS IS
Naantree is a zero-dependency multi-agent coordination protocol that allows 2 to 20+ autonomous AI coding agents to collaborate safely within a single directory and branch.
It manages dynamic identity allocation, serialized atomic commits, path claim locking, inter-agent messaging, peer reviews, quality gates, and coordinator failover.
CLI alias: `naantree` (written below as `nt`).

### 2. START (Run first, every time)
1. Run `nt doctor` to verify environment health, lock cleanliness, and invariant health.
2. Run `nt join` (optionally with `--tags <tags>` or `--role-hint <hint>`).
3. **TRUST YOUR JOIN OUTPUT**: Inspect the printed `role=coordinator|worker` and `coordinator_alive=true|false`. Do NOT assume agent-1 is the coordinator or guess roles from the user prompt. Role is decided by liveness.
4. **ALWAYS PASS `--as agent-N`**: Tool calls execute in fresh subshells where environment variables (like `export NAANTREE_AGENT`) do not persist. Use the assigned prefix `--as agent-N` on every single command (e.g. `nt --as agent-N status`).
5. Run `nt --as agent-N status` to inspect current swarm state.

### 3. IF YOU ARE THE COORDINATOR (`role=coordinator`)
1. Read the user's prompt and inspect the repository.
2. Initialize the plan: `nt --as agent-N plan init --goal "<goal>"`.
3. Set verification gate early: `nt --as agent-N gate set "<test and lint command>"`. This runs a baseline check immediately to surface missing packages or environment errors.
4. If prerequisites or toolchain setups are needed, add prerequisite tasks: `nt --as agent-N task add --title "<env setup>" --kind env --paths "<paths>"`. Environment tasks automatically gate subsequent code tasks.
5. Decompose the goal into small, bite-sized tasks: `nt --as agent-N task add --title "<title>" --paths "<globs>" --size S|M --deps <deps>`. Keep each task scoped to ~30 min.
6. Assign shared/hot files (e.g., config, schema, layout) behind one owner at a time.
7. Broadcast plan readiness: `nt --as agent-N send --to all --type INFO --ref plan -m "Plan ready with tasks"`.
8. Participate in the work loop as an active worker.

### 4. IF YOU ARE A WORKER (`role=worker`)
1. Check coordinator liveness from `nt --as agent-N status`:
   - If `coordinator=none` or the coordinator is stale/dead: claim the coordinator role with `nt --as agent-N coordinator claim`.
   - If an active coordinator exists: wait for tasks: `nt --as agent-N wait --for task --timeout 120`.
2. As soon as ready tasks appear, enter the standard work loop below.

### 5. THE WORK LOOP (Every agent)
1. Check inbox: `nt --as agent-N inbox --unread`. If unread needs-ack messages exist, answer or ack them: `nt --as agent-N ack <msgid>`.
2. Claim a ready task: `nt --as agent-N task claim` (auto-claims the best matching task and its path globs).
3. Work exclusively inside your claimed paths. Do not touch unclaimed files.
4. Run your local test/build check to verify your changes.
5. Commit staged changes: `nt --as agent-N commit -m "<concise message>" --files <paths>`.
6. Request peer review: `nt --as agent-N review request <taskId>`.
7. After another agent approves your review, mark task done: `nt --as agent-N task done <taskId> --summary "<summary>"`.
8. Release path claims: `nt --as agent-N release --all`.
9. Repeat. If no ready tasks exist: run `nt --as agent-N wait --timeout 120`. If idle for 3 consecutive waits, ask coordinator via `nt --as agent-N send`.

### 6. EXPLICIT ANTI-PATTERNS (Never do these)
- **NEVER inspect system processes (`ps`, `pstree`, `pgrep`, `kill`)**: Naantree runs inside short-lived CLI invocations. Process trees reveal nothing about agent state.
- **NEVER inspect other agents' internal files**: Do not read transcript logs, session memory, or private scratch files of other agents.
- **NEVER guess or resume an arbitrary agent ID**: Never run `nt join --resume <id>` for an ID you were not personally given by `join`. Resuming live agents is forbidden.
- **NEVER rely on shell exports**: Fresh tool call shells lose exported environment variables. Always pass `--as agent-N`.
- **USE OFFICIAL NAANTREE COMMANDS ONLY**: Coordinate strictly via `send`, `inbox`, `status`, `log`, `task`, and `claim`.

### 7. RULES
- Single git branch only: Never run `git checkout -b`, `git switch`, `git stash`, `git reset --hard`, `git add -A`, or `git push --force`.
- Never edit files you have not claimed with `nt --as agent-N claim` or `nt --as agent-N task claim`.
- Never touch paths claimed by another agent. Message the owner with a `REQUEST` instead.
- Commit small and often using `nt --as agent-N commit -m ... --files ...` only.
- Pull updates exclusively via `nt --as agent-N sync`.
- Never delete or weaken existing tests; never commit secrets or tokens.

### 8. COMMUNICATION
- Use structured message types: `REQUEST`, `INFO`, `BLOCKED`, `REVIEW-REQUEST`, `REVIEW-RESULT`, `DECISION`, `ACK`, `NACK`.
- Send messages with `nt --as agent-N send --to agent-N|all|coordinator --type <TYPE> --ref <ID> -m "<TEXT>"`.
- Keep messages under 5 lines with exact file paths and task IDs.
- Acknowledge messages requiring ACK promptly with `nt --as agent-N ack <msgid>`.
- Check inbox before claiming any task and immediately after every commit.

### 9. REVIEW DUTY
- While idle, inspect pending review requests in `nt --as agent-N inbox`.
- Review duties require a different author: verify the commit diff with `git show <hash>`, run the check, and submit verdict:
  `nt --as agent-N review result <taskId> --verdict approve|changes --notes "<evidence>"`.

### 10. CONFLICTS & BLOCKS
- Record architectural and technical decisions using `nt --as agent-N decide --title "<title>" --choice "<choice>" --why "<why>"`.
- Reproducible evidence beats opinions; tie-breakers go to the coordinator.
- Escalate unresolved product-scope decisions to human: `nt --as agent-N escalate -m "<issue description>"`.
- If blocked on a dependency or claim: run `nt --as agent-N task block <id> --reason "<reason>"`, notify via `nt --as agent-N send`, then claim another ready task.

### 11. FINISHING
- Only the coordinator may finish: run `nt --as agent-N gate run` to verify all tests pass, then run `nt --as agent-N finish`.
- The CLI verifies all tasks are done, claims released, gate passed, and ACKs resolved, then writes `.naantree/FINAL_REPORT.md` and outputs `NAANTREE_COMPLETE`.
- All agents: When `NAANTREE_COMPLETE` appears, stop immediately and output a 5-line summary of contributions.

### 12. FAILURE MODES & FAILOVER
- **Coordinator unresponsiveness / stale coordinator**: If coordinator heartbeat expires (>90s) or status reports `coordinator=none`, any worker can atomically claim the role using `nt --as agent-N coordinator claim` or wait for election via `nt --as agent-N wait --for coordinator`.
- **Lock held**: Retry command automatically; the CLI handles contention with exponential backoff and stale lock reclamation.
- **Claims expired**: Re-claim with `nt --as agent-N claim <paths>`.
- **Marked stale after inactivity**: Resume your own assigned identity with `nt --as agent-N join --resume agent-N --force`.
- **CLI anomaly / health check failure**: Run `nt doctor` to inspect swarm invariants and health.

### 13. READY-TO-PASTE USER PROMPT TEMPLATE
Paste this identical prompt to all agents in the swarm:

```text
You are collaborating on a shared codebase on a single git branch using the naantree coordination CLI.
1. Run `naantree doctor` then `naantree join`.
2. Inspect the printed output: observe your assigned agent ID (`agent-N`) and role (`role=coordinator` or `role=worker`). Do not assume agent-1 is coordinator.
3. On every subsequent command, always explicitly pass `--as agent-N` (e.g. `naantree --as agent-N ...`).
4. If you are coordinator: run `naantree plan init --goal "..."`, set gate with `naantree gate set "..."`, decompose tasks with `naantree task add`, and notify workers.
5. If you are worker: run `naantree wait --for task --timeout 120`. If no coordinator is alive, run `naantree coordinator claim`.
6. Follow the naantree work loop (claim task, edit claimed files, test, commit, review, mark done, release claims).
Goal: <INSERT USER GOAL HERE>
```
