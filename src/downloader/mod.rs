pub mod ytdlp;
pub mod tagger;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;

use crate::spotify::api::SpotifyTrack;

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
    pub active: Option<ActiveDownload>,
    pub completed: Vec<CompletedDownload>,
    pub failed: Vec<(SpotifyTrack, String)>,
}

impl DownloadManager {
    pub fn enqueue(&mut self, track: SpotifyTrack) {
        // Don't add duplicates
        if self.is_queued_or_active(&track.id) || self.is_completed(&track.id) {
            return;
        }
        self.queue.push_back(DownloadRequest { track });
    }

    pub fn enqueue_all(&mut self, tracks: Vec<SpotifyTrack>) {
        for track in tracks {
            self.enqueue(track);
        }
    }

    pub fn is_queued_or_active(&self, spotify_id: &str) -> bool {
        self.queue.iter().any(|r| r.track.id == spotify_id)
            || self
                .active
                .as_ref()
                .is_some_and(|a| a.request.track.id == spotify_id)
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
        if let Some(ref active) = self.active {
            if active.request.track.id == spotify_id {
                return Some(active.progress.lock().unwrap().phase.clone());
            }
        }
        if self.queue.iter().any(|r| r.track.id == spotify_id) {
            return Some(DownloadPhase::Queued);
        }
        None
    }

    pub fn active_progress(&self) -> Option<(SpotifyTrack, DownloadProgress)> {
        self.active.as_ref().map(|a| {
            (a.request.track.clone(), a.progress.lock().unwrap().clone())
        })
    }

    pub fn total_queued(&self) -> usize {
        self.queue.len() + if self.active.is_some() { 1 } else { 0 }
    }
}

pub fn downloads_dir() -> PathBuf {
    dirs::home_dir()
        .expect("could not find home directory")
        .join(".nightingale")
        .join("downloads")
}

pub struct DownloaderPlugin;

impl Plugin for DownloaderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DownloadManager>()
            .add_systems(Update, (process_download_queue, poll_active_download));
    }
}

fn process_download_queue(mut manager: ResMut<DownloadManager>) {
    if manager.active.is_some() || manager.queue.is_empty() {
        return;
    }

    let request = manager.queue.pop_front().unwrap();
    let progress = Arc::new(Mutex::new(DownloadProgress {
        phase: DownloadPhase::SearchingYoutube,
        percent: 0.0,
        error: None,
    }));

    let progress_clone = Arc::clone(&progress);
    let track = request.track.clone();

    let thread = std::thread::spawn(move || {
        download_track(track, progress_clone)
    });

    manager.active = Some(ActiveDownload {
        request,
        progress,
        thread: Some(thread),
    });
}

fn poll_active_download(mut manager: ResMut<DownloadManager>) {
    let is_done = {
        let Some(ref active) = manager.active else {
            return;
        };
        let p = active.progress.lock().unwrap();
        matches!(p.phase, DownloadPhase::Done | DownloadPhase::Failed)
    };

    if !is_done {
        return;
    }

    let mut active = manager.active.take().unwrap();
    let track = active.request.track.clone();

    if let Some(handle) = active.thread.take() {
        match handle.join() {
            Ok(Ok(path)) => {
                eprintln!("[downloader] Completed: {} - {}", track.artists.join(", "), track.name);
                manager.completed.push(CompletedDownload { track, path });
            }
            Ok(Err(err)) => {
                eprintln!("[downloader] Failed: {} - {} — {err}", track.artists.join(", "), track.name);
                manager.failed.push((track, err));
            }
            Err(_) => {
                manager.failed.push((track, "Thread panicked".to_string()));
            }
        }
    }
}

fn download_track(
    track: SpotifyTrack,
    progress: Arc<Mutex<DownloadProgress>>,
) -> Result<PathBuf, String> {
    let dl_dir = downloads_dir();
    let _ = std::fs::create_dir_all(&dl_dir);

    let output_path = dl_dir.join(format!("{}.opus", track.id));

    // Check if already downloaded
    if output_path.is_file() {
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
    ytdlp::ensure_ytdlp()?;

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
    let youtube_url = ytdlp::search_youtube(&search_query, track.duration_ms)?;

    // Download
    {
        let mut p = progress.lock().unwrap();
        p.phase = DownloadPhase::Downloading;
        p.percent = 0.0;
    }

    let progress_clone = Arc::clone(&progress);
    ytdlp::download_audio(&youtube_url, &output_path, move |pct| {
        let mut p = progress_clone.lock().unwrap();
        p.percent = pct;
    })?;

    // Tag metadata
    {
        let mut p = progress.lock().unwrap();
        p.phase = DownloadPhase::Tagging;
        p.percent = 95.0;
    }

    if let Err(e) = tagger::tag_file(&output_path, &track) {
        eprintln!("[downloader] Tagging failed (non-fatal): {e}");
    }

    // Done
    {
        let mut p = progress.lock().unwrap();
        p.phase = DownloadPhase::Done;
        p.percent = 100.0;
    }

    // Save metadata sidecar
    let meta_path = dl_dir.join(format!("{}.json", track.id));
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

    Ok(output_path)
}
