use clap::Parser;
use naantree::cli::{
    Cli, Commands, CoordinatorSubcommand, GateSubcommand, HooksSubcommand, PlanSubcommand,
    ReviewSubcommand, TaskSubcommand,
};
use naantree::cmds;
use naantree::core::context::Context;

fn main() {
    let cli = Cli::parse();
    let json_mode = cli.json;
    let verbose = cli.verbose;

    let is_doctor = matches!(cli.command, Commands::Doctor);
    let ctx = match Context::init(cli.r#as, cli.session.clone(), !is_doctor) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error initializing context: {}", e);
            std::process::exit(1);
        }
    };

    let result = match cli.command {
        Commands::Join {
            role_hint,
            tags,
            resume,
            force,
        } => cmds::lifecycle::run_join(&ctx, role_hint, tags, cli.session, resume, force),

        Commands::Coordinator { cmd } => match cmd {
            CoordinatorSubcommand::Claim => cmds::lifecycle::run_coordinator_claim(&ctx),
        },

        Commands::Whoami => cmds::lifecycle::run_whoami(&ctx),

        Commands::Status => cmds::status::run_status(&ctx),

        Commands::Heartbeat => cmds::lifecycle::run_heartbeat(&ctx),

        Commands::Leave => cmds::lifecycle::run_leave(&ctx),

        Commands::Plan { cmd } => match cmd {
            PlanSubcommand::Init { goal, allow_push } => cmds::plan::run_plan_init(&ctx, goal, allow_push),
        },

        Commands::Task { cmd } => match cmd {
            TaskSubcommand::Add {
                title,
                desc,
                tags,
                paths,
                deps,
                size,
                for_agent,
                reviewers,
                kind,
            } => cmds::task::run_task_add(&ctx, title, desc, tags, paths, deps, size, for_agent, reviewers, kind),
            TaskSubcommand::Accept { id } => cmds::task::run_task_accept(&ctx, id),
            TaskSubcommand::List { state, mine, ready } => cmds::task::run_task_list(&ctx, state, mine, ready),
            TaskSubcommand::Claim { id } => cmds::task::run_task_claim(&ctx, id),
            TaskSubcommand::Update { id, state, note } => cmds::task::run_task_update(&ctx, id, state, note),
            TaskSubcommand::Done { id, summary, commit } => cmds::task::run_task_done(&ctx, id, summary, commit),
            TaskSubcommand::Block { id, reason } => cmds::task::run_task_block(&ctx, id, reason),
            TaskSubcommand::Split { id, into } => cmds::task::run_task_split(&ctx, id, into),
        },

        Commands::Claim {
            paths,
            task,
            ttl,
            shared,
            wait,
            timeout,
        } => cmds::claim::run_claim(&ctx, paths, task, ttl, shared, wait, timeout),

        Commands::Release { paths, all } => cmds::claim::run_release(&ctx, paths, all),

        Commands::Claims => cmds::claim::run_claims(&ctx),

        Commands::Commit { message, files } => cmds::git_cmds::run_commit(&ctx, message, files),

        Commands::Sync => cmds::git_cmds::run_sync(&ctx),

        Commands::Push => cmds::git_cmds::run_push(&ctx),

        Commands::Hooks { cmd } => match cmd {
            HooksSubcommand::Install => cmds::hooks::run_hooks_install(&ctx),
            HooksSubcommand::PreCommit => cmds::hooks::run_hooks_pre_commit(&ctx),
        },

        Commands::Send {
            to,
            r#type,
            r#ref,
            message,
            needs_ack,
        } => cmds::comm::run_send(&ctx, to, r#type, r#ref, message, needs_ack),

        Commands::Inbox { unread, mark_read } => cmds::comm::run_inbox(&ctx, unread, mark_read),

        Commands::Ack { msg_id } => cmds::comm::run_ack(&ctx, msg_id),

        Commands::Thread { ref_id } => cmds::comm::run_thread(&ctx, ref_id),

        Commands::Wait {
            for_condition,
            timeout,
        } => cmds::comm::run_wait(&ctx, for_condition, timeout),

        Commands::Decide {
            title,
            choice,
            why,
            alternatives,
        } => cmds::comm::run_decide(&ctx, title, choice, why, alternatives),

        Commands::Decisions => cmds::comm::run_decisions(&ctx),

        Commands::Escalate { message } => cmds::comm::run_escalate(&ctx, message),

        Commands::Review { cmd } => match cmd {
            ReviewSubcommand::Request { task_id, to } => cmds::finish::run_review_request(&ctx, task_id, to),
            ReviewSubcommand::Result {
                task_id,
                verdict,
                notes,
            } => cmds::finish::run_review_result(&ctx, task_id, verdict, notes),
        },

        Commands::Gate { cmd } => match cmd {
            GateSubcommand::Set { cmd } => cmds::finish::run_gate_set(&ctx, cmd),
            GateSubcommand::Run => cmds::finish::run_gate_run(&ctx),
        },

        Commands::Finish => cmds::finish::run_finish(&ctx),

        Commands::Log { tail } => cmds::finish::run_log(&ctx, tail),

        Commands::Doctor => cmds::finish::run_doctor(&ctx),

        Commands::Reset { confirm } => cmds::finish::run_reset(&ctx, confirm),
    };

    match result {
        Ok(out) => {
            out.print(json_mode, verbose);
        }
        Err(err) => {
            if json_mode {
                let err_obj = serde_json::json!({
                    "error": err
                });
                eprintln!("{}", err_obj);
            } else {
                eprintln!("Error: {}", err);
            }
            std::process::exit(1);
        }
    }
}
