use clap::{Parser, Subcommand};

const VERSION_INFO: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_HASH"), ")");

#[derive(Parser, Debug)]
#[command(
    name = "naantree",
    author,
    version = VERSION_INFO,
    about = "Modular multi-agent coordination CLI"
)]
pub struct Cli {
    #[arg(long, global = true, help = "Run as specified agent ID")]
    pub r#as: Option<String>,

    #[arg(long, global = true, help = "Session key for identity resolution")]
    pub session: Option<String>,

    #[arg(long, global = true, help = "Output structured JSON")]
    pub json: bool,

    #[arg(long, global = true, help = "Print full output without truncation")]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(about = "Join the swarm and get an assigned identity")]
    Join {
        #[arg(long)]
        role_hint: Option<String>,
        #[arg(long)]
        tags: Option<String>,
        #[arg(long)]
        resume: Option<String>,
        #[arg(long)]
        force: bool,
    },

    #[command(about = "Coordinator operations")]
    Coordinator {
        #[command(subcommand)]
        cmd: CoordinatorSubcommand,
    },

    #[command(about = "Display caller identity")]
    Whoami,

    #[command(about = "Report current swarm, task, and lock status")]
    Status,

    #[command(about = "Send a heartbeat to keep caller agent alive")]
    Heartbeat,

    #[command(about = "Leave the swarm gracefully")]
    Leave,

    #[command(about = "Planning operations")]
    Plan {
        #[command(subcommand)]
        cmd: PlanSubcommand,
    },

    #[command(about = "Task board operations")]
    Task {
        #[command(subcommand)]
        cmd: TaskSubcommand,
    },

    #[command(about = "Claim path(s) to avoid edit collisions")]
    Claim {
        #[arg(required = true)]
        paths: Vec<String>,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        ttl: Option<String>,
        #[arg(long)]
        shared: bool,
        #[arg(long)]
        wait: bool,
        #[arg(long)]
        timeout: Option<u64>,
    },

    #[command(about = "Release path claim(s)")]
    Release {
        paths: Vec<String>,
        #[arg(long)]
        all: bool,
    },

    #[command(about = "List all active claims")]
    Claims,

    #[command(about = "Safely stage, commit, and pull --rebase on a single branch")]
    Commit {
        #[arg(short = 'm', long, required = true)]
        message: String,
        #[arg(long, required = true, value_delimiter = ',')]
        files: Vec<String>,
    },

    #[command(about = "Safe git pull --rebase under lock")]
    Sync,

    #[command(about = "Coordinator-only git push")]
    Push,

    #[command(about = "Manage git hooks")]
    Hooks {
        #[command(subcommand)]
        cmd: HooksSubcommand,
    },

    #[command(about = "Send a structured message")]
    Send {
        #[arg(long, required = true)]
        to: String,
        #[arg(long, required = true)]
        r#type: String,
        #[arg(long, required = true)]
        r#ref: String,
        #[arg(short = 'm', long, required = true)]
        message: String,
        #[arg(long)]
        needs_ack: bool,
    },

    #[command(about = "View inbox messages")]
    Inbox {
        #[arg(long)]
        unread: bool,
        #[arg(long)]
        mark_read: bool,
    },

    #[command(about = "Acknowledge a message")]
    Ack { msg_id: String },

    #[command(about = "View message thread")]
    Thread { ref_id: String },

    #[command(about = "Wait for new events without burning tokens")]
    Wait {
        #[arg(long = "for")]
        for_condition: Option<String>,
        #[arg(long)]
        timeout: Option<u64>,
    },

    #[command(about = "Record a project decision")]
    Decide {
        #[arg(long, required = true)]
        title: String,
        #[arg(long, required = true)]
        choice: String,
        #[arg(long, required = true)]
        why: String,
        #[arg(long)]
        alternatives: Option<String>,
    },

    #[command(about = "List recorded decisions")]
    Decisions,

    #[command(about = "Escalate an issue to the human queue")]
    Escalate {
        #[arg(short = 'm', long, required = true)]
        message: String,
    },

    #[command(about = "Code review operations")]
    Review {
        #[command(subcommand)]
        cmd: ReviewSubcommand,
    },

    #[command(about = "Quality gate verification")]
    Gate {
        #[command(subcommand)]
        cmd: GateSubcommand,
    },

    #[command(about = "Coordinator finish verification")]
    Finish,

    #[command(about = "View audit event log")]
    Log {
        #[arg(long)]
        tail: Option<usize>,
    },

    #[command(about = "Verify environment health")]
    Doctor,

    #[command(about = "Reset all state")]
    Reset {
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum PlanSubcommand {
    #[command(about = "Initialize project goal")]
    Init {
        #[arg(long, required = true)]
        goal: String,
        #[arg(long)]
        allow_push: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum TaskSubcommand {
    #[command(about = "Add a task")]
    Add {
        #[arg(long, required = true)]
        title: String,
        #[arg(long)]
        desc: Option<String>,
        #[arg(long)]
        tags: Option<String>,
        #[arg(long)]
        paths: Option<String>,
        #[arg(long)]
        deps: Option<String>,
        #[arg(long)]
        size: Option<String>,
        #[arg(long = "for")]
        for_agent: Option<String>,
        #[arg(long)]
        reviewers: Option<usize>,
        #[arg(long)]
        kind: Option<String>,
    },

    #[command(about = "Accept proposed task")]
    Accept { id: String },

    #[command(about = "List tasks")]
    List {
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        mine: bool,
        #[arg(long)]
        ready: bool,
    },

    #[command(about = "Claim a task")]
    Claim { id: Option<String> },

    #[command(about = "Update a task")]
    Update {
        id: String,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },

    #[command(about = "Mark task done")]
    Done {
        id: String,
        #[arg(long, required = true)]
        summary: String,
        #[arg(long)]
        commit: Option<String>,
    },

    #[command(about = "Mark task blocked")]
    Block {
        id: String,
        #[arg(long, required = true)]
        reason: String,
    },

    #[command(about = "Split task into subtasks")]
    Split {
        id: String,
        #[arg(long)]
        into: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum HooksSubcommand {
    #[command(about = "Install pre-commit hook")]
    Install,
    #[command(about = "Pre-commit hook entrypoint")]
    PreCommit,
}

#[derive(Subcommand, Debug)]
pub enum ReviewSubcommand {
    #[command(about = "Request a review")]
    Request {
        task_id: String,
        #[arg(long)]
        to: Option<String>,
    },
    #[command(about = "Submit a review verdict")]
    Result {
        task_id: String,
        #[arg(long, required = true)]
        verdict: String,
        #[arg(long, required = true)]
        notes: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum GateSubcommand {
    #[command(about = "Set verification command")]
    Set { cmd: String },
    #[command(about = "Run verification command under lock")]
    Run,
}

#[derive(Subcommand, Debug)]
pub enum CoordinatorSubcommand {
    #[command(about = "Claim the coordinator role if the current coordinator is stale")]
    Claim,
}
