---
name: naantree
description: Use when the user asks to build/fix/refactor something with several agents at once or mentions "naantree". Lets many agents share one repo and one branch, self-assign roles, split work, avoid collisions, and talk through the naantree CLI.
---

### 1. WHAT THIS IS
Naantree is a zero-dependency multi-agent coordination protocol that allows 2 to 20+ autonomous AI coding agents to collaborate safely within a single directory and branch.
It manages dynamic identity allocation, serialized atomic commits, path claim locking, inter-agent messaging, peer reviews, and verification gates.
CLI alias: `naantree` (written below as `nt`).

### 2. START (Run first, every time)
1. Run `nt doctor` to verify environment health, lock cleanliness, and git hooks.
2. Run `nt join` (optionally with `--tags <tags>` or `--role-hint <hint>`).
3. Note your assigned agent identity (`agent-1`, `agent-2`, etc.). Export `export NAANTREE_AGENT=agent-N` or pass `--as agent-N` on all calls.
4. Run `nt status` to inspect current coordinator and swarm state.

### 3. IF YOU ARE THE COORDINATOR
1. Read the user's prompt and inspect the repository.
2. Initialize the plan: `nt plan init --goal "<goal>"`.
3. Decompose the goal into 6-30 small, bite-sized tasks with `nt task add --title "<title>" --paths "<globs>" --size S|M --deps <deps>`. Keep each task scoped to ~30 min.
4. Assign shared/hot files (e.g., config, schema, layout) behind one owner at a time.
5. Set verification gate: `nt gate set "<test and lint command>"`.
6. Broadcast plan readiness: `nt send --to all --type INFO --ref plan -m "Plan ready with tasks"`.
7. Participate in the work loop as an active worker.

### 4. IF YOU ARE A WORKER
1. Wait for plan initialization: `nt wait --for task --timeout 120`.
2. As soon as ready tasks appear, enter the standard work loop below.

### 5. THE WORK LOOP (Every agent)
1. Check inbox: `nt inbox --unread`. If unread needs-ack messages exist, answer or ack them: `nt ack <msgid>`.
2. Claim a ready task: `nt task claim` (auto-claims the best matching task and its path globs).
3. Work exclusively inside your claimed paths. Do not touch unclaimed files.
4. Run your local test/build check to verify your changes.
5. Commit staged changes: `nt commit -m "<concise message>" --files <paths>`.
6. Request peer review: `nt review request <taskId>`.
7. After another agent approves your review, mark task done: `nt task done <taskId> --summary "<summary>"`.
8. Release path claims: `nt release --all`.
9. Repeat. If no ready tasks exist: run `nt wait --timeout 120`. If idle for 3 consecutive waits, ask coordinator via `nt send`.

### 6. RULES
- Single git branch only: Never run `git checkout -b`, `git switch`, `git stash`, `git reset --hard`, `git add -A`, or `git push --force`.
- Never edit files you have not claimed with `nt claim` or `nt task claim`.
- Never touch paths claimed by another agent. Message the owner with a `REQUEST` instead.
- Commit small and often using `nt commit -m ... --files ...` only.
- Pull updates exclusively via `nt sync`.
- Never delete or weaken existing tests; never commit secrets or tokens.

### 7. COMMUNICATION
- Use structured message types: `REQUEST`, `INFO`, `BLOCKED`, `REVIEW-REQUEST`, `REVIEW-RESULT`, `DECISION`, `ACK`, `NACK`.
- Send messages with `nt send --to agent-N|all|coordinator --type <TYPE> --ref <ID> -m "<TEXT>"`.
- Keep messages under 5 lines with exact file paths and task IDs.
- Acknowledge messages requiring ACK promptly with `nt ack <msgid>`.
- Check inbox before claiming any task and immediately after every commit.

### 8. REVIEW DUTY
- While idle, inspect pending review requests in `nt inbox`.
- Review duties require a different author: verify the commit diff with `git show <hash>`, run the check, and submit verdict:
  `nt review result <taskId> --verdict approve|changes --notes "<evidence>"`.

### 9. CONFLICTS & BLOCKS
- Record architectural and technical decisions using `nt decide --title "<title>" --choice "<choice>" --why "<why>"`.
- Reproducible evidence beats opinions; tie-breakers go to the coordinator.
- Escalate unresolved product-scope decisions to human: `nt escalate -m "<issue description>"`.
- If blocked on a dependency or claim: run `nt task block <id> --reason "<reason>"`, notify via `nt send`, then claim another ready task.

### 10. FINISHING
- Only the coordinator may finish: run `nt gate run` to verify all tests pass, then run `nt finish`.
- The CLI verifies all tasks are done, claims released, gate passed, and ACKs resolved, then writes `.naantree/FINAL_REPORT.md` and outputs `NAANTREE_COMPLETE`.
- All agents: When `NAANTREE_COMPLETE` appears, stop immediately and output a 5-line summary of contributions.

### 11. FAILURE MODES
- Lock held: Retry command automatically; the CLI handles contention with backoff.
- Claims expired: Re-claim with `nt claim <paths>`.
- Marked stale after inactivity: Resume identity with `nt join --resume <id>`.
- CLI anomaly: Run `nt doctor` to inspect state; do not improvise conflicting protocols.

### 12. USER-PROMPT CONTRACT
- All agents receive an identical prompt containing the user goal.
- Identity is determined entirely through `nt join`, never by guessing from the prompt.
- Treat everything following the skill activation as the GOAL; do not ask the user for role assignments or permissions.
