use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;

use crate::ui::UiTheme;

const MAX_LINES: usize = 500;
const POLL_INTERVAL_SECS: f64 = 0.5;
const OVERLAY_HEIGHT_PCT: f32 = 35.0;
const DISPLAY_LINES: usize = 60;
const LOG_FONT_SIZE: f32 = 11.0;

pub struct LogViewerPlugin;

impl Plugin for LogViewerPlugin {
    fn build(&self, app: &mut App) {
        let shared_lines = Arc::new(Mutex::new(VecDeque::<String>::new()));
        let tailer = LogTailer::spawn(log_file_path(), Arc::clone(&shared_lines));

        app.insert_resource(LogViewerState {
            visible: false,
            lines: VecDeque::new(),
            shared_lines,
            _tailer: tailer,
            dirty: true,
        })
        .add_systems(Update, (toggle_log_viewer, sync_log_lines, render_log_overlay));
    }
}

fn log_file_path() -> PathBuf {
    dirs::home_dir()
        .expect("could not find home directory")
        .join(".nightingale")
        .join("nightingale.log")
}

// --- Resources ---

#[derive(Resource)]
pub struct LogViewerState {
    pub visible: bool,
    lines: VecDeque<String>,
    shared_lines: Arc<Mutex<VecDeque<String>>>,
    _tailer: LogTailer,
    dirty: bool,
}

/// Handle to the background tailer thread (kept alive via the resource).
struct LogTailer {
    _handle: std::thread::JoinHandle<()>,
}

impl LogTailer {
    fn spawn(path: PathBuf, shared: Arc<Mutex<VecDeque<String>>>) -> Self {
        let handle = std::thread::spawn(move || {
            Self::run(path, shared);
        });
        Self { _handle: handle }
    }

    fn run(path: PathBuf, shared: Arc<Mutex<VecDeque<String>>>) {
        let mut file_pos: u64 = 0;

        // Read existing content first
        if let Ok(mut file) = std::fs::File::open(&path) {
            let mut buf = String::new();
            if file.read_to_string(&mut buf).is_ok() {
                let mut guard = shared.lock().unwrap();
                for line in buf.lines() {
                    if guard.len() >= MAX_LINES {
                        guard.pop_front();
                    }
                    guard.push_back(line.to_string());
                }
                file_pos = buf.len() as u64;
            }
        }

        loop {
            std::thread::sleep(std::time::Duration::from_millis(
                (POLL_INTERVAL_SECS * 1000.0) as u64,
            ));

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
                    if guard.len() >= MAX_LINES {
                        guard.pop_front();
                    }
                    guard.push_back(line.to_string());
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
        _  => DEFAULT_COLOR,
    }
}

fn parse_ansi_line(line: &str) -> Vec<AnsiSpan> {
    let mut spans = Vec::new();
    let mut current_color = DEFAULT_COLOR;
    let mut buf = String::new();
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Look for CSI sequence: ESC [ ... m
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                let mut seq = String::new();
                loop {
                    match chars.next() {
                        Some(c) if c.is_ascii_digit() || c == ';' => seq.push(c),
                        Some('m') => break,
                        _ => {
                            // Not a valid SGR sequence, emit what we consumed
                            buf.push('\x1b');
                            buf.push('[');
                            buf.push_str(&seq);
                            break;
                        }
                    }
                }
                // Flush current buffer
                if !buf.is_empty() {
                    spans.push(AnsiSpan { text: buf.clone(), color: current_color });
                    buf.clear();
                }
                // Parse SGR codes
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
                            // Skip bold/underline/etc (1, 2, 3, 4...) — just ignore
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
        spans.push(AnsiSpan { text: buf, color: current_color });
    }

    if spans.is_empty() {
        spans.push(AnsiSpan { text: String::new(), color: DEFAULT_COLOR });
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
        // Sync config so settings toggle stays in sync
        config.show_logs = Some(state.visible);
        config.save();
    }
}

fn sync_log_lines(
    mut state: ResMut<LogViewerState>,
    config: Res<crate::config::AppConfig>,
) {
    // Sync visibility with settings toggle
    let config_visible = config.show_logs();
    if config_visible != state.visible {
        state.visible = config_visible;
        state.dirty = true;
    }
    if !state.visible {
        return;
    }
    let shared = Arc::clone(&state.shared_lines);
    let guard = shared.lock().unwrap();
    if guard.len() != state.lines.len() || guard.back() != state.lines.back() {
        state.lines = guard.clone();
        state.dirty = true;
    }
}

fn render_log_overlay(
    mut commands: Commands,
    mut state: ResMut<LogViewerState>,
    theme: Res<UiTheme>,
    existing: Query<Entity, With<LogOverlayRoot>>,
) {
    if !state.dirty {
        return;
    }
    state.dirty = false;

    // Always despawn old overlay
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    if !state.visible {
        return;
    }

    // Collect display lines with ANSI parsing
    let display: Vec<Vec<AnsiSpan>> = state
        .lines
        .iter()
        .rev()
        .take(DISPLAY_LINES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|line| parse_ansi_line(line))
        .collect();

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
                    header.spawn((
                        Text::new("LOG OUTPUT"),
                        TextFont { font_size: 11.0, ..default() },
                        TextColor(theme.text_dim),
                    ));
                    header.spawn((
                        Text::new("[F12 to close]"),
                        TextFont { font_size: 11.0, ..default() },
                        TextColor(theme.text_dim),
                    ));
                });

            // Separator line
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

            // Log content — one Text entity per line, with ANSI-colored spans
            overlay
                .spawn(Node {
                    flex_grow: 1.0,
                    overflow: Overflow::clip(),
                    flex_direction: FlexDirection::ColumnReverse,
                    ..default()
                })
                .with_children(|scroll_area| {
                    scroll_area
                        .spawn(Node {
                            flex_direction: FlexDirection::Column,
                            ..default()
                        })
                        .with_children(|content| {
                            for line_spans in &display {
                                // First span becomes the root Text
                                let first = &line_spans[0];
                                let mut line_entity = content.spawn((
                                    LogOverlayText,
                                    Text::new(&first.text),
                                    TextFont { font_size: LOG_FONT_SIZE, ..default() },
                                    TextColor(first.color),
                                ));

                                // Remaining spans as children
                                if line_spans.len() > 1 {
                                    line_entity.with_children(|parent| {
                                        for span in &line_spans[1..] {
                                            parent.spawn((
                                                TextSpan::new(&span.text),
                                                TextFont { font_size: LOG_FONT_SIZE, ..default() },
                                                TextColor(span.color),
                                            ));
                                        }
                                    });
                                }
                            }
                        });
                });
        });
}
