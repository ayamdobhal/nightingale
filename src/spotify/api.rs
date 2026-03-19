use std::time::{Duration, Instant};

use serde::Deserialize;

const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const API_BASE: &str = "https://api.spotify.com/v1";

// Default credentials (client credentials flow — no user data exposed)
const DEFAULT_CLIENT_ID: &str = "REDACTED_CLIENT_ID";
const DEFAULT_CLIENT_SECRET: &str = "REDACTED_CLIENT_SECRET";

#[derive(Debug, Clone)]
pub struct SpotifyTrack {
    pub id: String,
    pub name: String,
    pub artists: Vec<String>,
    pub album_name: String,
    pub album_id: String,
    pub album_art_url: Option<String>,
    pub duration_ms: u64,
    pub track_number: u32,
}

#[derive(Debug, Clone)]
pub struct SpotifyAlbum {
    pub id: String,
    pub name: String,
    pub artists: Vec<String>,
    pub art_url: Option<String>,
    pub total_tracks: u32,
    pub release_date: String,
}

pub struct SpotifyClient {
    client_id: String,
    client_secret: String,
    access_token: Option<String>,
    expires_at: Option<Instant>,
    agent: ureq::Agent,
}

impl SpotifyClient {
    pub fn new(client_id: Option<&str>, client_secret: Option<&str>) -> Self {
        Self {
            client_id: client_id.unwrap_or(DEFAULT_CLIENT_ID).to_string(),
            client_secret: client_secret.unwrap_or(DEFAULT_CLIENT_SECRET).to_string(),
            access_token: None,
            expires_at: None,
            agent: ureq::Agent::new_with_defaults(),
        }
    }

    fn ensure_token(&mut self) -> Result<(), String> {
        if let (Some(_token), Some(expires)) = (&self.access_token, self.expires_at) {
            if Instant::now() < expires {
                return Ok(());
            }
        }
        self.refresh_token()
    }

    fn refresh_token(&mut self) -> Result<(), String> {
        let credentials = format!("{}:{}", self.client_id, self.client_secret);
        let encoded = base64_encode(credentials.as_bytes());

        let body = format!("grant_type=client_credentials");
        let resp = self
            .agent
            .post(TOKEN_URL)
            .header("Authorization", &format!("Basic {encoded}"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(body.as_bytes())
            .map_err(|e| format!("Token request failed: {e}"))?;

        let token_resp: TokenResponse = resp
            .into_body()
            .read_json()
            .map_err(|e| format!("Failed to parse token: {e}"))?;

        self.access_token = Some(token_resp.access_token);
        self.expires_at = Some(Instant::now() + Duration::from_secs(token_resp.expires_in.saturating_sub(60)));

        Ok(())
    }

    fn get(&mut self, url: &str) -> Result<String, String> {
        self.ensure_token()?;
        let token = self.access_token.as_ref().unwrap();

        let resp = self
            .agent
            .get(url)
            .header("Authorization", &format!("Bearer {token}"))
            .call()
            .map_err(|e| format!("API request failed: {e}"))?;

        let body = resp
            .into_body()
            .read_to_string()
            .map_err(|e| format!("Failed to read response: {e}"))?;

        Ok(body)
    }

    pub fn search_tracks(&mut self, query: &str, limit: u8) -> Result<Vec<SpotifyTrack>, String> {
        let url = format!(
            "{API_BASE}/search?q={}&type=track&limit={limit}",
            urlencoding::encode(query),
        );
        let body = self.get(&url)?;
        let resp: SearchResponse =
            serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))?;

        let Some(tracks) = resp.tracks else {
            return Ok(vec![]);
        };

        Ok(tracks.items.into_iter().map(|t| api_track_to_spotify_track(t)).collect())
    }

    pub fn search_albums(&mut self, query: &str, limit: u8) -> Result<Vec<SpotifyAlbum>, String> {
        let url = format!(
            "{API_BASE}/search?q={}&type=album&limit={limit}",
            urlencoding::encode(query),
        );
        let body = self.get(&url)?;
        let resp: SearchResponse =
            serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))?;

        let Some(albums) = resp.albums else {
            return Ok(vec![]);
        };

        Ok(albums
            .items
            .into_iter()
            .map(|a| SpotifyAlbum {
                id: a.id,
                name: a.name,
                artists: a.artists.into_iter().map(|ar| ar.name).collect(),
                art_url: a.images.first().map(|img| img.url.clone()),
                total_tracks: a.total_tracks.unwrap_or(0),
                release_date: a.release_date.unwrap_or_default(),
            })
            .collect())
    }

    pub fn album_tracks(&mut self, album_id: &str) -> Result<Vec<SpotifyTrack>, String> {
        // First get album details for art
        let album_url = format!("{API_BASE}/albums/{album_id}");
        let album_body = self.get(&album_url)?;
        let album: ApiAlbum =
            serde_json::from_str(&album_body).map_err(|e| format!("Parse error: {e}"))?;

        let art_url = album.images.first().map(|img| img.url.clone());
        let album_name = album.name.clone();

        // Then get tracks (paginated)
        let mut all_tracks = Vec::new();
        let mut offset = 0u32;
        loop {
            let url = format!("{API_BASE}/albums/{album_id}/tracks?limit=50&offset={offset}");
            let body = self.get(&url)?;
            let page: ApiPage<ApiSimpleTrack> =
                serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))?;

            for t in &page.items {
                all_tracks.push(SpotifyTrack {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    artists: t.artists.iter().map(|a| a.name.clone()).collect(),
                    album_name: album_name.clone(),
                    album_id: album_id.to_string(),
                    album_art_url: art_url.clone(),
                    duration_ms: t.duration_ms,
                    track_number: t.track_number,
                });
            }

            if page.next.is_none() || page.items.is_empty() {
                break;
            }
            offset += 50;
        }

        Ok(all_tracks)
    }

    pub fn track(&mut self, track_id: &str) -> Result<SpotifyTrack, String> {
        let url = format!("{API_BASE}/tracks/{track_id}");
        let body = self.get(&url)?;
        let t: ApiTrack =
            serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))?;
        Ok(api_track_to_spotify_track(t))
    }
}

fn api_track_to_spotify_track(t: ApiTrack) -> SpotifyTrack {
    SpotifyTrack {
        id: t.id,
        name: t.name,
        artists: t.artists.into_iter().map(|a| a.name).collect(),
        album_name: t.album.name.clone(),
        album_id: t.album.id,
        album_art_url: t.album.images.first().map(|img| img.url.clone()),
        duration_ms: t.duration_ms,
        track_number: t.track_number,
    }
}

// --- Simple base64 (avoids adding a crate) ---

fn base64_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[(triple >> 18 & 0x3F) as usize] as char);
        out.push(CHARS[(triple >> 12 & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[(triple >> 6 & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

// --- API response types ---

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct SearchResponse {
    tracks: Option<ApiPage<ApiTrack>>,
    albums: Option<ApiPage<ApiAlbum>>,
}

#[derive(Deserialize)]
struct ApiPage<T> {
    items: Vec<T>,
    next: Option<String>,
}

#[derive(Deserialize)]
struct ApiTrack {
    id: String,
    name: String,
    artists: Vec<ApiArtist>,
    album: ApiAlbum,
    duration_ms: u64,
    track_number: u32,
}

#[derive(Deserialize)]
struct ApiSimpleTrack {
    id: String,
    name: String,
    artists: Vec<ApiArtist>,
    duration_ms: u64,
    track_number: u32,
}

#[derive(Deserialize)]
struct ApiAlbum {
    id: String,
    name: String,
    artists: Vec<ApiArtist>,
    images: Vec<ApiImage>,
    total_tracks: Option<u32>,
    release_date: Option<String>,
}

#[derive(Deserialize)]
struct ApiArtist {
    name: String,
}

#[derive(Deserialize)]
struct ApiImage {
    url: String,
}
