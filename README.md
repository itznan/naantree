# Naantree

**Modular Multi-Agent Coordination Protocol & CLI**

Naantree allows 2 to 20+ autonomous AI coding agents to collaborate safely in the **same repository folder** and on the **same git branch**. Agents are given an identical prompt and autonomously organize: they auto-assign identities, elect a coordinator, decompose work into collision-free tasks, communicate, conduct peer reviews, and safely finish.

Official Skill Package: [github.com/itznan/naantree-skill](https://github.com/itznan/naantree-skill)

---

## Installation via `skills.sh`

You can install the `naantree` skill directly into your AI coding agents using `npx skills`:

```bash
# Add official skill package
npx skills add itznan/naantree-skill

# Or install directly from this repository
npx skills add itznan/naantree

# Or install globally across your machine
npx skills add -g itznan/naantree-skill
```

### Adding to Gemini & AI Coding Agents
- **Google Gemini / Antigravity**: Drop `SKILL.md` into `.agent/skills/naantree/SKILL.md` or `.gemini/skills/naantree/SKILL.md`, or run `npx skills add itznan/naantree-skill`.
- **Claude Code**: `.claude/skills/naantree/SKILL.md` or `~/.claude/skills/naantree/SKILL.md`.
- **Cursor**: `.cursor/skills/naantree/SKILL.md` or `.agents/skills/naantree/SKILL.md`.
- **GitHub Copilot**: `.github/skills/naantree/SKILL.md`.

---

## Key Features

1. **Self-Organizing Lifecycle**:
   - `naantree join` atomically assigns monotonic agent IDs (`agent-1`, `agent-2`...).
   - Role is determined by liveness (not ID number): if no live coordinator exists, the joining agent becomes coordinator. Invariant: at most one live coordinator.
   - Automatic dead-agent reaping: inactive agents (>90s default, configurable via `NAANTREE_STALE_TIMEOUT_MS`) are declared stale, their tasks return to `todo`, claims are released, and coordinator duties failover automatically.
   - Identity survives fresh subshells via `--as agent-N`, `NAANTREE_AGENT`, or session files keyed by session ID, parent PID, and TTY.

2. **Collision Control via Path Claims**:
   - `naantree claim <paths...>` locks files and directories (with recursive directory and glob pattern matching).
   - Conflicting claims fail fast, identifying the current claim owner and remaining TTL.
   - Shared "hot" files (e.g., `package.json`, `Cargo.lock`, config files) can be claimed with `--shared` queues.

3. **Serialized Git on a Single Branch**:
   - No branching, no worktrees, no merge conflicts.
   - `naantree commit -m "..." --files <files...>` validates path ownership, acquires an atomic global git lock, stages only claimed files, retries on `.git/index.lock` contention with backoff, and rebases cleanly.
   - Pre-commit guard (`naantree hooks install`) rejects unauthorized edits and prevents out-of-protocol commits.

4. **Task Board & Peer Review**:
   - Tasks support dependency graphs (`--deps`), sizes (`S`, `M`, `L`), and tags.
   - Claiming a task atomically claims its associated paths.
   - Tasks require review approval from an independent peer agent (`naantree review request` / `naantree review result`) before they can be marked `done`.

5. **Inter-Agent Communication & Blocking Wait**:
   - Structured messaging (`REQUEST`, `INFO`, `BLOCKED`, `REVIEW-REQUEST`, `REVIEW-RESULT`, `DECISION`, `ACK`).
   - `naantree wait` provides an efficient polling mechanism (checking every 2s) that returns as soon as a relevant event occurs, preventing idle agents from burning LLM tokens.

6. **Quality Gate & Final Report**:
   - Coordinators set project-wide verification commands (`naantree gate set "cargo test"`).
   - `naantree finish` validates that all tasks are done, claims released, verification passed, and all required ACKs resolved, then produces `.naantree/FINAL_REPORT.md` and outputs `NAANTREE_COMPLETE`.

---

## Command Reference

| Command | Description |
|---|---|
| `naantree doctor` | Verifies environment, toolchain, git repo, locks, and swarm invariants |
| `naantree join [--tags t1,t2] [--session <key>] [--resume <id> [--force]]` | Joins swarm and obtains an agent identity |
| `naantree coordinator claim` | Claims coordinator role if current coordinator is stale |
| `naantree whoami` | Displays caller's agent ID, role, status, and PID |
| `naantree status` | Overview of coordinator state, heartbeat ages, task board, and claims |
| `naantree plan init --goal "..."` | (Coordinator) Initializes the swarm's objective |
| `naantree task add --title "..." --paths "..." [--kind env]` | Creates a task on the task board (warns on path overlap) |
| `naantree task claim [id]` | Atomically claims a task and its paths |
| `naantree claim <paths...> [--shared]` | Claims path(s) to avoid collisions |
| `naantree release <paths...\|--all>` | Releases claimed paths |
| `naantree commit -m "..." --files <files>` | Safely commits claimed files under lock |
| `naantree review request <taskId>` | Requests peer review for completed work |
| `naantree review result <taskId> --verdict approve` | Submits peer review verdict |
| `naantree task done <taskId> --summary "..."` | Marks reviewed task as complete |
| `naantree send --to <target> --type <type> -m "..."` | Sends structured message to peer(s) |
| `naantree inbox [--unread]` | Reads incoming messages |
| `naantree wait [--for task\|message\|coordinator] [--timeout 120]` | Waits for events without token waste |
| `naantree gate set "<cmd>"` | Sets gate command and runs baseline verification check |
| `naantree gate run` | Executes quality gate command under lock |
| `naantree finish` | Validates completion and outputs `NAANTREE_COMPLETE` |

---

## Building & Testing

```bash
# Build release binary
cargo build --release

# Run full test suite (unit tests, 10-process concurrency test, 4-agent dry run)
cargo test
```
