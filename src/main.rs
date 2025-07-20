mod log_parser;
mod ui;

use clap::Parser;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEventKind, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use log_parser::{LogEntry, LogParser};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use std::{
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    time::Duration,
    path::Path,
    fs::File,
};
use futures::StreamExt;
use tokio::{
    sync::mpsc,
    time::interval,
};
use tokio_util::sync::CancellationToken;
use notify::{Watcher, RecursiveMode, RecommendedWatcher, Event as NotifyEvent, EventKind, Config};
use ui::{App, AppMode};
use arboard::Clipboard;
use std::sync::{Arc, Mutex};
use std::process::Command;
use log::{debug, error};


#[derive(Parser)]
#[command(name = "tracing-viewer")]
#[command(about = "A TUI application for filtering and viewing tracing logs")]
struct Cli {
    #[arg(short, long, help = "Input file path (default: stdin)")]
    input: Option<String>,
    
    #[arg(short, long, default_value = "300", help = "Refresh interval in milliseconds")]
    refresh: u64,

    #[arg(long, help = "Enable logging to the specified file")]
    log_file: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(log_path) = &cli.log_file {
        let log_file = std::fs::File::create(log_path)?;
        env_logger::Builder::from_default_env()
            .target(env_logger::Target::Pipe(Box::new(log_file)))
            .init();
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Terminal cleanup on panic or early exit
    let cleanup = || {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
    };
    
    // Register panic hook for proper cleanup
    std::panic::set_hook(Box::new(move |_| {
        cleanup();
    }));

    let (log_sender, mut log_receiver) = mpsc::unbounded_channel();

    let parser = LogParser::new()?;
    let mut app = App::new();
    
    // クリップボードオブジェクトを長期間保持するためのコンテナ
    let clipboard_holder: Arc<Mutex<Option<Clipboard>>> = Arc::new(Mutex::new(None));

    // Create cancellation token for background tasks
    let cancellation_token = CancellationToken::new();
    let mut background_tasks = Vec::new();

    if let Some(input_file) = cli.input {
        // 初期ファイル読み込み
        let file_content = std::fs::read_to_string(&input_file)?;
        let logs = parse_logs_from_content(&parser, &file_content);
        app.update_logs(logs);
        
        // ファイル監視を開始
        let input_file_clone = input_file.clone();
        let log_sender_clone = log_sender.clone();
        let token_clone = cancellation_token.clone();
        let watch_handle = tokio::spawn(async move {
            debug!("watch_file task started");
            if let Err(e) = watch_file(&input_file_clone, log_sender_clone, token_clone).await {
                error!("ファイル監視エラー: {}", e);
            }
            debug!("watch_file task ended");
        });
        background_tasks.push(watch_handle);
        
        // タスクが正常に開始されたことを確認
        debug!("watch_file task spawned successfully");
    } else {
        let token_clone = cancellation_token.clone();
        let stdin_handle = tokio::spawn(async move {
            let stdin = io::stdin();
            let reader = BufReader::new(stdin);
            
            for line in reader.lines() {
                if token_clone.is_cancelled() {
                    break;
                }
                if let Ok(line) = line {
                    if log_sender.send(line).is_err() {
                        break;
                    }
                }
            }
        });
        background_tasks.push(stdin_handle);
    }

    let mut refresh_interval = interval(Duration::from_millis(cli.refresh));
    let mut pending_logs = Vec::new();
    let mut should_redraw = true;
    let mut last_redraw_time = std::time::Instant::now();
    let min_redraw_interval = Duration::from_millis(16); // 約60fps
    let mut event_stream = EventStream::new();

    debug!("メインループ開始前の準備完了");

    // 初期画面を描画
    debug!("初期画面描画開始");
    terminal.draw(|f| ui::render(f, &mut app))?;
    debug!("初期画面描画完了");

    debug!("メインループに入ります");
    let result = async {
        loop {
            tokio::select! {
                _ = refresh_interval.tick() => {
                    if !pending_logs.is_empty() {
                        let logs = parse_logs_from_lines(&parser, &pending_logs);
                        app.add_logs(logs);
                        pending_logs.clear();
                        should_redraw = true;
                    }
                }
                
                log_line = log_receiver.recv() => {
                    if let Some(line) = log_line {
                        pending_logs.push(line);
                    }
                }
                
                event = event_stream.next() => {
                    if let Some(Ok(event)) = event {
                        let needs_redraw = handle_events(&event, &mut app, &clipboard_holder)?;
                        if needs_redraw {
                            should_redraw = true;
                        }
                        
                        // 画面サイズ変更イベントを検出（即座に再描画）
                        if matches!(event, Event::Resize(_, _)) {
                            should_redraw = true;
                            // Resizeイベントは即座に描画する
                            if let Err(e) = terminal.draw(|f| ui::render(f, &mut app)) {
                                error!("Resizeイベントでの描画エラー: {}", e);
                                return Err(e.into());
                            }
                            should_redraw = false;
                            last_redraw_time = std::time::Instant::now();
                        }
                    }
                }
            }

            // 再描画が必要で、かつ最小間隔が経過している場合のみ描画
            if should_redraw && last_redraw_time.elapsed() >= min_redraw_interval {
                if let Err(e) = terminal.draw(|f| ui::render(f, &mut app)) {
                    error!("メインループでの描画エラー: {}", e);
                    return Err(e.into());
                }
                should_redraw = false;
                last_redraw_time = std::time::Instant::now();
            }

            if app.should_quit {
                break;
            }
        }
        Ok::<(), anyhow::Error>(())
    }.await;

    // Cancel all background tasks
    cancellation_token.cancel();
    for task in background_tasks {
        task.abort();
    }

    // Always perform cleanup, regardless of how we exited
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    // Force exit to ensure process terminates
    match result {
        Ok(_) => std::process::exit(0),
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn handle_events(event: &Event, app: &mut App, clipboard_holder: &Arc<Mutex<Option<Clipboard>>>) -> anyhow::Result<bool> {
    match event {
        Event::Key(key) => {
            if key.kind == KeyEventKind::Press {
                match app.mode {
                    AppMode::ModuleSelection => {
                        match key.code {
                            KeyCode::Char('q') => {
                                app.quit();
                            }
                            KeyCode::Char(' ') | KeyCode::Enter => {
                                app.toggle_selected_module();
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                // モジュールリストの最下部にいる場合、ログレベル選択に移動
                                if app.module_list_state.selected().unwrap_or(0) == app.module_items.len().saturating_sub(1) {
                                    app.switch_to_log_level_mode();
                                } else {
                                    app.next_module();
                                }
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                // モジュールリストの最上部にいる場合、ログレベル選択の最下部に移動
                                if app.module_list_state.selected().unwrap_or(0) == 0 {
                                    app.switch_to_log_level_mode();
                                    // ログレベルリストの最下部に移動
                                    if !app.available_log_levels.is_empty() {
                                        app.selected_log_level_index = app.available_log_levels.len() - 1;
                                    }
                                } else {
                                    app.previous_module();
                                }
                            }
                            KeyCode::Tab => {
                                app.switch_to_log_mode();
                            }
                            KeyCode::Char('r') => {
                                app.filter_logs();
                            }
                            KeyCode::Char('1') => {
                                app.toggle_log_level("ERROR");
                            }
                            KeyCode::Char('2') => {
                                app.toggle_log_level("WARN");
                            }
                            KeyCode::Char('3') => {
                                app.toggle_log_level("INFO");
                            }
                            KeyCode::Char('4') => {
                                app.toggle_log_level("DEBUG");
                            }
                            KeyCode::Char('5') => {
                                app.toggle_log_level("TRACE");
                            }
                            KeyCode::Char('a') => {
                                app.select_all_modules();
                            }
                            KeyCode::Char('n') => {
                                app.deselect_all_modules();
                            }
                            KeyCode::Char(',') => {
                                app.decrease_panel_width();
                            }
                            KeyCode::Char('.') => {
                                app.increase_panel_width();
                            }
                            _ => {}
                        }
                    }
                    AppMode::LogNavigation => {
                        match key.code {
                            KeyCode::Char('q') => {
                                app.quit();
                            }
                            KeyCode::Tab => {
                                if app.show_filter_panel {
                                    app.switch_to_module_mode();
                                } else {
                                    app.show_filter_panel = true;
                                    app.switch_to_module_mode();
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                app.next_log_line();
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                app.previous_log_line();
                            }
                            KeyCode::Char('v') => {
                                app.start_text_selection();
                            }
                            KeyCode::Esc => {
                                app.scroll_to_bottom();
                                app.switch_to_module_mode();
                            }
                            KeyCode::Char('c') => {
                                app.clear_copy_message();
                            }
                            KeyCode::Char('1') => {
                                app.toggle_log_level("ERROR");
                            }
                            KeyCode::Char('2') => {
                                app.toggle_log_level("WARN");
                            }
                            KeyCode::Char('3') => {
                                app.toggle_log_level("INFO");
                            }
                            KeyCode::Char('4') => {
                                app.toggle_log_level("DEBUG");
                            }
                            KeyCode::Char('5') => {
                                app.toggle_log_level("TRACE");
                            }
                            _ => {}
                        }
                    }
                    AppMode::TextSelection => {
                        match key.code {
                            KeyCode::Char('q') => {
                                app.quit();
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                app.next_log_line();
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                app.previous_log_line();
                            }
                            KeyCode::Char('y') => {
                                let selected_text = app.copy_selected_logs()?;
                                if !selected_text.is_empty() {
                                    // まずarboardで試行
                                    let mut arboard_success = false;
                                    if let Ok(mut clipboard) = Clipboard::new() {
                                        if clipboard.set_text(&selected_text).is_ok() {
                                            arboard_success = true;
                                            // クリップボードオブジェクトを保持
                                            let holder_clone = clipboard_holder.clone();
                                            let _text_clone = selected_text.clone();
                                            tokio::spawn(async move {
                                                if let Ok(mut holder) = holder_clone.lock() {
                                                    *holder = Some(clipboard);
                                                }
                                                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                                            });
                                        }
                                    }
                                
                                    // arboardが失敗した場合やLinux環境での代替手段
                                    if !arboard_success {
                                        // xclipまたはwl-clipboardを試行
                                        let text_clone = selected_text.clone();
                                        tokio::spawn(async move {
                                            // xclip (X11) を試行
                                            if let Ok(mut child) = Command::new("xclip")
                                                .arg("-selection")
                                                .arg("clipboard")
                                                .stdin(std::process::Stdio::piped())
                                                .spawn() {
                                                if let Some(stdin) = child.stdin.as_mut() {
                                                    use std::io::Write;
                                                    let _ = stdin.write_all(text_clone.as_bytes());
                                                }
                                                let _ = child.wait();
                                            }
                                            // wl-clipboard (Wayland) も試行
                                            else if let Ok(mut child) = Command::new("wl-copy")
                                                .stdin(std::process::Stdio::piped())
                                                .spawn() {
                                                if let Some(stdin) = child.stdin.as_mut() {
                                                    use std::io::Write;
                                                    let _ = stdin.write_all(text_clone.as_bytes());
                                                }
                                                let _ = child.wait();
                                            }
                                        });
                                    }
                                }
                                app.clear_selection();
                            }
                            KeyCode::Esc => {
                                app.clear_selection();
                            }
                            KeyCode::Char('c') => {
                                app.clear_copy_message();
                                return Ok(true);
                            }
                            _ => return Ok(false),
                        }
                    }
                    AppMode::LogLevelFilter => {
                        match key.code {
                            KeyCode::Char('q') => {
                                app.quit();
                            }
                            KeyCode::Tab => {
                                app.switch_to_log_mode();
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                // ログレベルリストの最下部にいる場合、モジュール選択の最上部に移動
                                if app.selected_log_level_index == app.available_log_levels.len().saturating_sub(1) {
                                    app.switch_to_module_mode();
                                    // モジュールリストの最上部に移動
                                    app.module_list_state.select(Some(0));
                                } else {
                                    app.next_log_level();
                                }
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                // ログレベルリストの最上部にいる場合、モジュール選択に移動
                                if app.selected_log_level_index == 0 {
                                    app.switch_to_module_mode();
                                    // モジュールリストの最下部に移動
                                    if !app.module_items.is_empty() {
                                        app.module_list_state.select(Some(app.module_items.len() - 1));
                                    }
                                } else {
                                    app.previous_log_level();
                                }
                            }
                            KeyCode::Char(' ') | KeyCode::Enter => {
                                app.toggle_selected_log_level();
                            }
                            KeyCode::Char('1') => {
                                app.toggle_log_level("ERROR");
                            }
                            KeyCode::Char('2') => {
                                app.toggle_log_level("WARN");
                            }
                            KeyCode::Char('3') => {
                                app.toggle_log_level("INFO");
                            }
                            KeyCode::Char('4') => {
                                app.toggle_log_level("DEBUG");
                            }
                            KeyCode::Char('5') => {
                                app.toggle_log_level("TRACE");
                            }
                            KeyCode::Char(',') => {
                                app.decrease_panel_width();
                            }
                            KeyCode::Char('.') => {
                                app.increase_panel_width();
                            }
                            _ => return Ok(false),
                        }
                    }
                }
            }
            Ok(true)
        }
        Event::Mouse(mouse) => {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    // マウススクロールアップ（上に3行スクロール）
                    for _ in 0..3 {
                        if app.mode == AppMode::LogNavigation || app.mode == AppMode::TextSelection {
                            app.previous_log_line();
                        }
                    }
                    Ok(true)
                }
                MouseEventKind::ScrollDown => {
                    // マウススクロールダウン（下に3行スクロール）
                    for _ in 0..3 {
                        if app.mode == AppMode::LogNavigation || app.mode == AppMode::TextSelection {
                            app.next_log_line();
                        }
                    }
                    Ok(true)
                }
                _ => Ok(false),
            }
        }
        Event::Resize(_width, _height) => {
            // 画面サイズ変更時は常に再描画
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn parse_logs_from_content(parser: &LogParser, content: &str) -> Vec<LogEntry> {
    content
        .lines()
        .filter_map(|line| parser.parse_line(line))
        .collect()
}

fn parse_logs_from_lines(parser: &LogParser, lines: &[String]) -> Vec<LogEntry> {
    lines
        .iter()
        .filter_map(|line| parser.parse_line(line))
        .collect()
}

async fn watch_file(file_path: &str, log_sender: mpsc::UnboundedSender<String>, cancellation_token: CancellationToken) -> anyhow::Result<()> {
    let path = Path::new(file_path);
    if !path.exists() {
        return Err(anyhow::anyhow!("ファイルが存在しません: {}", file_path));
    }

    debug!("ファイル監視を開始: {}", file_path);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = RecommendedWatcher::new(
        move |res: Result<NotifyEvent, notify::Error>| {
            if let Ok(event) = res {
                debug!("ファイルイベント受信: {:?}", event);
                let _ = tx.send(event);
            } else if let Err(e) = res {
                error!("ファイル監視エラー: {}", e);
            }
        },
        Config::default(),
    )?;
    
    watcher.watch(path, RecursiveMode::NonRecursive)?;
    
    let mut file = File::open(path)?;
    let mut last_size = file.metadata()?.len();
    file.seek(SeekFrom::End(0))?;
    debug!("初期ファイルサイズ: {} bytes", last_size);

    loop {
        tokio::select! {
            _ = cancellation_token.cancelled() => {
                debug!("ファイル監視がキャンセルされました");
                break;
            }
            event = rx.recv() => {
                match event {
                    Some(event) => {
                        debug!("イベント処理中: {:?}", event.kind);
                        if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                            let mut file = File::open(path)?;
                            let current_size = file.metadata()?.len();
                            debug!("現在のファイルサイズ: {} bytes (前回: {} bytes)", current_size, last_size);
                            
                            if current_size > last_size {
                                file.seek(SeekFrom::Start(last_size))?;
                                let mut new_content = String::new();
                                file.read_to_string(&mut new_content)?;
                                debug!("新しいコンテンツ読み込み: {} bytes", new_content.len());
                                
                                for line in new_content.lines() {
                                    if !line.trim().is_empty() {
                                        if log_sender.send(line.to_string()).is_err() {
                                            debug!("ログ送信失敗、監視を終了");
                                            return Ok(());
                                        }
                                    }
                                }
                                last_size = current_size;
                            } else if current_size < last_size {
                                // ファイルが縮小された場合（ローテーションなど）
                                debug!("ファイルが縮小されました。リセット中...");
                                last_size = 0;
                                file.seek(SeekFrom::Start(0))?;
                            }
                        }
                    }
                    None => {
                        debug!("ファイル監視チャンネルがクローズされました");
                        break;
                    }
                }
            }
        }
    }
    
    Ok(())
}
