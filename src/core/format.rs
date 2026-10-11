use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub lines: Vec<String>,
    pub json_val: Option<Value>,
    pub next_hint: Option<String>,
}

impl Default for CommandOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandOutput {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            json_val: None,
            next_hint: None,
        }
    }

    pub fn line(mut self, text: impl Into<String>) -> Self {
        self.lines.push(text.into());
        self
    }

    pub fn lines(mut self, lines: Vec<String>) -> Self {
        self.lines.extend(lines);
        self
    }

    pub fn json<T: Serialize>(mut self, val: &T) -> Self {
        self.json_val = serde_json::to_value(val).ok();
        self
    }

    pub fn next(mut self, hint: impl Into<String>) -> Self {
        self.next_hint = Some(hint.into());
        self
    }

    pub fn print(&self, json_mode: bool, verbose: bool) {
        if json_mode {
            let mut val_obj = match &self.json_val {
                Some(Value::Object(map)) => map.clone(),
                Some(other) => {
                    let mut map = serde_json::Map::new();
                    map.insert("data".to_string(), other.clone());
                    map
                }
                None => {
                    let mut map = serde_json::Map::new();
                    map.insert(
                        "output".to_string(),
                        Value::Array(
                            self.lines
                                .iter()
                                .map(|s| Value::String(s.clone()))
                                .collect(),
                        ),
                    );
                    map
                }
            };
            if let Some(hint) = &self.next_hint {
                val_obj.insert("next".to_string(), Value::String(hint.clone()));
            }
            println!("{}", Value::Object(val_obj));
            return;
        }

        let mut all_lines = self.lines.clone();
        if let Some(hint) = &self.next_hint {
            all_lines.push(format!("next: {}", hint));
        }

        if !verbose && all_lines.len() > 40 {
            for line in &all_lines[..35] {
                println!("{}", line);
            }
            println!(
                "... [truncated {} lines; use --verbose to view all]",
                all_lines.len() - 36
            );
            if let Some(last) = all_lines.last() {
                println!("{}", last);
            }
        } else {
            for line in &all_lines {
                println!("{}", line);
            }
        }
    }
}
