use crate::core::context::Context;
use crate::core::format::CommandOutput;
use crate::core::lock::with_lock;

pub fn run_plan_init(
    ctx: &Context,
    goal: String,
    allow_push: bool,
) -> Result<CommandOutput, String> {
    let agent_id = ctx.require_agent()?;
    with_lock(&ctx.state_dir, &agent_id, || {
        let agents_data = ctx.store.load_agents();
        let agent = agents_data
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| format!("Agent {} not found", agent_id))?;

        if agent.role != "coordinator" {
            return Err("Only the coordinator can initialize the plan (plan init).".to_string());
        }

        let mut meta = ctx.store.load_meta();
        meta.goal = Some(goal.clone());
        meta.allow_push = allow_push;
        ctx.store.save_meta(&meta)?;

        let _ = ctx.store.log_event(
            "plan_init",
            Some(&agent_id),
            serde_json::json!({
                "goal": goal,
                "allow_push": allow_push,
                "coordinator": agent_id
            }),
        );

        Ok(CommandOutput::new()
            .line(format!(
                "goal=\"{}\" coordinator={} allow_push={}",
                goal, agent_id, allow_push
            ))
            .json(&serde_json::json!({
                "goal": goal,
                "coordinator": agent_id,
                "allow_push": allow_push
            }))
            .next("naantree task add --title \"<title>\""))
    })
}
