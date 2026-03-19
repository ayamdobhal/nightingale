# Spotify Search + yt-dlp Download — Implementation Plan

## Overview

Add the ability to search for tracks/albums via Spotify's Web API, then download them using yt-dlp with metadata from Spotify. Includes a download queue UI integrated into the existing menu.

---

## Part 1: Spotify API Client (no OAuth needed for search)

Spotify's **search endpoint** only needs a client credentials token (no user login). This means we can skip the full OAuth PKCE flow for now and just use client_id + client_secret to get an app token.

### 1.1 Client Credentials Auth

**File:** `src/spotify/mod.rs` (new module)

```rust
pub mod api;
```

**File:** `src/spotify/api.rs`

- `SpotifyClient` struct with `access_token: String`, `expires_at: Instant`
- `SpotifyClient::new(client_id, client_secret)` → POST `https://accounts.spotify.com/api/token` with `grant_type=client_credentials`
- Auto-refresh when token expires (tokens last 1 hour)
- Uses `ureq` (already a dependency)

### 1.2 Search API

Methods on `SpotifyClient`:

```rust
fn search_tracks(&self, query: &str, limit: u8) -> Vec<SpotifyTrack>
fn search_albums(&self, query: &str, limit: u8) -> Vec<SpotifyAlbum>
fn album_tracks(&self, album_id: &str) -> Vec<SpotifyTrack>
```

### 1.3 Data Types

```rust
struct SpotifyTrack {
    id: String,
    name: String,
    artists: Vec<String>,
    album_name: String,
    album_id: String,
    album_art_url: Option<String>,  // 640px image
    duration_ms: u64,
    track_number: u32,
}

struct SpotifyAlbum {
    id: String,
    name: String,
    artists: Vec<String>,
    art_url: Option<String>,
    total_tracks: u32,
    release_date: String,
}
```

### 1.4 Config

**File:** `src/config.rs`

Add:
```rust
pub spotify_client_id: Option<String>,
pub spotify_client_secret: Option<String>,
```

Default: read from env vars `SPOTIFY_CLIENT_ID` / `SPOTIFY_CLIENT_SECRET`, or from config.json.

Client ID: `REDACTED_CLIENT_ID`
Client Secret: `REDACTED_CLIENT_SECRET`

These can be hardcoded as defaults or shipped in a `.env` — client credentials don't expose user data.

---

## Part 2: yt-dlp Integration

### 2.1 yt-dlp Binary Vendor

**File:** `src/vendor.rs` (extend)

- Add `ytdlp_path() -> PathBuf` → `~/.nightingale/vendor/yt-dlp[.exe]`
- Add `download_ytdlp()` function:
  - GitHub releases: `https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp[.exe]`
  - Single binary, no deps
  - Download on first use (not during setup — keep setup fast)
  - Mark executable on unix (`chmod +x`)
- Add to `is_ready()` check? **No** — keep it optional. Check at download time.

### 2.2 Download Engine

**File:** `src/downloader/mod.rs` (new module)

```rust
pub mod ytdlp;
pub mod tagger;
```

**File:** `src/downloader/ytdlp.rs`

**Search YouTube for a Spotify track:**
```
yt-dlp --dump-json "ytsearch5:{artist} - {title}" --no-download
```
- Parse JSON output (one object per result)
- Pick best match by:
  1. Duration within ±15s of Spotify duration
  2. Prefer titles containing "audio", "official audio", "lyrics"
  3. Prefer channels matching artist name
  4. Reject titles containing "live", "remix", "cover" (unless the Spotify title does too)

**Download:**
```
yt-dlp -x --audio-format opus --audio-quality 0 \
  --no-playlist --no-warnings \
  --progress --newline \
  -o "{output_path}" "{youtube_url}"
```
- Parse progress lines: `[download]  45.2% of 5.23MiB at 1.2MiB/s ETA 00:03`
- Extract percentage for UI progress bar

**File:** `src/downloader/tagger.rs`

After download, tag with `lofty`:
- Title, artist, album, album artist, track number, year
- Embed album art (download from Spotify `album_art_url`)
- Write to the opus/mp3 file

### 2.3 Download Cache

**Dir:** `~/.nightingale/downloads/`

- Files: `{spotify_track_id}.opus`
- Sidecar: `{spotify_track_id}.json` (Spotify metadata for re-tagging)
- On search results, check if already downloaded → show as "Ready" instead of download button

---

## Part 3: Download Manager (Bevy Resource)

**File:** `src/downloader/mod.rs`

```rust
#[derive(Resource)]
pub struct DownloadManager {
    queue: VecDeque<DownloadRequest>,
    active: Option<ActiveDownload>,
    completed: Vec<CompletedDownload>,
    failed: Vec<(DownloadRequest, String)>,
}

struct DownloadRequest {
    track: SpotifyTrack,
}

struct ActiveDownload {
    track: SpotifyTrack,
    progress: Arc<Mutex<DownloadProgress>>,
    thread: Option<JoinHandle<()>>,
}

struct DownloadProgress {
    phase: DownloadPhase,
    percent: f32,
    error: Option<String>,
}

enum DownloadPhase {
    SearchingYoutube,
    Downloading,
    Converting,
    Tagging,
    AddingToLibrary,
    Done,
    Failed,
}

struct CompletedDownload {
    track: SpotifyTrack,
    path: PathBuf,
}
```

**Systems:**
- `process_download_queue` — picks next from queue, spawns background thread
- `poll_active_download` — checks progress, updates UI, moves to completed on done
- `auto_analyze_completed` — when a download finishes, add to `SongLibrary` and enqueue in `AnalysisQueue`

**Download thread flow:**
1. Ensure yt-dlp binary exists (download if not)
2. Search YouTube → pick best match
3. Download audio
4. Tag with Spotify metadata + album art
5. Signal done

---

## Part 4: Search & Download UI

### 4.1 Search Overlay

**File:** `src/menu/spotify_search.rs` (new)

Triggered by a new sidebar button: **"Search Spotify"** (or a search icon next to existing search bar).

**Layout:**
```
┌─────────────────────────────────────┐
│  SEARCH SPOTIFY                  [X]│
│  ┌─────────────────────────────────┐│
│  │ Search: [________________]      ││
│  │ [Tracks] [Albums]               ││
│  └─────────────────────────────────┘│
│                                     │
│  ┌─ Results ───────────────────────┐│
│  │ ♫ Song Title - Artist    [DL]  ││
│  │   Album Name · 3:45            ││
│  │                                 ││
│  │ ♫ Song Title 2 - Artist  [DL]  ││
│  │   Album Name · 4:12            ││
│  │                                 ││
│  │ 📀 Album Name - Artist  [DL]  ││
│  │   12 tracks · 2024             ││
│  └─────────────────────────────────┘│
│                                     │
│  ┌─ Downloads ─────────────────────┐│
│  │ ↓ Downloading: Song - Artist    ││
│  │   [████████░░░░] 67%           ││
│  │ ✓ Song 2 - Artist (Ready)      ││
│  │ ⏳ Song 3 - Artist (Queued)     ││
│  └─────────────────────────────────┘│
└─────────────────────────────────────┘
```

### 4.2 Components

```rust
#[derive(Component)]
struct SpotifySearchOverlay;

#[derive(Component)]
struct SpotifySearchInput;

#[derive(Component)]
struct SpotifySearchTab(SearchTab);  // Tracks or Albums

#[derive(Component)]
struct TrackDownloadButton { track_index: usize }

#[derive(Component)]
struct AlbumDownloadButton { album_index: usize }  // downloads all tracks

#[derive(Component)]
struct DownloadProgressBar;

#[derive(Component)]
struct DownloadStatusText;
```

### 4.3 Interaction Flow

1. User clicks "Search Spotify" in sidebar (or keyboard shortcut)
2. Overlay spawns with search input focused
3. User types query → debounced search (500ms after last keystroke)
4. Results appear as cards (track or album)
5. Click [DL] on a track → adds to download queue
6. Click [DL] on an album → fetches album tracks → adds all to queue
7. Download progress section shows at bottom of overlay
8. When download completes → auto-added to library → auto-queued for analysis
9. Song appears in main library list with a badge indicating source

### 4.4 Sidebar Integration

Add to `SidebarAction` enum:
```rust
SpotifySearch,
```

New button in sidebar: "Search Spotify" (below Change Folder).

### 4.5 Keyboard Navigation

| Action | Key |
|---|---|
| Open Spotify Search | Ctrl+F or sidebar button |
| Close overlay | Escape |
| Navigate results | Arrow keys |
| Download selected | Enter |
| Switch Tracks/Albums tab | Tab |

---

## Part 5: Library Integration

### 5.1 Song Source

Extend `Song` in `src/scanner/metadata.rs`:
```rust
pub enum SongSource {
    Local,
    Downloaded { spotify_id: String },
}

// Add to Song struct:
pub source: SongSource,
```

Default: `SongSource::Local` (existing songs unaffected).

### 5.2 Downloads in Library

When a download completes:
1. Create a `Song` from the downloaded file (same metadata extraction as folder scan)
2. Set `source: SongSource::Downloaded { spotify_id }`
3. Push to `SongLibrary.songs`
4. Enqueue in `AnalysisQueue`
5. Trigger menu rebuild to show new song

### 5.3 Persistence

On startup, scan `~/.nightingale/downloads/` and load any `.opus`/`.mp3` files into the library alongside the folder-scanned songs. This means downloaded songs persist across sessions even without a music folder.

---

## File Structure

```
src/
├── spotify/
│   ├── mod.rs              # module re-exports
│   └── api.rs              # client credentials auth + search/album API
├── downloader/
│   ├── mod.rs              # DownloadManager resource + systems
│   ├── ytdlp.rs            # yt-dlp search + download wrapper
│   └── tagger.rs           # lofty metadata tagging + album art embed
└── menu/
    └── spotify_search.rs   # search overlay UI + download progress
```

---

## Implementation Order

| Step | What | Depends on |
|---|---|---|
| 1 | `src/spotify/api.rs` — auth + search | Nothing |
| 2 | `src/downloader/ytdlp.rs` — search + download | Nothing |
| 3 | `src/downloader/tagger.rs` — metadata tagging | Step 1 (Spotify types) |
| 4 | `src/downloader/mod.rs` — DownloadManager | Steps 1-3 |
| 5 | `src/menu/spotify_search.rs` — UI | Steps 1, 4 |
| 6 | Library integration | Steps 4, 5 |

Steps 1-3 are independent and can be built in parallel. Step 4 ties them together. Step 5 is the UI layer. Step 6 is the glue.

---

## Notes

- **No user OAuth needed** — client credentials flow gives us search + metadata. We only need user OAuth if we want to access their playlists/liked songs (Phase 2 of IMPL_PLAN.md).
- **yt-dlp is downloaded lazily** — not during initial setup, only when user first tries to download a song. Keeps first-run experience fast.
- **Album art from Spotify** is always better quality than YouTube thumbnails. Always prefer Spotify metadata.
- **Duration matching** is critical — without it, yt-dlp might grab a 10-minute live version instead of the 4-minute studio cut.
- **Download format** defaults to opus (best quality/size ratio). Can add mp3/flac option later in settings.
- **Thread safety** — all downloads happen on background threads. UI polls progress via `Arc<Mutex<DownloadProgress>>` (same pattern as `AnalysisQueue`).
