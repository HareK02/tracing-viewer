use crate::log_parser::{LogEntry, ModuleTree};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::hash::{Hash, Hasher};

pub struct App {
    pub module_tree: ModuleTree,
    pub logs: Vec<LogEntry>,
    pub filtered_logs: Vec<LogEntry>,
    pub selected_module_index: usize,
    pub log_scroll_position: usize,
    pub module_list_state: ListState,
    pub module_items: Vec<ModuleItem>,
    pub should_quit: bool,
    pub current_log_line: usize,
    pub selection_start: Option<usize>,
    pub selection_end: Option<usize>,
    pub mode: AppMode,
    pub copy_message: Option<String>,
    pub auto_follow: bool,
    pub filter_dirty: bool,
    pub last_filter_hash: u64,
    pub last_terminal_size: (u16, u16),
    pub log_level_filter: HashSet<String>,
    pub available_log_levels: Vec<String>,
    pub selected_log_level_index: usize,
    pub show_filter_panel: bool,
    pub filter_panel_width: u16,
    pub last_action_was_focus_move: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    ModuleSelection,
    LogNavigation,
    TextSelection,
    LogLevelFilter,
}

#[derive(Debug, Clone)]
pub struct ModuleItem {
    pub name: String,
    pub full_path: String,
    pub level: usize,
    pub is_selected: bool,
}

impl App {
    /// Calculate the number of display lines for a log entry
    fn calculate_display_lines(entry: &LogEntry) -> usize {
        entry.message.lines().count()
    }

    /// Find which entry corresponds to the current scroll position
    fn find_entry_at_scroll_position(&self) -> usize {
        let mut current_line = 0;
        for (entry_idx, entry) in self.filtered_logs.iter().enumerate() {
            let entry_lines = Self::calculate_display_lines(entry);
            if current_line + entry_lines > self.log_scroll_position {
                return entry_idx;
            }
            current_line += entry_lines;
        }
        self.filtered_logs.len().saturating_sub(1)
    }

    /// Calculate the display line position of a specific entry
    fn get_entry_display_position(&self, entry_index: usize) -> usize {
        self.filtered_logs
            .iter()
            .take(entry_index)
            .map(Self::calculate_display_lines)
            .sum()
    }

    /// Ensure the focused entry is visible by adjusting scroll position (for focus movement)
    fn ensure_focus_visible_with_buffer(&mut self, visible_lines: usize) {
        if self.filtered_logs.is_empty() {
            return;
        }

        let focused_entry_start = self.get_entry_display_position(self.current_log_line);
        let focused_entry_end = focused_entry_start + Self::calculate_display_lines(&self.filtered_logs[self.current_log_line]);
        let scroll_end = self.log_scroll_position + visible_lines;
        
        // Calculate display positions for buffer entries
        let prev_entry_start = if self.current_log_line > 0 {
            self.get_entry_display_position(self.current_log_line - 1)
        } else {
            focused_entry_start
        };
        
        let next_entry_end = if self.current_log_line < self.filtered_logs.len() - 1 {
            let next_entry_start = self.get_entry_display_position(self.current_log_line + 1);
            next_entry_start + Self::calculate_display_lines(&self.filtered_logs[self.current_log_line + 1])
        } else {
            focused_entry_end
        };
        
        // If we can't see the previous entry, scroll up to show it
        if prev_entry_start < self.log_scroll_position && self.current_log_line > 0 {
            self.log_scroll_position = prev_entry_start;
        }
        // If we can't see the next entry, scroll down to show it
        else if next_entry_end > scroll_end && self.current_log_line < self.filtered_logs.len() - 1 {
            let total_display_lines = self.filtered_logs
                .iter()
                .map(Self::calculate_display_lines)
                .sum::<usize>();
            let max_scroll = total_display_lines.saturating_sub(visible_lines);
            
            // Position so that the next entry is fully visible at the bottom
            let target_scroll = next_entry_end - visible_lines;
            self.log_scroll_position = target_scroll.min(max_scroll);
        }
    }

    /// Adjust focus when scrolling to keep it on screen (for wheel scroll)
    fn adjust_focus_for_scroll_with_lines(&mut self, visible_lines: usize) {
        if self.filtered_logs.is_empty() {
            return;
        }

        let focused_entry_start = self.get_entry_display_position(self.current_log_line);
        let focused_entry_end = focused_entry_start + Self::calculate_display_lines(&self.filtered_logs[self.current_log_line]);
        let scroll_end = self.log_scroll_position + visible_lines;
        
        // If focus is completely off screen, find the best visible entry
        if focused_entry_end <= self.log_scroll_position || focused_entry_start >= scroll_end {
            // Find the closest visible entry to the current focus
            let mut best_entry = self.current_log_line;
            let mut best_distance = usize::MAX;
            
            for (entry_idx, entry) in self.filtered_logs.iter().enumerate() {
                let entry_start = self.get_entry_display_position(entry_idx);
                let entry_end = entry_start + Self::calculate_display_lines(entry);
                
                // Check if this entry is visible
                if entry_start < scroll_end && entry_end > self.log_scroll_position {
                    let distance = if entry_idx > self.current_log_line {
                        entry_idx - self.current_log_line
                    } else {
                        self.current_log_line - entry_idx
                    };
                    
                    if distance < best_distance {
                        best_distance = distance;
                        best_entry = entry_idx;
                    }
                }
            }
            
            self.current_log_line = best_entry;
        }
    }


    /// Calculate display range of entries that fit within visible lines
    fn calculate_display_range(&self, scroll_position: usize, visible_lines: usize) -> (usize, usize) {
        if self.filtered_logs.is_empty() {
            return (0, 0);
        }

        let mut current_line = 0;
        let mut start_entry = 0;
        let mut end_entry = 0;
        let mut found_start = false;

        for (entry_idx, entry) in self.filtered_logs.iter().enumerate() {
            let entry_lines = Self::calculate_display_lines(entry);
            
            // Find start entry
            if !found_start && current_line + entry_lines > scroll_position {
                start_entry = entry_idx;
                found_start = true;
            }
            
            // Find end entry
            if found_start && current_line >= scroll_position + visible_lines {
                end_entry = entry_idx;
                break;
            }
            
            current_line += entry_lines;
        }
        
        if !found_start {
            start_entry = self.filtered_logs.len().saturating_sub(1);
        }
        if end_entry == 0 {
            end_entry = self.filtered_logs.len();
        }
        
        (start_entry, end_entry)
    }

    pub fn new() -> Self {
        let mut app = Self {
            module_tree: ModuleTree::new("root".to_string()),
            logs: Vec::new(),
            filtered_logs: Vec::new(),
            selected_module_index: 0,
            log_scroll_position: 0,
            module_list_state: ListState::default(),
            module_items: Vec::new(),
            should_quit: false,
            current_log_line: 0,
            selection_start: None,
            selection_end: None,
            mode: AppMode::ModuleSelection,
            copy_message: None,
            auto_follow: true,
            filter_dirty: true,
            last_filter_hash: 0,
            last_terminal_size: (0, 0),
            log_level_filter: ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"].iter().map(|s| s.to_string()).collect(),
            available_log_levels: vec!["ERROR".to_string(), "WARN".to_string(), "INFO".to_string(), "DEBUG".to_string(), "TRACE".to_string()],
            selected_log_level_index: 0,
            show_filter_panel: true,
            filter_panel_width: 25,
            last_action_was_focus_move: false,
        };
        app.module_list_state.select(Some(0));
        app
    }

    pub fn update_logs(&mut self, logs: Vec<LogEntry>) {
        let old_log_count = self.filtered_logs.len();
        self.logs = logs;
        self.rebuild_module_tree();
        self.filter_dirty = true;  // フィルタ再実行を強制
        self.filter_logs();
        
        // 新しいログが追加されたときの自動追従
        if self.auto_follow && self.filtered_logs.len() > old_log_count {
            self.scroll_to_bottom();
        }
    }

    pub fn add_logs(&mut self, new_logs: Vec<LogEntry>) {
        let old_log_count = self.filtered_logs.len();
        let new_log_count = new_logs.len();
        
        if new_log_count == 0 {
            return;
        }
        
        // 新しいログを追加
        self.logs.extend(new_logs);
        
        // 新しいモジュールのみを追加
        for log in &self.logs[(self.logs.len() - new_log_count)..] {
            self.module_tree.insert_module(&log.target);
        }
        
        self.rebuild_module_items();
        
        // 新しいログのみをフィルタリングして効率化
        let new_filtered_logs: Vec<LogEntry> = self.logs[(self.logs.len() - new_log_count)..]
            .iter()
            .filter(|log| {
                self.module_tree.is_module_selected(&log.target) && 
                self.log_level_filter.contains(&log.level)
            })
            .cloned()
            .collect();
        
        self.filtered_logs.extend(new_filtered_logs);
        
        // 新しいログが追加されたときの自動追従
        if self.auto_follow && self.filtered_logs.len() > old_log_count {
            self.scroll_to_bottom();
        }
    }

    fn rebuild_module_tree(&mut self) {
        self.module_tree = ModuleTree::new("root".to_string());
        for log in &self.logs {
            self.module_tree.insert_module(&log.target);
        }
        self.rebuild_module_items();
    }

    fn rebuild_module_items(&mut self) {
        self.module_items.clear();
        let tree_clone = self.module_tree.clone();
        self.build_module_items_recursive(&tree_clone, "", 0);
    }

    fn build_module_items_recursive(&mut self, node: &ModuleTree, path_prefix: &str, level: usize) {
        if level > 0 {
            let full_path = if path_prefix.is_empty() {
                node.name.clone()
            } else {
                format!("{}::{}", path_prefix, node.name)
            };

            let item = ModuleItem {
                name: node.name.clone(),
                full_path: full_path.clone(),
                level,
                is_selected: node.is_selected,
            };
            self.module_items.push(item);

            // 子モジュールをアルファベット順にソートして処理
            let mut sorted_children: Vec<_> = node.children.iter().collect();
            sorted_children.sort_by_key(|(name, _)| name.as_str());
            
            for (_, child) in sorted_children {
                self.build_module_items_recursive(child, &full_path, level + 1);
            }
        } else {
            // ルートレベルでも同様にソート
            let mut sorted_children: Vec<_> = node.children.iter().collect();
            sorted_children.sort_by_key(|(name, _)| name.as_str());
            
            for (_, child) in sorted_children {
                self.build_module_items_recursive(child, path_prefix, level + 1);
            }
        }
    }

    fn calculate_filter_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.module_tree.hash(&mut hasher);
        
        // ログレベルフィルタもハッシュに含める
        let mut levels: Vec<_> = self.log_level_filter.iter().collect();
        levels.sort();
        for level in levels {
            level.hash(&mut hasher);
        }
        
        hasher.finish()
    }

    pub fn filter_logs(&mut self) {
        let current_hash = self.calculate_filter_hash();
        
        // フィルタ条件が変更されていない場合はスキップ
        if !self.filter_dirty && current_hash == self.last_filter_hash {
            return;
        }
        
        self.filtered_logs.clear();
        self.filtered_logs.extend(
            self.logs
                .iter()
                .filter(|log| {
                    self.module_tree.is_module_selected(&log.target) && 
                    self.log_level_filter.contains(&log.level)
                })
                .cloned()
        );
        
        self.filter_dirty = false;
        self.last_filter_hash = current_hash;
    }

    pub fn toggle_selected_module(&mut self) {
        if let Some(selected_index) = self.module_list_state.selected() {
            if selected_index < self.module_items.len() {
                let module_path = self.module_items[selected_index].full_path.clone();
                self.module_tree.toggle_selection(&module_path);
                self.rebuild_module_items();
                self.filter_dirty = true;
                self.filter_logs();
            }
        }
    }

    pub fn next_module(&mut self) {
        if !self.module_items.is_empty() {
            let selected = self.module_list_state.selected().unwrap_or(0);
            let next = (selected + 1) % self.module_items.len();
            self.module_list_state.select(Some(next));
        }
    }

    pub fn previous_module(&mut self) {
        if !self.module_items.is_empty() {
            let selected = self.module_list_state.selected().unwrap_or(0);
            let previous = if selected == 0 {
                self.module_items.len() - 1
            } else {
                selected - 1
            };
            self.module_list_state.select(Some(previous));
        }
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    pub fn switch_to_log_mode(&mut self) {
        self.mode = AppMode::LogNavigation;
        self.show_filter_panel = false;
    }

    pub fn switch_to_module_mode(&mut self) {
        self.mode = AppMode::ModuleSelection;
        self.show_filter_panel = true;
    }

    pub fn start_text_selection(&mut self) {
        self.mode = AppMode::TextSelection;
        self.selection_start = Some(self.current_log_line);
        self.selection_end = Some(self.current_log_line);
    }

    pub fn clear_selection(&mut self) {
        self.selection_start = None;
        self.selection_end = None;
        self.mode = AppMode::LogNavigation;
        self.copy_message = None;
    }

    /// Move focus to next entry and auto-scroll if needed
    pub fn next_log_entry(&mut self) {
        if !self.filtered_logs.is_empty() {
            let old_line = self.current_log_line;
            self.current_log_line = (self.current_log_line + 1).min(self.filtered_logs.len() - 1);
            
            // 最後の行に到達した場合は自動追従を再開
            if self.current_log_line == self.filtered_logs.len() - 1 {
                self.auto_follow = true;
            } else if old_line != self.current_log_line {
                // 手動でナビゲーションした場合は自動追従を停止
                self.auto_follow = false;
            }
            
            // Mark this as a focus movement action
            self.last_action_was_focus_move = true;
            
            if self.mode == AppMode::TextSelection {
                self.selection_end = Some(self.current_log_line);
            }
        }
    }

    /// Move focus to previous entry and auto-scroll if needed
    pub fn previous_log_entry(&mut self) {
        if self.current_log_line > 0 {
            self.current_log_line -= 1;
            // 手動で上にナビゲーションした場合は自動追従を停止
            self.auto_follow = false;
            
            // Mark this as a focus movement action
            self.last_action_was_focus_move = true;
            
            if self.mode == AppMode::TextSelection {
                self.selection_end = Some(self.current_log_line);
            }
        }
    }

    /// Scroll display area without changing focus
    pub fn scroll_down(&mut self, lines: usize) {
        if !self.filtered_logs.is_empty() {
            let total_display_lines = self.filtered_logs
                .iter()
                .map(Self::calculate_display_lines)
                .sum::<usize>();
            
            let max_scroll = total_display_lines.saturating_sub(1);
            self.log_scroll_position = (self.log_scroll_position + lines).min(max_scroll);
            self.auto_follow = false;
            
            // Mark this as a scroll action
            self.last_action_was_focus_move = false;
        }
    }

    /// Scroll display area without changing focus
    pub fn scroll_up(&mut self, lines: usize) {
        self.log_scroll_position = self.log_scroll_position.saturating_sub(lines);
        self.auto_follow = false;
        
        // Mark this as a scroll action
        self.last_action_was_focus_move = false;
    }


    pub fn page_up(&mut self, visible_lines: usize) {
        let scroll_amount = visible_lines.saturating_sub(1).max(1);
        self.log_scroll_position = self.log_scroll_position.saturating_sub(scroll_amount);
        self.auto_follow = false;
        
        // Mark this as a scroll action
        self.last_action_was_focus_move = false;
        
        if self.mode == AppMode::TextSelection {
            self.selection_end = Some(self.current_log_line);
        }
    }

    pub fn page_down(&mut self, visible_lines: usize) {
        let total_display_lines = self.filtered_logs
            .iter()
            .map(Self::calculate_display_lines)
            .sum::<usize>();
        
        let scroll_amount = visible_lines.saturating_sub(1).max(1);
        let max_scroll = total_display_lines.saturating_sub(visible_lines.min(total_display_lines));
        
        self.log_scroll_position = (self.log_scroll_position + scroll_amount).min(max_scroll);
        self.auto_follow = false;
        
        // Mark this as a scroll action
        self.last_action_was_focus_move = false;
        
        if self.mode == AppMode::TextSelection {
            self.selection_end = Some(self.current_log_line);
        }
    }

    pub fn copy_selected_logs(&mut self) -> anyhow::Result<String> {
        if let (Some(selection_start), Some(selection_end)) = (self.selection_start, self.selection_end) {
            let start = selection_start.min(selection_end);
            let end = selection_start.max(selection_end);
            
            let selected_logs: Vec<String> = self.filtered_logs
                .iter()
                .skip(start)
                .take(end - start + 1)
                .map(|log| format!("[{}] {} {}: {}", log.timestamp, log.level, log.target, log.message))
                .collect();
            
            let content = selected_logs.join("\n");
            let lines_count = end - start + 1;
            self.copy_message = Some(format!("Copied {} lines to clipboard", lines_count));
            Ok(content)
        } else {
            Ok(String::new())
        }
    }

    pub fn clear_copy_message(&mut self) {
        self.copy_message = None;
    }

    pub fn scroll_to_bottom(&mut self) {
        if !self.filtered_logs.is_empty() {
            self.current_log_line = self.filtered_logs.len() - 1;
            // 最後のエントリの最後の表示行にスクロール
            let total_display_lines = self.filtered_logs
                .iter()
                .map(Self::calculate_display_lines)
                .sum::<usize>();
            self.log_scroll_position = total_display_lines.saturating_sub(1);
            self.auto_follow = true;
        }
    }

    pub fn toggle_log_level(&mut self, level: &str) {
        if self.log_level_filter.contains(level) {
            self.log_level_filter.remove(level);
        } else {
            self.log_level_filter.insert(level.to_string());
        }
        self.filter_dirty = true;
        self.filter_logs();
    }

    pub fn switch_to_log_level_mode(&mut self) {
        self.mode = AppMode::LogLevelFilter;
        self.show_filter_panel = true;
    }

    pub fn select_all_modules(&mut self) {
        self.module_tree.select_all();
        self.rebuild_module_items();
        self.filter_dirty = true;
        self.filter_logs();
    }

    pub fn deselect_all_modules(&mut self) {
        self.module_tree.deselect_all();
        self.rebuild_module_items();
        self.filter_dirty = true;
        self.filter_logs();
    }

    pub fn next_log_level(&mut self) {
        if !self.available_log_levels.is_empty() {
            self.selected_log_level_index = (self.selected_log_level_index + 1) % self.available_log_levels.len();
        }
    }

    pub fn previous_log_level(&mut self) {
        if !self.available_log_levels.is_empty() {
            self.selected_log_level_index = if self.selected_log_level_index == 0 {
                self.available_log_levels.len() - 1
            } else {
                self.selected_log_level_index - 1
            };
        }
    }

    pub fn toggle_selected_log_level(&mut self) {
        if self.selected_log_level_index < self.available_log_levels.len() {
            let level = self.available_log_levels[self.selected_log_level_index].clone();
            self.toggle_log_level(&level);
        }
    }

    pub fn decrease_panel_width(&mut self) {
        if self.filter_panel_width > 10 {
            self.filter_panel_width -= 5;
        }
    }

    pub fn increase_panel_width(&mut self) {
        if self.filter_panel_width < 50 {
            self.filter_panel_width += 5;
        }
    }
    pub fn update_scroll_position_with_height(&mut self, visible_lines: usize) {
        if visible_lines <= 2 || self.filtered_logs.is_empty() {
            return;
        }
        
        // 総表示行数を計算
        let total_display_lines = self.filtered_logs
            .iter()
            .map(Self::calculate_display_lines)
            .sum::<usize>();
        
        // 表示可能な最大スクロール位置を計算
        let max_scroll = total_display_lines.saturating_sub(visible_lines);
        
        // スクロール位置が範囲内に収まるように制限
        self.log_scroll_position = self.log_scroll_position.min(max_scroll);
        
        // Update helper methods with correct visible lines
        self.update_focus_visibility_with_lines(visible_lines);
    }

    /// Update focus visibility calculations with actual visible lines
    fn update_focus_visibility_with_lines(&mut self, visible_lines: usize) {
        if self.filtered_logs.is_empty() {
            return;
        }

        // Apply different logic based on last action
        if self.last_action_was_focus_move && (self.mode == AppMode::LogNavigation || self.mode == AppMode::TextSelection) {
            // Focus movement: adjust scroll to ensure buffer visibility
            self.ensure_focus_visible_with_buffer(visible_lines);
        } else {
            // Scroll action: adjust focus to stay on screen
            self.adjust_focus_for_scroll_with_lines(visible_lines);
        }
    }
}

pub fn render(f: &mut Frame, app: &mut App) {
    let current_size = (f.area().width, f.area().height);
    
    // 画面サイズが変更された場合の処理
    if app.last_terminal_size != current_size {
        app.last_terminal_size = current_size;
        // スクロール位置を調整（画面サイズに合わせて）
        if app.auto_follow && !app.filtered_logs.is_empty() {
            app.scroll_to_bottom();
        }
    }
    
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(f.area());

    if app.show_filter_panel {
        let remaining_width = 100 - app.filter_panel_width;
        let top_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(app.filter_panel_width),  // 左パネル（ログレベル+モジュール）
                Constraint::Length(1),       // 区切り線
                Constraint::Percentage(remaining_width.saturating_sub(1)),  // ログエリア
            ])
            .split(main_chunks[0]);

        render_left_panel(f, app, top_chunks[0]);
        render_separator(f, top_chunks[1]);
        render_logs(f, app, top_chunks[2]);
    } else {
        render_logs(f, app, main_chunks[0]);
    }
    render_status_bar(f, app, main_chunks[1]);
}

fn render_left_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),  // ログレベルフィルター用
            Constraint::Length(1),  // 区切り線用
            Constraint::Min(0),     // モジュールツリー用
        ])
        .split(area);

    render_log_level_filter(f, app, left_chunks[0]);
    render_horizontal_separator(f, left_chunks[1]);
    render_module_tree(f, app, left_chunks[2]);
}


fn render_module_tree(f: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app.module_items
        .iter()
        .enumerate()
        .map(|(_index, item)| {
            let indent = "  ".repeat(item.level.saturating_sub(1));
            let checkbox = if item.is_selected { "☑" } else { "☐" };
            
            // フォーカスがある場合は矢印分を空けておく、ない場合は直接スペースを追加
            let prefix = if app.mode == AppMode::ModuleSelection {
                // フォーカスがある場合、選択行のみハイライトシンボルで矢印が表示される
                ""
            } else {
                // フォーカスがない場合、すべての行に矢印分のスペースを追加
                "  "
            };
            
            let content = format!("{}{}{} {}", prefix, indent, checkbox, item.name);
            
            let style = if item.is_selected {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Gray)
            };

            ListItem::new(Line::from(Span::styled(content, style)))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .highlight_symbol("→ ");

    // モジュール選択モードの場合のみフォーカス表示
    let mut list_state = ListState::default();
    if app.mode == AppMode::ModuleSelection {
        list_state.select(app.module_list_state.selected());
    }

    f.render_stateful_widget(list, area, &mut list_state);
}

fn render_separator(f: &mut Frame, area: Rect) {
    let separator_text: Vec<Line> = (0..area.height)
        .map(|_| Line::from("│"))
        .collect();
    
    let separator = Paragraph::new(separator_text)
        .style(Style::default().fg(Color::DarkGray));
    
    f.render_widget(separator, area);
}

fn render_horizontal_separator(f: &mut Frame, area: Rect) {
    let separator_text = "─".repeat(area.width as usize);
    let separator = Paragraph::new(separator_text)
        .style(Style::default().fg(Color::DarkGray));
    
    f.render_widget(separator, area);
}

fn render_logs(f: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);
    let log_area = chunks[0];
    let pagination_area = chunks[1];

    // 実際の表示可能行数でスクロール位置を更新
    let visible_lines = log_area.height as usize;
    app.update_scroll_position_with_height(visible_lines);
    
    // ログが空の場合は早期リターン
    if app.filtered_logs.is_empty() {
        f.render_widget(Paragraph::new("No logs to display"), log_area);
        return;
    }
    
    // 表示範囲を実際の表示行数に基づいて計算
    let (start_entry, end_entry) = app.calculate_display_range(app.log_scroll_position, visible_lines);
    
    // 安全性を確保するため、インデックスの妥当性を再チェック
    let start_index = start_entry.min(app.filtered_logs.len());
    let end_index = end_entry.min(app.filtered_logs.len()).max(start_index);
    
    // スクロール位置から開始エントリまでの表示行数を計算
    let skip_lines = app.filtered_logs
        .iter()
        .take(start_index)
        .map(App::calculate_display_lines)
        .sum::<usize>();
    
    let scroll_offset = app.log_scroll_position.saturating_sub(skip_lines);

    let log_content: Vec<Line> = app.filtered_logs
        .iter()
        .skip(start_index)
        .take(end_index - start_index)
        .enumerate()
        .flat_map(|(relative_index, log)| {
            let index = start_index + relative_index;
            let level_style = match log.level.as_str() {
                "ERROR" => Style::default().fg(Color::Red),
                "WARN" => Style::default().fg(Color::Yellow),
                "INFO" => Style::default().fg(Color::Green),
                "DEBUG" => Style::default().fg(Color::Blue),
                "TRACE" => Style::default().fg(Color::Magenta),
                _ => Style::default().fg(Color::White),
            };

            let is_selected = app.selection_start.is_some() && app.selection_end.is_some() && {
                let start = app.selection_start.unwrap().min(app.selection_end.unwrap());
                let end = app.selection_start.unwrap().max(app.selection_end.unwrap());
                index >= start && index <= end
            };

            let is_current = index == app.current_log_line && 
                (app.mode == AppMode::LogNavigation || app.mode == AppMode::TextSelection);

            let mut base_style = Style::default();
            if is_selected {
                base_style = base_style.bg(Color::DarkGray);
            }
            if is_current {
                // Make focus more prominent with bright background and bold text
                base_style = base_style.bg(Color::Blue).add_modifier(Modifier::BOLD);
            }

            let message_lines: Vec<&str> = log.message.lines().collect();
            let mut lines = Vec::new();
            
            for (line_index, message_line) in message_lines.iter().enumerate() {
                if line_index == 0 {
                    // First line includes timestamp, level, and target
                    lines.push(Line::from(vec![
                        Span::styled(format!("[{}] ", log.timestamp), base_style.fg(Color::Cyan)),
                        Span::styled(format!("{:<5} ", log.level), base_style.patch(level_style)),
                        Span::styled(format!("{}: ", log.target), base_style.fg(Color::Yellow)),
                        Span::styled(*message_line, base_style),
                    ]));
                } else {
                    // Continuation lines are indented
                    lines.push(Line::from(vec![
                        Span::styled("    ", base_style), // Indentation for continuation
                        Span::styled(*message_line, base_style),
                    ]));
                }
            }
            
            lines
        })
        .collect();

    let paragraph = Paragraph::new(log_content)
        .scroll((scroll_offset as u16, 0));

    f.render_widget(paragraph, log_area);

    if !app.filtered_logs.is_empty() {
        let total_display_lines = app.filtered_logs
            .iter()
            .map(App::calculate_display_lines)
            .sum::<usize>();
        let start_line = app.log_scroll_position + 1;
        let end_line = (app.log_scroll_position + visible_lines).min(total_display_lines);
        
        let pagination_text = format!("{}-{} of {} lines ({} entries)", 
            start_line, end_line, total_display_lines, app.filtered_logs.len());
        let pagination_paragraph = Paragraph::new(pagination_text)
            .style(Style::default().fg(Color::DarkGray));
            
        f.render_widget(pagination_paragraph, pagination_area);
    }
}

fn render_log_level_filter(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app.available_log_levels
        .iter()
        .enumerate()
        .map(|(_index, level)| {
            let checkbox = if app.log_level_filter.contains(level) { "☑" } else { "☐" };
            
            // フォーカスがある場合は矢印分を空けておく、ない場合は直接スペースを追加
            let prefix = if app.mode == AppMode::LogLevelFilter {
                // フォーカスがある場合、選択行のみハイライトシンボルで矢印が表示される
                ""
            } else {
                // フォーカスがない場合、すべての行に矢印分のスペースを追加
                "  "
            };
            
            let content = format!("{}{} {}", prefix, checkbox, level);
            
            let style = match level.as_str() {
                "ERROR" => Style::default().fg(Color::Red),
                "WARN" => Style::default().fg(Color::Yellow),
                "INFO" => Style::default().fg(Color::Green),
                "DEBUG" => Style::default().fg(Color::Blue),
                "TRACE" => Style::default().fg(Color::Magenta),
                _ => Style::default().fg(Color::White),
            };

            let final_style = if !app.log_level_filter.contains(level) {
                style.fg(Color::DarkGray)
            } else {
                style
            };

            ListItem::new(Line::from(Span::styled(content, final_style)))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .highlight_symbol("→ ");

    // ログレベル選択状態を管理
    let mut list_state = ListState::default();
    if app.mode == AppMode::LogLevelFilter {
        list_state.select(Some(app.selected_log_level_index));
    }

    f.render_stateful_widget(list, area, &mut list_state);
}

fn create_colored_help_line(parts: Vec<(&str, &str)>) -> Line<'static> {
    let mut spans = Vec::new();
    
    for (i, (key, desc)) in parts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(", ", Style::default().fg(Color::DarkGray)));
        }
        spans.push(Span::styled((*key).to_string(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(": ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled((*desc).to_string(), Style::default().fg(Color::White)));
    }
    
    Line::from(spans)
}

fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    if let Some(ref message) = app.copy_message {
        let status_paragraph = Paragraph::new(message.clone())
            .style(Style::default().fg(Color::Green));
        f.render_widget(status_paragraph, area);
        return;
    }

    let help_line = match app.mode {
        AppMode::ModuleSelection => {
            let mut parts = vec![
                ("↑↓/jk", "Navigate"),
                ("Space", "Toggle"),
                ("a", "All"),
                ("n", "None"),
            ];
            if app.show_filter_panel {
                parts.extend_from_slice(&[(",/.", "Resize panel")]);
            }
            parts.extend_from_slice(&[("Tab", "Logs"), ("q", "Quit")]);
            
            let mut spans = vec![
                Span::styled("Module Selection: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            ];
            spans.extend(create_colored_help_line(parts).spans);
            Line::from(spans)
        },
        AppMode::LogNavigation => {
            let mut parts = vec![
                ("↑↓/jk", "Move focus"),
                ("Wheel", "Scroll view"),
                ("PgUp/PgDn", "Page scroll"),
                ("v", "Select text"),
            ];
            if app.show_filter_panel {
                parts.push(("Tab", "Modules"));
            } else {
                parts.push(("Tab", "Show filters"));
            }
            parts.push(("q", "Quit"));
            
            let mut spans = vec![
                Span::styled("Log Navigation: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            ];
            spans.extend(create_colored_help_line(parts).spans);
            Line::from(spans)
        },
        AppMode::TextSelection => {
            let parts = vec![
                ("↑↓/jk", "Extend selection"),
                ("Wheel", "Scroll view"),
                ("PgUp/PgDn", "Page scroll"),
                ("y", "Copy"),
                ("Esc", "Cancel"),
            ];
            
            let mut spans = vec![
                Span::styled("Text Selection: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            ];
            spans.extend(create_colored_help_line(parts).spans);
            Line::from(spans)
        },
        AppMode::LogLevelFilter => {
            let mut parts = vec![
                ("↑↓/jk", "Navigate"),
                ("Space", "Toggle level"),
                ("1-5", "Quick toggle"),
            ];
            if app.show_filter_panel {
                parts.push((",/.", "Resize panel"));
            }
            parts.extend_from_slice(&[("Tab", "Logs"), ("q", "Quit")]);
            
            let mut spans = vec![
                Span::styled("Log Level Filter: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            ];
            spans.extend(create_colored_help_line(parts).spans);
            Line::from(spans)
        },
    };

    let status_paragraph = Paragraph::new(help_line);
    f.render_widget(status_paragraph, area);
}