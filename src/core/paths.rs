use glob::Pattern;

pub fn normalize_path(p: &str) -> String {
    let mut s = p.trim().replace('\\', "/");
    while s.starts_with("./") {
        s = s[2..].to_string();
    }
    while s.starts_with('/') {
        s = s[1..].to_string();
    }
    while s.ends_with('/') && !s.is_empty() {
        s.pop();
    }
    s
}

/// Check if a single target file is covered by a claim pattern.
pub fn file_matches_pattern(claim_pattern: &str, file_path: &str) -> bool {
    let pat = normalize_path(claim_pattern);
    let target = normalize_path(file_path);

    if pat.is_empty() || target.is_empty() {
        return false;
    }

    if pat == target {
        return true;
    }

    // Recursive directory match: claim "src/auth" covers "src/auth/login.rs"
    if target.starts_with(&(pat.clone() + "/")) {
        return true;
    }

    // Pattern ending with /** covers anything inside
    if pat.ends_with("/**") {
        let prefix = &pat[..pat.len() - 3];
        if target == prefix || target.starts_with(&(prefix.to_string() + "/")) {
            return true;
        }
    }

    // Pattern ending with /* covers direct children
    if pat.ends_with("/*") {
        let prefix = &pat[..pat.len() - 2];
        if target.starts_with(&(prefix.to_string() + "/")) {
            let rest = &target[prefix.len() + 1..];
            if !rest.contains('/') {
                return true;
            }
        }
    }

    // Standard glob pattern match with literal separator
    if let Ok(pattern) = Pattern::new(&pat) {
        let opts = glob::MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: false,
        };
        if pattern.matches_with(&target, opts) {
            return true;
        }
    }

    false
}

/// Check if two claim patterns overlap each other.
pub fn paths_overlap(p1: &str, p2: &str) -> bool {
    let n1 = normalize_path(p1);
    let n2 = normalize_path(p2);

    if n1.is_empty() || n2.is_empty() {
        return false;
    }

    if n1 == n2 {
        return true;
    }

    // Check directory prefix containment
    if n1.starts_with(&(n2.clone() + "/")) || n2.starts_with(&(n1.clone() + "/")) {
        return true;
    }

    // Check if one pattern matches the other
    if file_matches_pattern(&n1, &n2) || file_matches_pattern(&n2, &n1) {
        return true;
    }

    // Check if both have glob patterns sharing base directory
    let base1 = get_glob_base(&n1);
    let base2 = get_glob_base(&n2);

    if !base1.is_empty() && !base2.is_empty() {
        if base1 == base2
            || base1.starts_with(&(base2.clone() + "/"))
            || base2.starts_with(&(base1.clone() + "/"))
        {
            // If both are wildcard patterns under the same directory, check potential collision
            if (n1.contains('*') || n1.contains('?')) && (n2.contains('*') || n2.contains('?')) {
                return true;
            }
        }
    }

    false
}

fn get_glob_base(p: &str) -> String {
    let mut parts = Vec::new();
    for part in p.split('/') {
        if part.contains('*') || part.contains('?') || part.contains('[') {
            break;
        }
        parts.push(part);
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlap_exact_and_dir() {
        assert!(paths_overlap("src/auth", "src/auth"));
        assert!(paths_overlap("src/auth", "src/auth/login.rs"));
        assert!(paths_overlap("src/auth/login.rs", "src/auth"));
        assert!(!paths_overlap("src/auth", "src/payment"));
    }

    #[test]
    fn test_overlap_globs() {
        assert!(paths_overlap("src/**/*.rs", "src/auth/login.rs"));
        assert!(paths_overlap("src/auth/login.rs", "src/**/*.rs"));
        assert!(paths_overlap("*.json", "package.json"));
        assert!(!paths_overlap("src/*.js", "tests/*.js"));
    }
}
