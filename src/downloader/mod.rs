pub mod ytdlp;
pub mod tagger;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;

use crate::config::AppConfig;
use crate::spotify::api::SpotifyTrack;

const MAX_CONCURRENT: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadPhase {
    Queued,
    FetchingYtdlp,
    SearchingYoutube,
    Downloading,
    Converting,
    Tagging,
    AddingToLibrary,
    Done,
    Failed,
}

impl DownloadPhase {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::FetchingYtdlp => "Installing yt-dlp...",
            Self::SearchingYoutube => "Searching YouTube...",
            Self::Downloading => "Downloading...",
            Self::Converting => "Converting...",
            Self::Tagging => "Tagging metadata...",
            Self::AddingToLibrary => "Adding to library...",
            Self::Done => "Done",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub phase: DownloadPhase,
    pub percent: f32,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct DownloadRequest {
    pub track: SpotifyTrack,
}

pub struct ActiveDownload {
    pub request: DownloadRequest,
    pub progress: Arc<Mutex<DownloadProgress>>,
    pub thread: Option<std::thread::JoinHandle<Result<PathBuf, String>>>,
}

#[derive(Clone)]
pub struct CompletedDownload {
    pub track: SpotifyTrack,
    pub path: PathBuf,
}

#[derive(Resource, Default)]
pub struct DownloadManager {
    pub queue: VecDeque<DownloadRequest>,
    pub active: Vec<ActiveDownload>,
    pub completed: Vec<CompletedDownload>,
    pub failed: Vec<(SpotifyTrack, String)>,
}

impl DownloadManager {
    pub fn enqueue(&mut self, track: SpotifyTrack) {
        if self.is_queued_or_active(&track.id) || self.is_completed(&track.id) {
            eprintln!("[downloader] Skipping duplicate: {} - {}", track.artists.join(", "), track.name);
            return;
        }
        eprintln!("[downloader] Enqueued: {} - {} (queue size: {})", track.artists.join(", "), track.name, self.queue.len() + 1);
        self.queue.push_back(DownloadRequest { track });
    }

    pub fn enqueue_all(&mut self, tracks: Vec<SpotifyTrack>) {
        eprintln!("[downloader] Enqueuing {} tracks", tracks.len());
        for track in tracks {
            self.enqueue(track);
        }
    }

    pub fn is_queued_or_active(&self, spotify_id: &str) -> bool {
        self.queue.iter().any(|r| r.track.id == spotify_id)
            || self.active.iter().any(|a| a.request.track.id == spotify_id)
    }

    pub fn is_completed(&self, spotify_id: &str) -> bool {
        self.completed.iter().any(|c| c.track.id == spotify_id)
    }

    pub fn is_failed(&self, spotify_id: &str) -> bool {
        self.failed.iter().any(|(t, _)| t.id == spotify_id)
    }

    pub fn status_of(&self, spotify_id: &str) -> Option<DownloadPhase> {
        if self.is_completed(spotify_id) {
            return Some(DownloadPhase::Done);
        }
        if self.is_failed(spotify_id) {
            return Some(DownloadPhase::Failed);
        }
        for active in &self.active {
            if active.request.track.id == spotify_id {
                return Some(active.progress.lock().unwrap().phase.clone());
            }
        }
        if self.queue.iter().any(|r| r.track.id == spotify_id) {
            return Some(DownloadPhase::Queued);
        }
        None
    }

    pub fn active_progress_list(&self) -> Vec<(SpotifyTrack, DownloadProgress)> {
        self.active
            .iter()
            .map(|a| (a.request.track.clone(), a.progress.lock().unwrap().clone()))
            .collect()
    }

    /// For backward compat with UI expecting single active
    pub fn active_progress(&self) -> Option<(SpotifyTrack, DownloadProgress)> {
        self.active.first().map(|a| {
            (a.request.track.clone(), a.progress.lock().unwrap().clone())
        })
    }

    pub fn total_queued(&self) -> usize {
        self.queue.len() + self.active.len()
    }
}

pub struct DownloaderPlugin;

impl Plugin for DownloaderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DownloadManager>()
            .add_systems(Update, (process_download_queue, poll_active_downloads));
    }
}

fn process_download_queue(
    mut manager: ResMut<DownloadManager>,
    config: Res<AppConfig>,
) {
    while manager.active.len() < MAX_CONCURRENT && !manager.queue.is_empty() {
        let request = manager.queue.pop_front().unwrap();
        let progress = Arc::new(Mutex::new(DownloadProgress {
            phase: DownloadPhase::SearchingYoutube,
            percent: 0.0,
            error: None,
        }));

        let progress_clone = Arc::clone(&progress);
        let track = request.track.clone();
        let music_folder = config.last_folder.clone();
        let audio_format = config.download_format().to_string();
        let timeout_secs = config.download_timeout();

        eprintln!(
            "[downloader] Starting download: {} - {} (active: {}, queued: {}, format: {audio_format})",
            track.artists.join(", "),
            track.name,
            manager.active.len() + 1,
            manager.queue.len(),
        );

        let thread = std::thread::spawn(move || {
            download_track(track, progress_clone, music_folder, audio_format, timeout_secs)
        });

        manager.active.push(ActiveDownload {
            request,
            progress,
            thread: Some(thread),
        });
    }
}

fn poll_active_downloads(mut manager: ResMut<DownloadManager>) {
    let mut i = 0;
    while i < manager.active.len() {
        let is_done = {
            let p = manager.active[i].progress.lock().unwrap();
            matches!(p.phase, DownloadPhase::Done | DownloadPhase::Failed)
        };

        if !is_done {
            i += 1;
            continue;
        }

        let mut active = manager.active.remove(i);
        let track = active.request.track.clone();

        if let Some(handle) = active.thread.take() {
            match handle.join() {
                Ok(Ok(path)) => {
                    eprintln!(
                        "[downloader] Completed: {} - {} -> {}",
                        track.artists.join(", "),
                        track.name,
                        path.display()
                    );
                    manager.completed.push(CompletedDownload { track, path });
                }
                Ok(Err(err)) => {
                    eprintln!(
                        "[downloader] Failed: {} - {} — {}",
                        track.artists.join(", "),
                        track.name,
                        err
                    );
                    manager.failed.push((track, err));
                }
                Err(_) => {
                    eprintln!("[downloader] Thread panicked for: {} - {}", track.artists.join(", "), track.name);
                    manager.failed.push((track, "Thread panicked".to_string()));
                }
            }
        }
        // don't increment i — we removed an element
    }
}

/// Determine output path: music_folder/Album/Artist - Title.opus
fn output_path_for(track: &SpotifyTrack, music_folder: Option<&PathBuf>, config_format: Option<&str>) -> PathBuf {
    let base = match music_folder {
        Some(folder) if folder.is_dir() => folder.clone(),
        _ => {
            let fallback = dirs::home_dir()
                .expect("could not find home directory")
                .join(".nightingale")
                .join("downloads");
            let _ = std::fs::create_dir_all(&fallback);
            fallback
        }
    };

    // Sanitize names for filesystem
    let album = sanitize_filename(&track.album_name);
    let artist = track.artists.first().cloned().unwrap_or_else(|| "Unknown".to_string());
    let filename = sanitize_filename(&format!("{} - {}", artist, track.name));

    let dir = base.join(&album);
    let _ = std::fs::create_dir_all(&dir);

    let format = config_format.unwrap_or("flac");
    dir.join(format!("{filename}.{format}"))
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn download_track(
    track: SpotifyTrack,
    progress: Arc<Mutex<DownloadProgress>>,
    music_folder: Option<PathBuf>,
    audio_format: String,
    timeout_secs: u64,
) -> Result<PathBuf, String> {
    let output_path = output_path_for(&track, music_folder.as_ref(), Some(&audio_format));

    eprintln!(
        "[downloader] Output path: {}",
        output_path.display()
    );

    // Check if already downloaded
    if output_path.is_file() {
        eprintln!("[downloader] Already exists, skipping: {}", output_path.display());
        let mut p = progress.lock().unwrap();
        p.phase = DownloadPhase::Done;
        p.percent = 100.0;
        return Ok(output_path);
    }

    // Ensure yt-dlp is available
    {
        let mut p = progress.lock().unwrap();
        p.phase = DownloadPhase::FetchingYtdlp;
    }
    eprintln!("[downloader] Ensuring yt-dlp is available...");
    ytdlp::ensure_ytdlp()?;
    eprintln!("[downloader] yt-dlp ready");

    // Search YouTube
    {
        let mut p = progress.lock().unwrap();
        p.phase = DownloadPhase::SearchingYoutube;
        p.percent = 0.0;
    }

    let search_query = format!(
        "{} - {} audio",
        track.artists.first().unwrap_or(&"Unknown".to_string()),
        track.name,
    );
    eprintln!("[downloader] Searching YouTube: \"{}\"", search_query);
    let youtube_url = ytdlp::search_youtube(&search_query, track.duration_ms)?;
    eprintln!("[downloader] Found: {youtube_url}");

    // Download
    {
        let mut p = progress.lock().unwrap();
        p.phase = DownloadPhase::Downloading;
        p.percent = 0.0;
    }

    eprintln!("[downloader] Downloading audio (format={audio_format}, timeout={timeout_secs}s)...");
    let progress_clone = Arc::clone(&progress);
    ytdlp::download_audio(&youtube_url, &output_path, &audio_format, timeout_secs, move |pct| {
        let mut p = progress_clone.lock().unwrap();
        p.percent = pct;
        if pct as u32 % 25 == 0 {
            eprintln!("[downloader] Progress: {pct:.0}%");
        }
    })?;
    eprintln!("[downloader] Download complete: {}", output_path.display());

    // Tag metadata
    {
        let mut p = progress.lock().unwrap();
        p.phase = DownloadPhase::Tagging;
        p.percent = 95.0;
    }

    eprintln!("[downloader] Tagging metadata...");
    if let Err(e) = tagger::tag_file(&output_path, &track) {
        eprintln!("[downloader] Tagging failed (non-fatal): {e}");
    }

    // Done
    {
        let mut p = progress.lock().unwrap();
        p.phase = DownloadPhase::Done;
        p.percent = 100.0;
    }

    // Save metadata sidecar next to the file
    let meta_path = output_path.with_extension("json");
    let meta_json = serde_json::json!({
        "id": track.id,
        "name": track.name,
        "artists": track.artists,
        "album_name": track.album_name,
        "album_id": track.album_id,
        "album_art_url": track.album_art_url,
        "duration_ms": track.duration_ms,
        "track_number": track.track_number,
    });
    let _ = std::fs::write(&meta_path, serde_json::to_string_pretty(&meta_json).unwrap());

    eprintln!(
        "[downloader] Done: {} - {} -> {}",
        track.artists.join(", "),
        track.name,
        output_path.display()
    );

    Ok(output_path)
}
