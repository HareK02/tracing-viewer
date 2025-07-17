use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
}