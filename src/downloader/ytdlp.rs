use std::io::{BufRead, BufReader, Read as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::vendor::silent_command;

fn ytdlp_dir() -> PathBuf {
    dirs::home_dir()
        .expect("could not find home directory")
        .join(".nightingale")
        .join("vendor")
        .join("yt-dlp")
}

pub fn ytdlp_path() -> PathBuf {
    let name = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };
    ytdlp_dir().join(name)
}

pub fn is_available() -> bool {
    ytdlp_path().is_file()
}

pub fn ensure_ytdlp() -> Result<(), String> {
    if is_available() {
        return Ok(());
    }
    download_ytdlp()
}

fn download_ytdlp() -> Result<(), String> {
    let dir = ytdlp_dir();
    let _ = std::fs::create_dir_all(&dir);

    let (url, filename) = if cfg!(target_os = "windows") {
        (
            "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe",
            "yt-dlp.exe",
        )
    } else if cfg!(target_os = "macos") {
        (
            "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos",
            "yt-dlp",
        )
    } else {
        (
            "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp",
            "yt-dlp",
        )
    };

    eprintln!("[yt-dlp] Downloading from {url}");

    let agent = ureq::Agent::new_with_defaults();
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("Failed to download yt-dlp: {e}"))?;

    let dest = dir.join(filename);
    let mut bytes = Vec::new();
    resp.into_body()
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Failed to read yt-dlp binary: {e}"))?;

    std::fs::write(&dest, &bytes).map_err(|e| format!("Failed to write yt-dlp: {e}"))?;

    // Make executable on unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }

    eprintln!("[yt-dlp] Installed to {}", dest.display());
    Ok(())
}

/// Search YouTube and return the best matching URL.
/// Uses duration matching to avoid grabbing wrong versions.
pub fn search_youtube(query: &str, expected_duration_ms: u64) -> Result<String, String> {
    let ytdlp = ytdlp_path();
    let search_term = format!("ytsearch5:{query}");

    let mut cmd = silent_command(&ytdlp);
    cmd.args([
        "--dump-json",
        "--no-download",
        "--no-warnings",
        "--flat-playlist",
        &search_term,
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run yt-dlp search: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("yt-dlp search failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected_secs = expected_duration_ms as f64 / 1000.0;

    #[derive(serde::Deserialize)]
    struct YtResult {
        url: Option<String>,
        webpage_url: Option<String>,
        id: Option<String>,
        title: Option<String>,
        duration: Option<f64>,
        channel: Option<String>,
    }

    let mut candidates: Vec<(f64, String, String)> = Vec::new(); // (score, url, title)

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(result) = serde_json::from_str::<YtResult>(line) else {
            continue;
        };

        let url = result
            .webpage_url
            .or(result.url)
            .or(result.id.map(|id| format!("https://www.youtube.com/watch?v={id}")))
            .unwrap_or_default();

        if url.is_empty() {
            continue;
        }

        let title = result.title.unwrap_or_default();
        let title_lower = title.to_lowercase();
        let duration = result.duration.unwrap_or(0.0);

        // Duration penalty: higher = worse match
        let duration_diff = (duration - expected_secs).abs();
        if duration_diff > 30.0 {
            continue; // Skip if way off
        }
        let duration_score = duration_diff;

        // Prefer "official audio", "audio" in title
        let audio_bonus = if title_lower.contains("official audio") {
            -20.0
        } else if title_lower.contains("audio") {
            -10.0
        } else if title_lower.contains("lyrics") || title_lower.contains("lyric") {
            -5.0
        } else {
            0.0
        };

        // Penalize live, remix, cover (unless query contains them)
        let query_lower = query.to_lowercase();
        let live_penalty = if title_lower.contains("live") && !query_lower.contains("live") {
            15.0
        } else {
            0.0
        };
        let remix_penalty = if title_lower.contains("remix") && !query_lower.contains("remix") {
            20.0
        } else {
            0.0
        };
        let cover_penalty = if title_lower.contains("cover") && !query_lower.contains("cover") {
            25.0
        } else {
            0.0
        };

        let total_score = duration_score + audio_bonus + live_penalty + remix_penalty + cover_penalty;
        candidates.push((total_score, url, title));
    }

    if candidates.is_empty() {
        return Err(format!("No YouTube results found for: {query}"));
    }

    candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let (score, url, title) = &candidates[0];
    eprintln!("[yt-dlp] Best match: \"{title}\" (score={score:.1}) → {url}");

    Ok(url.clone())
}

/// Download audio from a YouTube URL as opus.
/// Calls `on_progress(percent)` as download progresses.
pub fn download_audio(
    url: &str,
    output_path: &Path,
    on_progress: impl Fn(f32) + Send,
) -> Result<(), String> {
    let ytdlp = ytdlp_path();
    let ffmpeg = crate::vendor::ffmpeg_path();
    let ffmpeg_dir = ffmpeg.parent().unwrap_or(std::path::Path::new("."));

    let output_str = output_path.to_string_lossy();

    // Remove extension — yt-dlp adds it based on --audio-format
    let output_stem = output_str.trim_end_matches(".opus");

    let mut cmd = silent_command(&ytdlp);
    cmd.args([
        "-x",
        "--audio-format", "opus",
        "--audio-quality", "0",
        "--no-playlist",
        "--no-warnings",
        "--progress",
        "--newline",
        "--ffmpeg-location", &ffmpeg_dir.to_string_lossy(),
        "-o", &format!("{output_stem}.%(ext)s"),
        url,
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start yt-dlp download: {e}"))?;

    // Read stdout for progress
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines().flatten() {
            if let Some(pct) = parse_download_progress(&line) {
                on_progress(pct);
            }
        }
    }

    let status = child.wait().map_err(|e| format!("yt-dlp wait failed: {e}"))?;

    if !status.success() {
        return Err("yt-dlp download failed".to_string());
    }

    // yt-dlp might create the file with a slightly different name; check
    if !output_path.is_file() {
        // Try common variations
        let webm_path = PathBuf::from(format!("{output_stem}.webm"));
        let m4a_path = PathBuf::from(format!("{output_stem}.m4a"));

        for alt in [&webm_path, &m4a_path] {
            if alt.is_file() {
                let _ = std::fs::rename(alt, output_path);
                break;
            }
        }
    }

    if !output_path.is_file() {
        return Err(format!("Expected output file not found: {}", output_path.display()));
    }

    Ok(())
}

/// Parse yt-dlp progress line like: `[download]  45.2% of 5.23MiB at 1.2MiB/s ETA 00:03`
fn parse_download_progress(line: &str) -> Option<f32> {
    if !line.contains("[download]") || !line.contains('%') {
        return None;
    }
    // Find the percentage
    let after_bracket = line.split("[download]").nth(1)?;
    let pct_str = after_bracket.trim().split('%').next()?.trim();
    pct_str.parse::<f32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_progress() {
        assert_eq!(
            parse_download_progress("[download]  45.2% of 5.23MiB at 1.2MiB/s ETA 00:03"),
            Some(45.2)
        );
        assert_eq!(
            parse_download_progress("[download] 100% of 5.23MiB in 00:03"),
            Some(100.0)
        );
        assert_eq!(parse_download_progress("some other line"), None);
    }
}
