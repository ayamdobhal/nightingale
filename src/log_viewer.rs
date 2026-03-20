use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;

use crate::ui::UiTheme;

const MAX_BUFFER_BYTES: usize = 10 * 1024 * 1024; // 10 MB
const POLL_INTERVAL_MS: u64 = 50;
const OVERLAY_HEIGHT_PCT: f32 = 35.0;
const LOG_FONT_SIZE: f32 = 11.0;
const LINE_HEIGHT: f32 = 14.0;

pub struct LogViewerPlugin;

impl Plugin for LogViewerPlugin {
    fn build(&self, app: &mut App) {
        let shared = Arc::new(Mutex::new(SharedLog::new()));
        let tailer = LogTailer::spawn(log_file_path(), Arc::clone(&shared));

        app.insert_resource(LogViewerState {
            visible: false,
            lines: Vec::new(),
            shared,
            _tailer: tailer,
            dirty: true,
            scroll_offset: 0,
            auto_scroll: true,
            version: 0,
            last_synced_version: 0,
        })
        .add_systems(
            Update,
            (toggle_log_viewer, sync_log_lines, handle_keyboard_scroll, handle_mouse_scroll, render_log_overlay),
        );
    }
}

fn log_file_path() -> PathBuf {
    dirs::home_dir()
        .expect("could not find home directory")
        .join(".nightingale")
        .join("nightingale.log")
}

// --- Shared buffer ---

struct SharedLog {
    lines: VecDeque<String>,
    total_bytes: usize,
    version: u64,
}

impl SharedLog {
    fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            total_bytes: 0,
            version: 0,
        }
    }

    fn push_line(&mut self, line: String) {
        let line_bytes = line.len();
        self.total_bytes += line_bytes;
        self.lines.push_back(line);

        // Evict oldest lines until under budget
        while self.total_bytes > MAX_BUFFER_BYTES && !self.lines.is_empty() {
            if let Some(old) = self.lines.pop_front() {
                self.total_bytes -= old.len();
            }
        }
        self.version += 1;
    }
}

// --- Resources ---

#[derive(Resource)]
pub struct LogViewerState {
    pub visible: bool,
    lines: Vec<String>,
    shared: Arc<Mutex<SharedLog>>,
    _tailer: LogTailer,
    dirty: bool,
    /// 0 = bottom (newest), increases upward
    scroll_offset: usize,
    auto_scroll: bool,
    version: u64,
    last_synced_version: u64,
}

/// Handle to the background tailer thread (kept alive via the resource).
struct LogTailer {
    _handle: std::thread::JoinHandle<()>,
}

impl LogTailer {
    fn spawn(path: PathBuf, shared: Arc<Mutex<SharedLog>>) -> Self {
        let handle = std::thread::spawn(move || {
            Self::run(path, shared);
        });
        Self { _handle: handle }
    }

    fn run(path: PathBuf, shared: Arc<Mutex<SharedLog>>) {
        let mut file_pos: u64 = 0;

        // Read existing content
        if let Ok(mut file) = std::fs::File::open(&path) {
            let mut buf = String::new();
            if file.read_to_string(&mut buf).is_ok() {
                let mut guard = shared.lock().unwrap();
                for line in buf.lines() {
                    guard.push_line(line.to_string());
                }
                file_pos = buf.len() as u64;
            }
        }

        loop {
            std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));

            let Ok(mut file) = std::fs::File::open(&path) else {
                continue;
            };

            let meta_len = file.metadata().map(|m| m.len()).unwrap_or(0);

            // File was truncated / rotated
            if meta_len < file_pos {
                file_pos = 0;
            }

            if meta_len == file_pos {
                continue;
            }

            if file.seek(SeekFrom::Start(file_pos)).is_err() {
                continue;
            }

            let mut buf = String::new();
            let Ok(bytes_read) = file.read_to_string(&mut buf) else {
                continue;
            };
            file_pos += bytes_read as u64;

            if !buf.is_empty() {
                let mut guard = shared.lock().unwrap();
                for line in buf.lines() {
                    guard.push_line(line.to_string());
                }
            }
        }
    }
}

// --- ANSI color parsing ---

#[derive(Clone)]
struct AnsiSpan {
    text: String,
    color: Color,
}

/// Default log text color (light gray)
const DEFAULT_COLOR: Color = Color::srgb(0.78, 0.78, 0.82);

fn ansi_code_to_color(code: u8) -> Color {
    match code {
        30 => Color::srgb(0.20, 0.20, 0.20), // black
        31 => Color::srgb(0.90, 0.30, 0.30), // red
        32 => Color::srgb(0.35, 0.85, 0.35), // green
        33 => Color::srgb(0.90, 0.85, 0.30), // yellow
        34 => Color::srgb(0.40, 0.55, 1.00), // blue
        35 => Color::srgb(0.80, 0.45, 0.90), // magenta
        36 => Color::srgb(0.40, 0.85, 0.85), // cyan
        37 => Color::srgb(0.85, 0.85, 0.85), // white
        // bright variants
        90 => Color::srgb(0.50, 0.50, 0.50), // bright black (gray)
        91 => Color::srgb(1.00, 0.45, 0.45), // bright red
        92 => Color::srgb(0.50, 1.00, 0.50), // bright green
        93 => Color::srgb(1.00, 1.00, 0.50), // bright yellow
        94 => Color::srgb(0.55, 0.70, 1.00), // bright blue
        95 => Color::srgb(1.00, 0.60, 1.00), // bright magenta
        96 => Color::srgb(0.55, 1.00, 1.00), // bright cyan
        97 => Color::srgb(1.00, 1.00, 1.00), // bright white
        _ => DEFAULT_COLOR,
    }
}

fn parse_ansi_line(line: &str) -> Vec<AnsiSpan> {
    let mut spans = Vec::new();
    let mut current_color = DEFAULT_COLOR;
    let mut buf = String::new();
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                let mut seq = String::new();
                loop {
                    match chars.next() {
                        Some(c) if c.is_ascii_digit() || c == ';' => seq.push(c),
                        Some('m') => break,
                        _ => {
                            buf.push('\x1b');
                            buf.push('[');
                            buf.push_str(&seq);
                            break;
                        }
                    }
                }
                if !buf.is_empty() {
                    spans.push(AnsiSpan {
                        text: buf.clone(),
                        color: current_color,
                    });
                    buf.clear();
                }
                if seq.is_empty() || seq == "0" {
                    current_color = DEFAULT_COLOR;
                } else {
                    for part in seq.split(';') {
                        if let Ok(code) = part.parse::<u8>() {
                            if code == 0 {
                                current_color = DEFAULT_COLOR;
                            } else if (30..=37).contains(&code) || (90..=97).contains(&code) {
                                current_color = ansi_code_to_color(code);
                            }
                        }
                    }
                }
            } else {
                buf.push(ch);
            }
        } else {
            buf.push(ch);
        }
    }

    if !buf.is_empty() {
        spans.push(AnsiSpan {
            text: buf,
            color: current_color,
        });
    }

    if spans.is_empty() {
        spans.push(AnsiSpan {
            text: String::new(),
            color: DEFAULT_COLOR,
        });
    }

    spans
}

// --- Components ---

#[derive(Component)]
struct LogOverlayRoot;

#[derive(Component)]
struct LogOverlayText;

// --- Systems ---

fn toggle_log_viewer(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<LogViewerState>,
    mut config: ResMut<crate::config::AppConfig>,
) {
    if keyboard.just_pressed(KeyCode::F12) {
        state.visible = !state.visible;
        state.dirty = true;
        state.scroll_offset = 0;
        state.auto_scroll = true;
        config.show_logs = Some(state.visible);
        config.save();
    }
}

fn sync_log_lines(mut state: ResMut<LogViewerState>, config: Res<crate::config::AppConfig>) {
    // Sync visibility with settings toggle
    let config_visible = config.show_logs();
    if config_visible != state.visible {
        state.visible = config_visible;
        state.dirty = true;
    }
    if !state.visible {
        return;
    }

    let shared = Arc::clone(&state.shared);
    let guard = shared.lock().unwrap();

    if guard.version == state.last_synced_version {
        return;
    }

    state.lines = guard.lines.iter().cloned().collect();
    state.version = guard.version;
    state.last_synced_version = guard.version;

    // Auto-scroll keeps us pinned to bottom
    if state.auto_scroll {
        state.scroll_offset = 0;
    }
    state.dirty = true;
}

fn apply_scroll(state: &mut LogViewerState, delta_lines: i32) {
    let total_lines = state.lines.len();
    if total_lines == 0 {
        return;
    }

    if delta_lines > 0 {
        // Scrolling up (into history)
        state.scroll_offset = state
            .scroll_offset
            .saturating_add(delta_lines as usize);
        state.auto_scroll = false;
    } else if delta_lines < 0 {
        // Scrolling down (toward newest)
        state.scroll_offset = state
            .scroll_offset
            .saturating_sub((-delta_lines) as usize);
        if state.scroll_offset == 0 {
            state.auto_scroll = true;
        }
    }

    let max_scroll = total_lines.saturating_sub(1);
    state.scroll_offset = state.scroll_offset.min(max_scroll);
    state.dirty = true;
}

fn handle_keyboard_scroll(
    mut state: ResMut<LogViewerState>,
    keyboard: Res<ButtonInput<KeyCode>>,
) {
    if !state.visible {
        return;
    }

    // Page Up / Page Down for big jumps, arrow keys for single lines
    if keyboard.just_pressed(KeyCode::PageUp) {
        apply_scroll(&mut state, 20);
    }
    if keyboard.just_pressed(KeyCode::PageDown) {
        apply_scroll(&mut state, -20);
    }
    if keyboard.just_pressed(KeyCode::ArrowUp) {
        apply_scroll(&mut state, 3);
    }
    if keyboard.just_pressed(KeyCode::ArrowDown) {
        apply_scroll(&mut state, -3);
    }
    // Home = jump to oldest, End = jump to newest
    if keyboard.just_pressed(KeyCode::Home) {
        state.scroll_offset = state.lines.len().saturating_sub(1);
        state.auto_scroll = false;
        state.dirty = true;
    }
    if keyboard.just_pressed(KeyCode::End) {
        state.scroll_offset = 0;
        state.auto_scroll = true;
        state.dirty = true;
    }
}

fn handle_mouse_scroll(
    mut mouse_wheel: MessageReader<MouseWheel>,
    mut state: ResMut<LogViewerState>,
) {
    if !state.visible {
        return;
    }

    for event in mouse_wheel.read() {
        let scroll_lines = match event.unit {
            MouseScrollUnit::Line => event.y as i32,
            MouseScrollUnit::Pixel => (event.y / LINE_HEIGHT) as i32,
        };
        apply_scroll(&mut state, scroll_lines);
    }
}

fn render_log_overlay(
    mut commands: Commands,
    mut state: ResMut<LogViewerState>,
    theme: Res<UiTheme>,
    existing: Query<Entity, With<LogOverlayRoot>>,
    windows: Query<&Window>,
) {
    if !state.dirty {
        return;
    }
    state.dirty = false;

    for entity in &existing {
        commands.entity(entity).despawn();
    }

    if !state.visible {
        return;
    }

    // Calculate how many lines fit in the overlay
    let window_height = windows
        .iter()
        .next()
        .map(|w| w.height())
        .unwrap_or(720.0);
    let overlay_height = window_height * (OVERLAY_HEIGHT_PCT / 100.0);
    let header_height = 30.0;
    let content_height = overlay_height - header_height;
    let visible_lines = ((content_height / LINE_HEIGHT) as usize).max(1);

    let total_lines = state.lines.len();
    let scroll = state.scroll_offset.min(total_lines.saturating_sub(visible_lines));

    // Window into lines: skip `scroll` from the end, take `visible_lines`
    let end = total_lines.saturating_sub(scroll);
    let start = end.saturating_sub(visible_lines);
    let display: Vec<Vec<AnsiSpan>> = state.lines[start..end]
        .iter()
        .map(|line| parse_ansi_line(line))
        .collect();

    // Scroll indicator
    let scroll_info = if scroll > 0 {
        format!("[{} lines above | scroll to see more]", scroll)
    } else {
        "[F12 to close]".to_string()
    };

    commands
        .spawn((
            LogOverlayRoot,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(OVERLAY_HEIGHT_PCT),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(12.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.04, 0.88)),
            GlobalZIndex(900),
        ))
        .with_children(|overlay| {
            // Header row
            overlay
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    margin: UiRect::bottom(Val::Px(6.0)),
                    flex_shrink: 0.0,
                    ..default()
                })
                .with_children(|header| {
                    let line_count = format!(
                        "LOG OUTPUT ({} lines, {:.1} MB)",
                        total_lines,
                        state.lines.iter().map(|l| l.len()).sum::<usize>() as f64
                            / (1024.0 * 1024.0)
                    );
                    header.spawn((
                        Text::new(line_count),
                        TextFont {
                            font_size: 11.0,
                            ..default()
                        },
                        TextColor(theme.text_dim),
                    ));
                    header.spawn((
                        Text::new(scroll_info),
                        TextFont {
                            font_size: 11.0,
                            ..default()
                        },
                        TextColor(theme.text_dim),
                    ));
                });

            // Separator
            overlay.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(1.0),
                    margin: UiRect::bottom(Val::Px(6.0)),
                    flex_shrink: 0.0,
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.1)),
            ));

            // Log content — only visible lines rendered
            overlay
                .spawn(Node {
                    flex_grow: 1.0,
                    overflow: Overflow::clip(),
                    flex_direction: FlexDirection::Column,
                    ..default()
                })
                .with_children(|content| {
                    for line_spans in &display {
                        let first = &line_spans[0];
                        let mut line_entity = content.spawn((
                            LogOverlayText,
                            Text::new(&first.text),
                            TextFont {
                                font_size: LOG_FONT_SIZE,
                                ..default()
                            },
                            TextColor(first.color),
                            Node {
                                height: Val::Px(LINE_HEIGHT),
                                ..default()
                            },
                        ));

                        if line_spans.len() > 1 {
                            line_entity.with_children(|parent| {
                                for span in &line_spans[1..] {
                                    parent.spawn((
                                        TextSpan::new(&span.text),
                                        TextFont {
                                            font_size: LOG_FONT_SIZE,
                                            ..default()
                                        },
                                        TextColor(span.color),
                                    ));
                                }
                            });
                        }
                    }
                });
        });
}
