mod adb;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, DisableLineWrap, Clear, ClearType},
    ExecutableCommand,
};
use ratatui::{prelude::*, widgets::*};
use std::io::stdout;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

enum AppScreen {
    Devices,
    Commands,
    Files,
    RunBinary,
    Logs,
}

struct App {
    devices: Vec<adb::Device>,
    selected_device: Option<usize>,
    selected_devices: Vec<usize>,
    screen: AppScreen,
    command_input: String,
    command_output: Vec<String>,
    command_history: Vec<String>,
    history_index: Option<usize>,
    file_remote_path: String,
    file_local_path: String,
    binary_path: String,
    binary_args: String,
    status_message: String,
    logs: Vec<String>,
    should_quit: bool,
    device_rx: Receiver<Vec<adb::Device>>,
}

impl App {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();

        // Spawn background thread to listen for ADB device changes
        thread::spawn(move || {
            adb::track_devices(tx);
        });

        Self {
            devices: Vec::new(),
            selected_device: None,
            selected_devices: Vec::new(),
            screen: AppScreen::Devices,
            command_input: String::new(),
            command_output: Vec::new(),
            command_history: Vec::new(),
            history_index: None,
            file_remote_path: String::new(),
            file_local_path: String::new(),
            binary_path: String::new(),
            binary_args: String::new(),
            status_message: "Press 'q' to quit, 'r' to refresh devices".to_string(),
            logs: Vec::new(),
            should_quit: false,
            device_rx: rx,
        }
    }

    fn add_log(&mut self, message: String) {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        self.logs.push(format!("[{}] {}", timestamp, message));
        if self.logs.len() > 1000 {
            self.logs.remove(0);
        }
    }

    fn refresh_devices(&mut self) {
        self.add_log("Refreshing devices...".to_string());

        match adb::get_devices() {
            Ok(devices) => {
                let count = devices.len();
                self.add_log(format!("Found {} device(s)", count));

                for device in &devices {
                    self.add_log(format!("  - {} ({}) - status: {}",
                        device.id,
                        device.model.as_deref().unwrap_or("Unknown"),
                        device.status));
                }

                self.devices = devices;
                self.status_message = format!("Found {} device(s)", count);
            }
            Err(e) => {
                self.add_log(format!("Error getting devices: {}", e));
                self.status_message = format!("Error: {}", e);
            }
        }
    }

    fn update_devices(&mut self, new_devices: Vec<adb::Device>) {
        let old_count = self.devices.len();
        let new_count = new_devices.len();

        if old_count != new_count {
            self.add_log(format!("Device list changed: {} -> {} device(s)", old_count, new_count));
        }

        self.devices = new_devices;

        // Update selected_device if needed
        if let Some(idx) = self.selected_device {
            if idx >= self.devices.len() {
                self.selected_device = if self.devices.is_empty() { None } else { Some(0) };
            }
        }

        // Remove invalid multi-selections
        self.selected_devices.retain(|&idx| idx < self.devices.len());

        if old_count != new_count {
            self.status_message = format!("Device list updated: {} device(s)", new_count);
        }
    }

    fn toggle_device_selection(&mut self, index: usize) {
        if let Some(pos) = self.selected_devices.iter().position(|&i| i == index) {
            self.selected_devices.remove(pos);
        } else {
            self.selected_devices.push(index);
        }
    }

    fn execute_command(&mut self) {
        if self.command_input.is_empty() {
            return;
        }

        let devices: Vec<String> = if self.selected_devices.is_empty() {
            if let Some(idx) = self.selected_device {
                vec![self.devices[idx].id.clone()]
            } else {
                self.status_message = "No device selected".to_string();
                return;
            }
        } else {
            self.selected_devices.iter().map(|&i| self.devices[i].id.clone()).collect()
        };

        self.command_output.clear();
        self.add_log(format!("Executing command: {} on {} device(s)", self.command_input, devices.len()));
        self.command_output.push(format!("Executing: {} on {} device(s)", self.command_input, devices.len()));

        let results = adb::batch_execute(&devices, &self.command_input);
        for (device_id, result) in results {
            self.add_log(format!("Running on device: {}", device_id));
            match result {
                Ok((stdout, stderr)) => {
                    self.command_output.push(format!("=== Device: {} ===", device_id));
                    if !stdout.is_empty() {
                        self.command_output.push(stdout);
                    }
                    if !stderr.is_empty() {
                        self.command_output.push(format!("STDERR: {}", stderr));
                    }
                    self.add_log(format!("Command completed on device: {}", device_id));
                }
                Err(e) => {
                    self.command_output.push(format!("=== Device: {} - Error: {} ===", device_id, e));
                    self.add_log(format!("Error on device {}: {}", device_id, e));
                }
            }
        }

        // Add to history (avoid duplicates of consecutive commands)
        if self.command_history.last() != Some(&self.command_input) {
            self.command_history.push(self.command_input.clone());
        }
        self.history_index = None;
        self.command_input.clear();
    }

    fn pull_file(&mut self) {
        if self.file_remote_path.is_empty() || self.file_local_path.is_empty() {
            self.status_message = "Please specify both remote and local paths".to_string();
            return;
        }

        let devices: Vec<String> = if self.selected_devices.is_empty() {
            if let Some(idx) = self.selected_device {
                vec![self.devices[idx].id.clone()]
            } else {
                self.status_message = "No device selected".to_string();
                return;
            }
        } else {
            self.selected_devices.iter().map(|&i| self.devices[i].id.clone()).collect()
        };

        self.add_log(format!("Pulling {} to {} from {} device(s)", 
            self.file_remote_path, self.file_local_path, devices.len()));
        
        let results = adb::batch_pull(&devices, &self.file_remote_path, &self.file_local_path);
        for (device_id, result) in results {
            match result {
                Ok(()) => {
                    self.add_log(format!("Successfully pulled from device {}", device_id));
                    self.status_message = format!("Pulled from device {} successfully", device_id);
                }
                Err(e) => {
                    self.add_log(format!("Failed to pull from device {}: {}", device_id, e));
                    self.status_message = format!("Failed to pull from device {}: {}", device_id, e);
                }
            }
        }
    }

    fn push_file(&mut self) {
        if self.file_remote_path.is_empty() || self.file_local_path.is_empty() {
            self.status_message = "Please specify both local and remote paths".to_string();
            return;
        }

        let devices: Vec<String> = if self.selected_devices.is_empty() {
            if let Some(idx) = self.selected_device {
                vec![self.devices[idx].id.clone()]
            } else {
                self.status_message = "No device selected".to_string();
                return;
            }
        } else {
            self.selected_devices.iter().map(|&i| self.devices[i].id.clone()).collect()
        };

        self.add_log(format!("Pushing {} to {} on {} device(s)", 
            self.file_local_path, self.file_remote_path, devices.len()));
        
        let results = adb::batch_push(&devices, &self.file_local_path, &self.file_remote_path);
        for (device_id, result) in results {
            match result {
                Ok(()) => {
                    self.add_log(format!("Successfully pushed to device {}", device_id));
                    self.status_message = format!("Pushed to device {} successfully", device_id);
                }
                Err(e) => {
                    self.add_log(format!("Failed to push to device {}: {}", device_id, e));
                    self.status_message = format!("Failed to push to device {}: {}", device_id, e);
                }
            }
        }
    }

    fn run_binary(&mut self) {
        if self.binary_path.is_empty() {
            self.status_message = "Please specify binary path".to_string();
            return;
        }

        let device_id = if let Some(idx) = self.selected_device {
            self.devices[idx].id.clone()
        } else {
            self.status_message = "No device selected".to_string();
            return;
        };

        self.add_log(format!("Running binary {} on device {} with args: {}", 
            self.binary_path, device_id, self.binary_args));
        
        let args: Vec<&str> = self.binary_args.split_whitespace().collect();
        match adb::run_binary(&device_id, &self.binary_path, &args) {
            Ok((stdout, stderr)) => {
                self.command_output.clear();
                self.command_output.push(format!("=== Running: {} ===", self.binary_path));
                if !stdout.is_empty() {
                    self.command_output.push(stdout.clone());
                    self.add_log(format!("Output: {}", stdout));
                }
                if !stderr.is_empty() {
                    self.command_output.push(format!("STDERR: {}", stderr));
                    self.add_log(format!("STDERR: {}", stderr));
                }
                self.add_log(format!("Binary execution completed on device {}", device_id));
            }
            Err(e) => {
                self.add_log(format!("Failed to run binary on device {}: {}", device_id, e));
                self.status_message = format!("Failed to run binary: {}", e);
            }
        }
    }
}

fn main() -> Result<()> {
    // Check if --check flag is provided
    if std::env::args().any(|arg| arg == "--check") {
        return check_adb();
    }
    
    enable_raw_mode()?;
    stdout().execute(DisableLineWrap)?;
    stdout().execute(Clear(ClearType::All))?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut app = App::new();
    app.refresh_devices();

    let result = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    result
}

fn check_adb() -> Result<()> {
    match adb::find_adb_path() {
        Ok(path) => {
            println!("ADB found at: {}", path);
            match adb::get_devices() {
                Ok(devices) => {
                    println!("Found {} device(s):", devices.len());
                    for d in &devices {
                        println!("  - {} ({}) - {}", d.id, d.model.as_deref().unwrap_or("Unknown"), d.status);
                    }
                }
                Err(e) => println!("Error getting devices: {}", e),
            }
        }
        Err(e) => {
            println!("ADB not found: {}", e);
            return Err(e);
        }
    }
    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_key_event(key, app);
                }
            }
        }

        // Check for device updates from background thread
        while let Ok(new_devices) = app.device_rx.try_recv() {
            app.update_devices(new_devices);
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn handle_key_event(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::ALT) => {
            if matches!(app.screen, AppScreen::Devices) {
                app.should_quit = true;
            } else {
                app.screen = AppScreen::Devices;
            }
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::ALT) => {
            if matches!(app.screen, AppScreen::Devices) {
                app.refresh_devices();
            }
        }
        KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::ALT) => app.screen = AppScreen::Devices,
        KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::ALT) => app.screen = AppScreen::Commands,
        KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::ALT) => app.screen = AppScreen::Files,
        KeyCode::Char('4') if key.modifiers.contains(KeyModifiers::ALT) => app.screen = AppScreen::RunBinary,
        KeyCode::Char('5') if key.modifiers.contains(KeyModifiers::ALT) => app.screen = AppScreen::Logs,
        _ => handle_screen_keys(key, app),
    }
}

fn handle_screen_keys(key: KeyEvent, app: &mut App) {
    match app.screen {
        AppScreen::Devices => match key.code {
            KeyCode::Up if !key.modifiers.contains(KeyModifiers::ALT) => {
                if let Some(idx) = app.selected_device {
                    if idx > 0 {
                        app.selected_device = Some(idx - 1);
                    }
                } else if !app.devices.is_empty() {
                    app.selected_device = Some(0);
                }
            }
            KeyCode::Down if !key.modifiers.contains(KeyModifiers::ALT) => {
                if let Some(idx) = app.selected_device {
                    if idx < app.devices.len() - 1 {
                        app.selected_device = Some(idx + 1);
                    }
                } else if !app.devices.is_empty() {
                    app.selected_device = Some(0);
                }
            }
            KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::ALT) => {
                if let Some(idx) = app.selected_device {
                    app.toggle_device_selection(idx);
                }
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => {
                if let Some(idx) = app.selected_device {
                    if idx > 0 {
                        app.selected_device = Some(idx - 1);
                    }
                } else if !app.devices.is_empty() {
                    app.selected_device = Some(0);
                }
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => {
                if let Some(idx) = app.selected_device {
                    if idx < app.devices.len() - 1 {
                        app.selected_device = Some(idx + 1);
                    }
                } else if !app.devices.is_empty() {
                    app.selected_device = Some(0);
                }
            }
            _ => {}
        },
        AppScreen::Commands => match key.code {
            KeyCode::Enter => app.execute_command(),
            KeyCode::Char(c) => {
                app.history_index = None;
                app.command_input.push(c);
            }
            KeyCode::Backspace => {
                app.history_index = None;
                app.command_input.pop();
            }
            KeyCode::Up => {
                if !app.command_history.is_empty() {
                    let new_index = match app.history_index {
                        None => app.command_history.len().saturating_sub(1),
                        Some(0) => 0,
                        Some(idx) => idx - 1,
                    };
                    app.history_index = Some(new_index);
                    app.command_input = app.command_history[new_index].clone();
                }
            }
            KeyCode::Down => {
                match app.history_index {
                    Some(idx) if idx + 1 < app.command_history.len() => {
                        app.history_index = Some(idx + 1);
                        app.command_input = app.command_history[idx + 1].clone();
                    }
                    Some(_) => {
                        app.history_index = None;
                        app.command_input.clear();
                    }
                    None => {}
                }
            }
            KeyCode::Left => {
                // No-op for now, could add cursor movement later
            }
            KeyCode::Right => {
                // No-op for now, could add cursor movement later
            }
            _ => {}
        },
        AppScreen::Files => match key.code {
            KeyCode::Enter => {
                if app.file_remote_path.is_empty() || app.file_local_path.is_empty() {
                    app.status_message = "Press Alt+p to push or Alt+l to pull".to_string();
                }
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::ALT) => app.push_file(),
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::ALT) => app.pull_file(),
            KeyCode::Char(c) => {
                if app.file_remote_path.is_empty() || app.file_local_path.is_empty() {
                    // Determine which field to edit based on cursor position
                } else {
                    app.file_remote_path.push(c);
                }
            }
            KeyCode::Backspace => {
                if !app.file_remote_path.is_empty() {
                    app.file_remote_path.pop();
                } else if !app.file_local_path.is_empty() {
                    app.file_local_path.pop();
                }
            }
            _ => {}
        },
        AppScreen::RunBinary => match key.code {
            KeyCode::Enter => app.run_binary(),
            KeyCode::Char(c) => {
                if app.binary_path.is_empty() || app.binary_args.is_empty() {
                    app.binary_path.push(c);
                } else {
                    app.binary_args.push(c);
                }
            }
            KeyCode::Backspace => {
                if !app.binary_args.is_empty() {
                    app.binary_args.pop();
                } else if !app.binary_path.is_empty() {
                    app.binary_path.pop();
                }
            }
            _ => {}
        },
        AppScreen::Logs => match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::ALT) => app.logs.clear(),
            _ => {}
        },
    }
}

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.size());

    let tabs = Tabs::new(vec!["Devices [1]", "Commands [2]", "Files [3]", "Run Binary [4]", "Logs [5]"])
        .select(match app.screen {
            AppScreen::Devices => 0,
            AppScreen::Commands => 1,
            AppScreen::Files => 2,
            AppScreen::RunBinary => 3,
            AppScreen::Logs => 4,
        })
        .block(Block::default().borders(Borders::ALL).title("ADB TUI Manager"))
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(tabs, chunks[0]);

    match app.screen {
        AppScreen::Devices => render_devices(f, chunks[1], app),
        AppScreen::Commands => render_commands(f, chunks[1], app),
        AppScreen::Files => render_files(f, chunks[1], app),
        AppScreen::RunBinary => render_run_binary(f, chunks[1], app),
        AppScreen::Logs => render_logs(f, chunks[1], app),
    }

    let status = Paragraph::new(app.status_message.clone())
        .block(Block::default().borders(Borders::ALL).title("Status"));
    f.render_widget(status, chunks[2]);
}

fn render_devices(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let device_items: Vec<ListItem> = app
        .devices
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let selected = app.selected_device == Some(i);
            let multi_selected = app.selected_devices.contains(&i);
            let marker = if multi_selected { "[x]" } else { "[ ]" };
            let style = if selected {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            ListItem::new(format!(
                "{} {} - {} ({})",
                marker,
                d.id,
                d.model.as_deref().unwrap_or("Unknown"),
                d.status
            ))
            .style(style)
        })
        .collect();

    let devices_list = List::new(device_items)
        .block(Block::default().borders(Borders::ALL).title("Devices (Space to multi-select)"))
        .highlight_style(Style::default().bg(Color::DarkGray));
    f.render_widget(devices_list, chunks[0]);

    let info = if let Some(idx) = app.selected_device {
        let d = &app.devices[idx];
        format!(
            "Device ID: {}\nStatus: {}\nModel: {}\n\nSelected for batch: {} device(s)",
            d.id,
            d.status,
            d.model.as_deref().unwrap_or("Unknown"),
            app.selected_devices.len()
        )
    } else {
        "No device selected".to_string()
    };

    let info_widget = Paragraph::new(info)
        .block(Block::default().borders(Borders::ALL).title("Device Info"));
    f.render_widget(info_widget, chunks[1]);
}

fn render_commands(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    // Render input area with the actual input text via ratatui
    let input = Paragraph::new(Text::from(app.command_input.as_str()))
        .block(Block::default().borders(Borders::ALL).title("Command (Enter to execute)"));
    f.render_widget(input, chunks[0]);

    // Position cursor at the end of the input text
    // chunks[0] is the input widget area including borders
    // Text content starts at (x + 1 for left border + 1 for padding, y + 1 for top border)
    let cursor_x = chunks[0].x + 2 + app.command_input.chars().count() as u16;
    let cursor_y = chunks[0].y + 1;
    f.set_cursor(cursor_x, cursor_y);

    let output: Vec<ListItem> = app
        .command_output
        .iter()
        .map(|line| ListItem::new(line.as_str()))
        .collect();

    let output_list = List::new(output)
        .block(Block::default().borders(Borders::ALL).title("Output"));
    f.render_widget(output_list, chunks[1]);
}

fn render_files(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(area);

    let remote = Paragraph::new(app.file_remote_path.clone())
        .block(Block::default().borders(Borders::ALL).title("Remote Path (device)"));
    f.render_widget(remote, chunks[0]);

    let local = Paragraph::new(app.file_local_path.clone())
        .block(Block::default().borders(Borders::ALL).title("Local Path (host)"));
    f.render_widget(local, chunks[1]);

    let help = Paragraph::new("Press 'p' to push, 'l' to pull")
        .block(Block::default().borders(Borders::ALL).title("Actions"));
    f.render_widget(help, chunks[2]);
}

fn render_run_binary(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    let path = Paragraph::new(app.binary_path.clone())
        .block(Block::default().borders(Borders::ALL).title("Binary Path on Device"));
    f.render_widget(path, chunks[0]);

    let args = Paragraph::new(app.binary_args.clone())
        .block(Block::default().borders(Borders::ALL).title("Arguments (space separated)"));
    f.render_widget(args, chunks[1]);

    let output: Vec<ListItem> = app
        .command_output
        .iter()
        .map(|line| ListItem::new(line.as_str()))
        .collect();

    let output_list = List::new(output)
        .block(Block::default().borders(Borders::ALL).title("Output (Enter to run)"));
    f.render_widget(output_list, chunks[2]);
}

fn render_logs(f: &mut Frame, area: Rect, app: &App) {
    let logs: Vec<ListItem> = app
        .logs
        .iter()
        .rev()
        .map(|line| ListItem::new(line.as_str()))
        .collect();

    let logs_list = List::new(logs)
        .block(Block::default().borders(Borders::ALL).title("ADB Command Logs (newest first)"));
    f.render_widget(logs_list, area);
}
