use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ModuleTree {
    pub name: String,
    pub children: HashMap<String, ModuleTree>,
    pub is_selected: bool,
}

impl Hash for ModuleTree {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.is_selected.hash(state);
        
        // HashMapの順序は不定なので、キーをソートしてからハッシュ化
        let mut keys: Vec<_> = self.children.keys().collect();
        keys.sort();
        for key in keys {
            key.hash(state);
            self.children[key].hash(state);
        }
    }
}

impl ModuleTree {
    pub fn new(name: String) -> Self {
        Self {
            name,
            children: HashMap::new(),
            is_selected: true,
        }
    }

    pub fn insert_module(&mut self, module_path: &str) {
        let parts: Vec<&str> = module_path.split("::").collect();
        let mut current = self;

        for part in parts {
            if !current.children.contains_key(part) {
                current.children
                    .entry(part.to_string())
                    .or_insert_with(|| ModuleTree::new(part.to_string()));
            }
            current = current.children.get_mut(part).unwrap();
        }
    }

    pub fn is_module_selected(&self, module_path: &str) -> bool {
        let parts: Vec<&str> = module_path.split("::").collect();
        let mut current = self;

        for part in parts {
            if let Some(child) = current.children.get(part) {
                current = child;
            } else {
                return false;
            }
        }
        current.is_selected
    }

    pub fn toggle_selection(&mut self, module_path: &str) {
        let parts: Vec<&str> = module_path.split("::").collect();
        self.toggle_selection_recursive(&parts, 0);
    }

    fn toggle_selection_recursive(&mut self, parts: &[&str], index: usize) {
        if index >= parts.len() {
            self.is_selected = !self.is_selected;
            self.propagate_selection_to_children(self.is_selected);
            return;
        }

        if let Some(child) = self.children.get_mut(parts[index]) {
            child.toggle_selection_recursive(parts, index + 1);
        }
    }

    fn propagate_selection_to_children(&mut self, selected: bool) {
        self.is_selected = selected;
        for child in self.children.values_mut() {
            child.propagate_selection_to_children(selected);
        }
    }

    pub fn select_all(&mut self) {
        self.propagate_selection_to_children(true);
    }

    pub fn deselect_all(&mut self) {
        self.propagate_selection_to_children(false);
    }
}

pub struct LogParser {
    tracing_regex: Regex,
}

impl LogParser {
    pub fn new() -> anyhow::Result<Self> {
        let tracing_regex = Regex::new(
            r"(?P<timestamp>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z?)\s+(?P<level>\w+)\s+(?P<target>[\w:]+):\s*(?P<message>.*)"
        )?;

        Ok(Self { tracing_regex })
    }

    pub fn parse_line(&self, line: &str) -> Option<LogEntry> {
        if let Some(captures) = self.tracing_regex.captures(line) {
            let timestamp = captures.name("timestamp")?.as_str().to_string();
            let level = captures.name("level")?.as_str().to_string();
            let target = captures.name("target")?.as_str().to_string();
            let message = captures.name("message")?.as_str().to_string();

            Some(LogEntry {
                timestamp,
                level,
                target,
                message,
                fields: HashMap::new(),
            })
        } else {
            None
        }
    }

    pub fn parse_multiline_logs(&self, content: &str) -> Vec<LogEntry> {
        let mut entries = Vec::new();
        let lines: Vec<&str> = content.lines().collect();
        let mut current_entry: Option<LogEntry> = None;
        
        for line in lines {
            if let Some(new_entry) = self.parse_line(line) {
                // 新しいエントリが見つかった場合、前のエントリを保存
                if let Some(entry) = current_entry.take() {
                    entries.push(entry);
                }
                current_entry = Some(new_entry);
            } else if let Some(ref mut entry) = current_entry {
                // 既存のエントリの続きの行として追加
                if !line.trim().is_empty() {
                    entry.message.push('\n');
                    entry.message.push_str(line);
                }
            }
        }
        
        // 最後のエントリを保存
        if let Some(entry) = current_entry {
            entries.push(entry);
        }
        
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tracing_log() {
        let parser = LogParser::new().unwrap();
        let line = "2024-01-01T12:00:00.123Z INFO myapp::module::submodule: This is a test message";
        
        let entry = parser.parse_line(line).unwrap();
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.target, "myapp::module::submodule");
        assert_eq!(entry.message, "This is a test message");
    }

    #[test]
    fn test_module_tree_insertion() {
        let mut tree = ModuleTree::new("root".to_string());
        tree.insert_module("myapp::module::submodule");
        
        assert!(tree.children.contains_key("myapp"));
        assert!(tree.children["myapp"].children.contains_key("module"));
        assert!(tree.children["myapp"].children["module"].children.contains_key("submodule"));
    }

    #[test]
    fn test_module_selection() {
        let mut tree = ModuleTree::new("root".to_string());
        tree.insert_module("myapp::module");
        
        assert!(tree.is_module_selected("myapp::module"));
        tree.toggle_selection("myapp::module");
        assert!(!tree.is_module_selected("myapp::module"));
    }

    #[test]
    fn test_multiline_log_parsing() {
        let parser = LogParser::new().unwrap();
        let content = r#"2024-01-01T12:00:00.123Z INFO myapp::module: First log message
This is a continuation line
And another line
2024-01-01T12:00:01.456Z ERROR myapp::other: Second log message
Error details on next line
    with indented content
2024-01-01T12:00:02.789Z WARN myapp::third: Third message"#;

        let entries = parser.parse_multiline_logs(content);
        assert_eq!(entries.len(), 3);
        
        assert_eq!(entries[0].level, "INFO");
        assert_eq!(entries[0].message, "First log message\nThis is a continuation line\nAnd another line");
        
        assert_eq!(entries[1].level, "ERROR");
        assert_eq!(entries[1].message, "Second log message\nError details on next line\n    with indented content");
        
        assert_eq!(entries[2].level, "WARN");
        assert_eq!(entries[2].message, "Third message");
    }
}