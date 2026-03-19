use std::sync::{Arc, Mutex};

use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;

use super::components::*;
use crate::downloader::{DownloadManager, DownloadPhase};
use crate::spotify::api::{SpotifyAlbum, SpotifyClient, SpotifyTrack};
use crate::config::AppConfig;
use crate::ui::UiTheme;

// --- Resources ---

#[derive(Resource)]
pub struct SpotifySearchState {
    pub query: String,
    pub active_tab: SpotifySearchTab,
    pub track_results: Vec<SpotifyTrack>,
    pub album_results: Vec<SpotifyAlbum>,
    pub searching: bool,
    pub error: Option<String>,
    pub dirty: bool,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pending: Option<Arc<Mutex<Option<SearchResult>>>>,
}

enum SearchResult {
    Tracks(Result<Vec<SpotifyTrack>, String>),
    Albums(Result<Vec<SpotifyAlbum>, String>),
}

// Pending album track fetch
#[derive(Resource)]
pub struct PendingAlbumFetch {
    album_name: String,
    result: Arc<Mutex<Option<Result<Vec<SpotifyTrack>, String>>>>,
}

impl Default for SpotifySearchState {
    fn default() -> Self {
        Self {
            query: String::new(),
            active_tab: SpotifySearchTab::Tracks,
            track_results: Vec::new(),
            album_results: Vec::new(),
            searching: false,
            error: None,
            dirty: true,
            client_id: None,
            client_secret: None,
            pending: None,
        }
    }
}

// --- Spawn / Despawn ---

pub fn spawn_spotify_search(commands: &mut Commands, theme: &UiTheme, config: &AppConfig) {
    let mut state = SpotifySearchState::default();
    state.client_id = config.spotify_client_id.clone();
    state.client_secret = config.spotify_client_secret.clone();
    commands.insert_resource(state);
    let state = SpotifySearchState::default();
    rebuild_overlay(commands, theme, &state, &DownloadManager::default());
}

fn despawn_spotify_search(commands: &mut Commands, overlay: &Query<Entity, With<SpotifySearchOverlay>>) {
    for entity in overlay {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<SpotifySearchState>();
    commands.remove_resource::<PendingAlbumFetch>();
}

fn rebuild_overlay(
    commands: &mut Commands,
    theme: &UiTheme,
    state: &SpotifySearchState,
    dl_manager: &DownloadManager,
) {
    commands
        .spawn((
            SpotifySearchOverlay,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(theme.overlay_dim),
            GlobalZIndex(10),
        ))
        .with_children(|overlay| {
            overlay
                .spawn((
                    Node {
                        width: Val::Px(640.0),
                        height: Val::Px(520.0),
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(Val::Px(24.0)),
                        border_radius: BorderRadius::all(Val::Px(8.0)),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    BackgroundColor(theme.surface),
                ))
                .with_children(|card| {
                    // Header
                    card.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
                        margin: UiRect::bottom(Val::Px(12.0)),
                        flex_shrink: 0.0,
                        ..default()
                    })
                    .with_children(|header| {
                        header.spawn((
                            Text::new("Search Spotify"),
                            TextFont { font_size: 20.0, ..default() },
                            TextColor(theme.text_primary),
                        ));
                        // Close button
                        header.spawn((
                            SpotifyCloseButton,
                            Button,
                            Node {
                                padding: UiRect::new(Val::Px(8.0), Val::Px(8.0), Val::Px(4.0), Val::Px(4.0)),
                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(theme.popup_btn),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                Text::new("X"),
                                TextFont { font_size: 14.0, ..default() },
                                TextColor(theme.text_secondary),
                            ));
                        });
                    });

                    // Search input display
                    card.spawn((
                        SpotifySearchInput,
                        Node {
                            padding: UiRect::new(Val::Px(12.0), Val::Px(12.0), Val::Px(8.0), Val::Px(8.0)),
                            margin: UiRect::bottom(Val::Px(8.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            flex_shrink: 0.0,
                            ..default()
                        },
                        BorderColor::all(theme.text_dim),
                        BackgroundColor(theme.card_bg),
                    ))
                    .with_children(|input| {
                        let display_text = if state.query.is_empty() {
                            "Type to search..."
                        } else {
                            &state.query
                        };
                        let color = if state.query.is_empty() {
                            theme.text_dim
                        } else {
                            theme.text_primary
                        };
                        input.spawn((
                            Text::new(display_text),
                            TextFont { font_size: 14.0, ..default() },
                            TextColor(color),
                        ));
                    });

                    // Tab bar
                    card.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(6.0),
                        margin: UiRect::bottom(Val::Px(8.0)),
                        flex_shrink: 0.0,
                        ..default()
                    })
                    .with_children(|tabs| {
                        for tab in [SpotifySearchTab::Tracks, SpotifySearchTab::Albums] {
                            let label = match tab {
                                SpotifySearchTab::Tracks => "Tracks",
                                SpotifySearchTab::Albums => "Albums",
                            };
                            let is_active = state.active_tab == tab;
                            let bg = if is_active { theme.accent } else { theme.popup_btn };
                            let text_color = if is_active { Color::WHITE } else { theme.text_secondary };
                            tabs.spawn((
                                SpotifyTabButton(tab),
                                Button,
                                Node {
                                    padding: UiRect::new(Val::Px(12.0), Val::Px(12.0), Val::Px(6.0), Val::Px(6.0)),
                                    border_radius: BorderRadius::all(Val::Px(4.0)),
                                    ..default()
                                },
                                BackgroundColor(bg),
                            ))
                            .with_children(|btn| {
                                btn.spawn((
                                    Text::new(label),
                                    TextFont { font_size: 13.0, ..default() },
                                    TextColor(text_color),
                                ));
                            });
                        }

                        // Status text
                        if state.searching {
                            tabs.spawn((
                                Text::new("Searching..."),
                                TextFont { font_size: 12.0, ..default() },
                                TextColor(theme.text_dim),
                                Node { margin: UiRect::left(Val::Px(8.0)), ..default() },
                            ));
                        }
                    });

                    // Error
                    if let Some(ref err) = state.error {
                        card.spawn((
                            Text::new(err.as_str()),
                            TextFont { font_size: 12.0, ..default() },
                            TextColor(theme.badge_failed),
                            Node {
                                margin: UiRect::bottom(Val::Px(6.0)),
                                flex_shrink: 0.0,
                                ..default()
                            },
                        ));
                    }

                    // Results area
                    card.spawn((
                        SpotifySearchResultsRoot,
                        Node {
                            flex_grow: 1.0,
                            flex_basis: Val::Px(0.0),
                            flex_direction: FlexDirection::Column,
                            overflow: Overflow::scroll_y(),
                            row_gap: Val::Px(4.0),
                            ..default()
                        },
                    ))
                    .with_children(|results| {
                        match state.active_tab {
                            SpotifySearchTab::Tracks => {
                                if state.track_results.is_empty() && !state.query.is_empty() && !state.searching {
                                    results.spawn((
                                        Text::new("No tracks found"),
                                        TextFont { font_size: 13.0, ..default() },
                                        TextColor(theme.text_dim),
                                    ));
                                }
                                for (i, track) in state.track_results.iter().enumerate() {
                                    spawn_track_row(results, theme, track, i, dl_manager);
                                }
                            }
                            SpotifySearchTab::Albums => {
                                if state.album_results.is_empty() && !state.query.is_empty() && !state.searching {
                                    results.spawn((
                                        Text::new("No albums found"),
                                        TextFont { font_size: 13.0, ..default() },
                                        TextColor(theme.text_dim),
                                    ));
                                }
                                for (i, album) in state.album_results.iter().enumerate() {
                                    spawn_album_row(results, theme, album, i);
                                }
                            }
                        }
                    });

                    // Download queue section
                    let active_list = dl_manager.active_progress_list();
                    let queued = dl_manager.total_queued();
                    let completed_count = dl_manager.completed.len();
                    let failed_count = dl_manager.failed.len();

                    if !active_list.is_empty() || queued > 0 || completed_count > 0 || failed_count > 0 {
                        card.spawn(Node {
                            height: Val::Px(1.0),
                            width: Val::Percent(100.0),
                            margin: UiRect::new(Val::ZERO, Val::ZERO, Val::Px(8.0), Val::Px(4.0)),
                            flex_shrink: 0.0,
                            ..default()
                        })
                        .insert(BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.1)));

                        card.spawn((
                            SpotifyDownloadQueueRoot,
                            Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(3.0),
                                max_height: Val::Px(100.0),
                                overflow: Overflow::scroll_y(),
                                flex_shrink: 0.0,
                                ..default()
                            },
                        ))
                        .with_children(|queue| {
                            // Active downloads
                            for (track, progress) in &active_list {
                                let status = format!(
                                    "{} {} - {} ({:.0}%)",
                                    if matches!(progress.phase, DownloadPhase::Done) { "OK" } else { ">>" },
                                    track.artists.first().unwrap_or(&"?".to_string()),
                                    track.name,
                                    progress.percent,
                                );
                                let color = match progress.phase {
                                    DownloadPhase::Done => theme.badge_ready,
                                    DownloadPhase::Failed => theme.badge_failed,
                                    _ => theme.badge_analyzing,
                                };
                                queue.spawn((
                                    Text::new(status),
                                    TextFont { font_size: 11.0, ..default() },
                                    TextColor(color),
                                ));
                            }

                            // Queued
                            for req in &dl_manager.queue {
                                queue.spawn((
                                    Text::new(format!(
                                        ".. {} - {}",
                                        req.track.artists.first().unwrap_or(&"?".to_string()),
                                        req.track.name,
                                    )),
                                    TextFont { font_size: 11.0, ..default() },
                                    TextColor(theme.text_dim),
                                ));
                            }

                            // Recent completed
                            for cd in dl_manager.completed.iter().rev().take(3) {
                                queue.spawn((
                                    Text::new(format!(
                                        "OK {} - {}",
                                        cd.track.artists.first().unwrap_or(&"?".to_string()),
                                        cd.track.name,
                                    )),
                                    TextFont { font_size: 11.0, ..default() },
                                    TextColor(theme.badge_ready),
                                ));
                            }

                            // Recent failed
                            for (track, err) in dl_manager.failed.iter().rev().take(2) {
                                queue.spawn((
                                    Text::new(format!(
                                        "!! {} - {} ({})",
                                        track.artists.first().unwrap_or(&"?".to_string()),
                                        track.name,
                                        err,
                                    )),
                                    TextFont { font_size: 11.0, ..default() },
                                    TextColor(theme.badge_failed),
                                ));
                            }
                        });
                    }
                });
        });
}

fn spawn_track_row(
    parent: &mut ChildSpawnerCommands,
    theme: &UiTheme,
    track: &SpotifyTrack,
    index: usize,
    dl_manager: &DownloadManager,
) {
    let duration_secs = track.duration_ms / 1000;
    let mins = duration_secs / 60;
    let secs = duration_secs % 60;
    let artist = track.artists.join(", ");

    let status = dl_manager.status_of(&track.id);

    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect::new(Val::Px(8.0), Val::Px(8.0), Val::Px(6.0), Val::Px(6.0)),
            border_radius: BorderRadius::all(Val::Px(4.0)),
            ..default()
        })
        .insert(BackgroundColor(theme.card_bg))
        .with_children(|row| {
            // Track info
            row.spawn(Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                ..default()
            })
            .with_children(|info| {
                info.spawn((
                    Text::new(&track.name),
                    TextFont { font_size: 13.0, ..default() },
                    TextColor(theme.text_primary),
                ));
                info.spawn((
                    Text::new(format!("{artist} · {} · {mins}:{secs:02}", track.album_name)),
                    TextFont { font_size: 11.0, ..default() },
                    TextColor(theme.text_secondary),
                ));
            });

            // Download button or status
            match status {
                Some(DownloadPhase::Done) => {
                    spawn_status_badge(row, theme, "Ready", theme.badge_ready);
                }
                Some(DownloadPhase::Failed) => {
                    spawn_status_badge(row, theme, "Failed", theme.badge_failed);
                }
                Some(DownloadPhase::Queued) => {
                    spawn_status_badge(row, theme, "Queued", theme.badge_queued);
                }
                Some(_) => {
                    spawn_status_badge(row, theme, "DL...", theme.badge_analyzing);
                }
                None => {
                    row.spawn((
                        SpotifyTrackDownloadBtn { index },
                        Button,
                        Node {
                            padding: UiRect::new(Val::Px(10.0), Val::Px(10.0), Val::Px(5.0), Val::Px(5.0)),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            margin: UiRect::left(Val::Px(8.0)),
                            ..default()
                        },
                        BackgroundColor(theme.accent),
                    ))
                    .with_children(|btn| {
                        btn.spawn((
                            Text::new("DL"),
                            TextFont { font_size: 12.0, ..default() },
                            TextColor(Color::WHITE),
                        ));
                    });
                }
            }
        });
}

fn spawn_album_row(
    parent: &mut ChildSpawnerCommands,
    theme: &UiTheme,
    album: &SpotifyAlbum,
    index: usize,
) {
    let artist = album.artists.join(", ");

    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect::new(Val::Px(8.0), Val::Px(8.0), Val::Px(6.0), Val::Px(6.0)),
            border_radius: BorderRadius::all(Val::Px(4.0)),
            ..default()
        })
        .insert(BackgroundColor(theme.card_bg))
        .with_children(|row| {
            row.spawn(Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                ..default()
            })
            .with_children(|info| {
                info.spawn((
                    Text::new(&album.name),
                    TextFont { font_size: 13.0, ..default() },
                    TextColor(theme.text_primary),
                ));
                info.spawn((
                    Text::new(format!("{artist} · {} tracks · {}", album.total_tracks, album.release_date)),
                    TextFont { font_size: 11.0, ..default() },
                    TextColor(theme.text_secondary),
                ));
            });

            row.spawn((
                SpotifyAlbumDownloadBtn { index },
                Button,
                Node {
                    padding: UiRect::new(Val::Px(10.0), Val::Px(10.0), Val::Px(5.0), Val::Px(5.0)),
                    border_radius: BorderRadius::all(Val::Px(4.0)),
                    margin: UiRect::left(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(theme.accent),
            ))
            .with_children(|btn| {
                btn.spawn((
                    Text::new("DL All"),
                    TextFont { font_size: 12.0, ..default() },
                    TextColor(Color::WHITE),
                ));
            });
        });
}

fn spawn_status_badge(parent: &mut ChildSpawnerCommands, theme: &UiTheme, label: &str, color: Color) {
    parent
        .spawn((
            Node {
                padding: UiRect::new(Val::Px(8.0), Val::Px(8.0), Val::Px(3.0), Val::Px(3.0)),
                border_radius: BorderRadius::all(Val::Px(4.0)),
                margin: UiRect::left(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(color),
        ))
        .with_children(|badge| {
            badge.spawn((
                Text::new(label),
                TextFont { font_size: 11.0, ..default() },
                TextColor(Color::WHITE),
            ));
        });
}

// --- Systems ---

/// Handle keyboard input in the Spotify search overlay
pub(super) fn handle_spotify_search_input(
    mut key_events: MessageReader<KeyboardInput>,
    mut commands: Commands,
    mut state: Option<ResMut<SpotifySearchState>>,
    overlay: Query<Entity, With<SpotifySearchOverlay>>,
    theme: Res<UiTheme>,
    dl_manager: Res<DownloadManager>,
) {
    let Some(ref mut state) = state else { return };
    if overlay.is_empty() { return; }

    // Check for pending search results
    let pending_result = state.pending.as_ref().and_then(|p| {
        p.try_lock().ok().and_then(|mut guard| guard.take())
    });
    if let Some(result) = pending_result {
        state.searching = false;
        state.pending = None;
        match result {
            SearchResult::Tracks(Ok(tracks)) => {
                state.track_results = tracks;
                state.error = None;
            }
            SearchResult::Tracks(Err(e)) => {
                state.error = Some(e);
            }
            SearchResult::Albums(Ok(albums)) => {
                state.album_results = albums;
                state.error = None;
            }
            SearchResult::Albums(Err(e)) => {
                state.error = Some(e);
            }
        }
        state.dirty = true;
    }

    let mut changed = false;

    for ev in key_events.read() {
        if !ev.state.is_pressed() {
            continue;
        }

        if ev.key_code == KeyCode::Escape {
            despawn_spotify_search(&mut commands, &overlay);
            return;
        }

        if ev.key_code == KeyCode::Backspace {
            if !state.query.is_empty() {
                state.query.pop();
                changed = true;
            }
            continue;
        }

        if ev.key_code == KeyCode::Enter {
            if !state.query.is_empty() && !state.searching {
                trigger_search(state);
            }
            continue;
        }

        if ev.key_code == KeyCode::Tab {
            state.active_tab = match state.active_tab {
                SpotifySearchTab::Tracks => SpotifySearchTab::Albums,
                SpotifySearchTab::Albums => SpotifySearchTab::Tracks,
            };
            if !state.query.is_empty() && !state.searching {
                trigger_search(state);
            } else {
                state.dirty = true;
            }
            continue;
        }

        if let Some(ref text) = ev.text {
            for c in text.chars() {
                if !c.is_control() {
                    state.query.push(c);
                    changed = true;
                }
            }
        }
    }

    if changed || state.dirty {
        // Rebuild overlay
        for entity in &overlay {
            commands.entity(entity).despawn();
        }
        state.dirty = false;
        rebuild_overlay(&mut commands, &theme, state, &dl_manager);
    }
}

fn trigger_search(state: &mut SpotifySearchState) {
    state.searching = true;
    state.dirty = true;

    let query = state.query.clone();
    let tab = state.active_tab;
    let cid = state.client_id.clone();
    let csecret = state.client_secret.clone();
    let result: Arc<Mutex<Option<SearchResult>>> = Arc::new(Mutex::new(None));
    let result_clone = Arc::clone(&result);

    std::thread::spawn(move || {
        let mut client = match SpotifyClient::new(cid.as_deref(), csecret.as_deref()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[spotify] {e}");
                *result_clone.lock().unwrap() = Some(SearchResult::Tracks(Err(e)));
                return;
            }
        };
        let search_result = match tab {
            SpotifySearchTab::Tracks => {
                SearchResult::Tracks(client.search_tracks(&query, 20))
            }
            SpotifySearchTab::Albums => {
                SearchResult::Albums(client.search_albums(&query, 20))
            }
        };
        *result_clone.lock().unwrap() = Some(search_result);
    });

    state.pending = Some(result);
}

/// Handle button clicks (close, tabs, download, album download)
pub(super) fn handle_spotify_search_interaction(
    mut commands: Commands,
    close_query: Query<&Interaction, (Changed<Interaction>, With<SpotifyCloseButton>)>,
    tab_query: Query<(&Interaction, &SpotifyTabButton), Changed<Interaction>>,
    track_dl_query: Query<(&Interaction, &SpotifyTrackDownloadBtn), Changed<Interaction>>,
    album_dl_query: Query<(&Interaction, &SpotifyAlbumDownloadBtn), Changed<Interaction>>,
    overlay: Query<Entity, With<SpotifySearchOverlay>>,
    mut state: Option<ResMut<SpotifySearchState>>,
    mut dl_manager: ResMut<DownloadManager>,
    theme: Res<UiTheme>,
    pending_album: Option<Res<PendingAlbumFetch>>,
) {
    if overlay.is_empty() { return; }
    let Some(ref mut state) = state else { return; };

    // Check pending album fetch
    let mut remove_pending = false;
    if let Some(ref pending) = pending_album {
        if let Ok(mut guard) = pending.result.try_lock() {
            if let Some(result) = guard.take() {
                match result {
                    Ok(tracks) => {
                        eprintln!("[spotify] Fetched {} tracks from album '{}'", tracks.len(), pending.album_name);
                        dl_manager.enqueue_all(tracks);
                        state.dirty = true;
                    }
                    Err(e) => {
                        state.error = Some(format!("Album fetch failed: {e}"));
                        state.dirty = true;
                    }
                }
                remove_pending = true;
            }
        }
    }
    if remove_pending {
        commands.remove_resource::<PendingAlbumFetch>();
    }

    // Collect actions first to avoid borrow conflicts
    let mut should_close = false;
    let mut new_tab: Option<SpotifySearchTab> = None;
    let mut tracks_to_dl: Vec<usize> = Vec::new();
    let mut albums_to_dl: Vec<usize> = Vec::new();

    for interaction in &close_query {
        if matches!(interaction, Interaction::Pressed) {
            should_close = true;
        }
    }

    for (interaction, tab_btn) in &tab_query {
        if matches!(interaction, Interaction::Pressed) && state.active_tab != tab_btn.0 {
            new_tab = Some(tab_btn.0);
        }
    }

    for (interaction, btn) in &track_dl_query {
        if matches!(interaction, Interaction::Pressed) {
            tracks_to_dl.push(btn.index);
        }
    }

    for (interaction, btn) in &album_dl_query {
        if matches!(interaction, Interaction::Pressed) {
            albums_to_dl.push(btn.index);
        }
    }

    // Now apply actions
    if should_close {
        despawn_spotify_search(&mut commands, &overlay);
        return;
    }

    if let Some(tab) = new_tab {
        state.active_tab = tab;
        if !state.query.is_empty() && !state.searching {
            trigger_search(state);
        } else {
            state.dirty = true;
        }
    }

    for idx in tracks_to_dl {
        if idx < state.track_results.len() {
            let track = state.track_results[idx].clone();
            eprintln!("[spotify] Enqueuing download: {} - {}", track.artists.join(", "), track.name);
            dl_manager.enqueue(track);
            state.dirty = true;
        }
    }

    for idx in albums_to_dl {
        if idx < state.album_results.len() {
            let album = &state.album_results[idx];
            let album_id = album.id.clone();
            let album_name = album.name.clone();
            eprintln!("[spotify] Fetching album tracks: {album_name}");

            let result: Arc<Mutex<Option<Result<Vec<SpotifyTrack>, String>>>> =
                Arc::new(Mutex::new(None));
            let result_clone = Arc::clone(&result);

            let cid2 = state.client_id.clone();
            let csecret2 = state.client_secret.clone();
            std::thread::spawn(move || {
                let mut client = match SpotifyClient::new(cid2.as_deref(), csecret2.as_deref()) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[spotify] {e}");
                        *result_clone.lock().unwrap() = Some(Err(e));
                        return;
                    }
                };
                *result_clone.lock().unwrap() = Some(client.album_tracks(&album_id));
            });

            commands.insert_resource(PendingAlbumFetch {
                album_name,
                result,
            });
        }
    }
}

/// Periodically rebuild the overlay to update download progress
pub(super) fn update_spotify_download_status(
    mut commands: Commands,
    state: Option<Res<SpotifySearchState>>,
    overlay: Query<Entity, With<SpotifySearchOverlay>>,
    dl_manager: Res<DownloadManager>,
    theme: Res<UiTheme>,
) {
    let Some(ref state) = state else { return; };
    if overlay.is_empty() { return; }

    // Only rebuild if download manager has active work
    if dl_manager.is_changed() {
        for entity in &overlay {
            commands.entity(entity).despawn();
        }
        rebuild_overlay(&mut commands, &theme, state, &dl_manager);
    }
}
