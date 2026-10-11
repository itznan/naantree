pub fn extract_id_num(id: &str) -> Option<u64> {
    id.split('-')
        .next_back()
        .and_then(|s| s.parse::<u64>().ok())
}

pub fn next_id_with_prefix(prefix: &str, existing: &[String]) -> String {
    let mut max_id = 0u64;
    for item in existing {
        if item.starts_with(prefix) {
            if let Some(num) = extract_id_num(item) {
                if num > max_id {
                    max_id = num;
                }
            }
        }
    }
    format!("{}-{}", prefix, max_id + 1)
}

pub fn next_agent_id(existing: &[String]) -> String {
    next_id_with_prefix("agent", existing)
}

pub fn next_task_id(existing: &[String]) -> String {
    next_id_with_prefix("task", existing)
}

pub fn next_msg_id(existing: &[String]) -> String {
    next_id_with_prefix("msg", existing)
}

pub fn next_dec_id(existing: &[String]) -> String {
    next_id_with_prefix("dec", existing)
}

pub fn next_claim_id(existing: &[String]) -> String {
    next_id_with_prefix("claim", existing)
}

pub fn next_evt_id(existing: &[String]) -> String {
    next_id_with_prefix("evt", existing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_ids() {
        assert_eq!(next_agent_id(&[]), "agent-1");
        assert_eq!(
            next_agent_id(&["agent-1".into(), "agent-2".into()]),
            "agent-3"
        );
        assert_eq!(extract_id_num("agent-42"), Some(42));
    }
}
