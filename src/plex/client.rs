//! Plex API client.
//!
//! Handles all communication with the Plex Media Server.

use super::auth::PlexClientInfo;
use super::constants::*;
use super::error::ApiError;
use super::models::*;
use reqwest::Client;
use std::time::Duration;

/// Plex API client.
#[derive(Clone)]
pub struct PlexClient {
    http: Client,
    client_info: PlexClientInfo,
    auth_token: Option<String>,
    server_url: Option<String>,
}

impl PlexClient {
    /// Create a new PlexClient.
    pub fn new(client_info: PlexClientInfo) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            http,
            client_info,
            auth_token: None,
            server_url: None,
        }
    }

    /// Create a new PlexClient with server URL, optional token, and client_identifier.
    /// Used for background tasks that need their own client instance.
    ///
    /// IMPORTANT: The client_identifier MUST match the one the token was issued for,
    /// otherwise Plex will reject requests with 400 errors. Always pass the
    /// client_identifier from auth.yaml, not a new random one.
    pub fn new_with_url(server_url: &str, token: Option<&str>, client_identifier: &str) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .expect("Failed to create HTTP client");

        let mut client_info = PlexClientInfo::default();
        client_info.client_identifier = client_identifier.to_string();

        Self {
            http,
            client_info,
            auth_token: token.map(|s| s.to_string()),
            server_url: Some(server_url.trim_end_matches('/').to_string()),
        }
    }

    /// Set the authentication token.
    pub fn set_auth_token(&mut self, token: String) {
        self.auth_token = Some(token);
    }

    /// Get the authentication token.
    pub fn token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }

    /// Set the server URL.
    pub fn set_server(&mut self, url: String) {
        self.server_url = Some(url.trim_end_matches('/').to_string());
    }

    /// Get the current server URL.
    pub fn server_url(&self) -> Option<&str> {
        self.server_url.as_deref()
    }

    /// Check if authenticated.
    pub fn is_authenticated(&self) -> bool {
        self.auth_token.is_some()
    }

    /// Check if server is set.
    pub fn has_server(&self) -> bool {
        self.server_url.is_some()
    }

    fn require_server(&self) -> Result<&str, ApiError> {
        self.server_url.as_deref().ok_or(ApiError::NoServerSelected)
    }

    fn require_token(&self) -> Result<&str, ApiError> {
        self.auth_token.as_deref().ok_or(ApiError::NotAuthenticated)
    }

    /// Build headers for Plex API requests.
    ///
    /// Returns an error if any header value contains invalid characters.
    fn build_headers(&self) -> Result<reqwest::header::HeaderMap, ApiError> {
        use reqwest::header::HeaderValue;

        let mut headers = reqwest::header::HeaderMap::new();

        headers.insert(
            HEADER_PLEX_PRODUCT,
            HeaderValue::from_str(&self.client_info.product)
                .map_err(|_| ApiError::InvalidHeader(HEADER_PLEX_PRODUCT.to_string()))?,
        );
        headers.insert(
            HEADER_PLEX_VERSION,
            HeaderValue::from_str(&self.client_info.version)
                .map_err(|_| ApiError::InvalidHeader(HEADER_PLEX_VERSION.to_string()))?,
        );
        headers.insert(
            HEADER_PLEX_CLIENT_ID,
            HeaderValue::from_str(&self.client_info.client_identifier)
                .map_err(|_| ApiError::InvalidHeader(HEADER_PLEX_CLIENT_ID.to_string()))?,
        );
        headers.insert(
            HEADER_PLEX_DEVICE_NAME,
            HeaderValue::from_str(&self.client_info.device_name)
                .map_err(|_| ApiError::InvalidHeader(HEADER_PLEX_DEVICE_NAME.to_string()))?,
        );
        headers.insert(
            HEADER_PLEX_PLATFORM,
            HeaderValue::from_str(&self.client_info.platform)
                .map_err(|_| ApiError::InvalidHeader(HEADER_PLEX_PLATFORM.to_string()))?,
        );
        headers.insert("Accept", HeaderValue::from_static("application/json"));

        if let Some(ref token) = self.auth_token {
            headers.insert(
                HEADER_PLEX_TOKEN,
                HeaderValue::from_str(token)
                    .map_err(|_| ApiError::InvalidHeader(HEADER_PLEX_TOKEN.to_string()))?,
            );
        }

        Ok(headers)
    }

    /// Build URL from server base and path.
    fn build_url(&self, path: &str) -> Result<String, ApiError> {
        Ok(format!("{}{}", self.require_server()?, path))
    }

    /// Make a GET request and return raw text (for debugging).
    pub async fn get_raw(&self, path: &str) -> Result<String, ApiError> {
        let url = self.build_url(path)?;
        let response = self
            .http
            .get(&url)
            .headers(self.build_headers()?)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(ApiError::ServerError {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }

        Ok(response.text().await?)
    }

    /// Make a GET request to the server.
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let url = self.build_url(path)?;

        tracing::debug!("GET {}", url);

        let response = self
            .http
            .get(&url)
            .headers(self.build_headers()?)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("HTTP request failed for URL '{}': {}", url, e);
                e
            })?;

        if !response.status().is_success() {
            return Err(ApiError::ServerError {
                status: response.status().as_u16(),
                message: response.text().await.unwrap_or_default(),
            });
        }

        let text = response.text().await?;
        tracing::trace!("Response: {}", &text[..text.len().min(1000)]);

        let data: T = serde_json::from_str(&text).map_err(|e| {
            tracing::error!("JSON parse error: {} - Response: {}", e, &text[..text.len().min(500)]);
            ApiError::ParseError(format!("JSON parse error: {}", e))
        })?;
        Ok(data)
    }

    // ========================================================================
    // Library Methods
    // ========================================================================

    /// Get all library sections.
    pub async fn get_libraries(&self) -> Result<Vec<Library>, ApiError> {
        let response: LibrarySectionsResponse = self.get(EP_LIBRARY_SECTIONS).await?;
        Ok(response.media_container.directory)
    }

    /// Get music libraries only.
    pub async fn get_music_libraries(&self) -> Result<Vec<Library>, ApiError> {
        let libraries = self.get_libraries().await?;
        Ok(libraries.into_iter().filter(|l| l.is_music()).collect())
    }

    // ========================================================================
    // Artist Methods
    // ========================================================================

    /// Get artists in a library section with pagination.
    pub async fn get_artists_page(
        &self,
        library_key: &str,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<Artist>, u32), ApiError> {
        let path = if limit > 0 {
            format!(
                "{}/{}/all?type={}&{}={}&{}={}",
                EP_LIBRARY_SECTIONS, library_key, TYPE_ARTIST,
                PARAM_CONTAINER_START, offset,
                PARAM_CONTAINER_SIZE, limit
            )
        } else {
            format!("{}/{}/all?type={}", EP_LIBRARY_SECTIONS, library_key, TYPE_ARTIST)
        };
        let response: ArtistsResponse = self.get(&path).await?;
        let total = response.media_container.total_size.unwrap_or(response.media_container.size);
        Ok((response.media_container.metadata, total))
    }

    /// Get all artists in a library section (loads everything - can be slow).
    pub async fn get_artists(&self, library_key: &str) -> Result<Vec<Artist>, ApiError> {
        let (artists, _) = self.get_artists_page(library_key, 0, 0).await?;
        Ok(artists)
    }

    /// Get a specific artist by rating key.
    pub async fn get_artist(&self, rating_key: &str) -> Result<Artist, ApiError> {
        let path = format!("{}/{}", EP_LIBRARY_METADATA, rating_key);
        let response: ArtistsResponse = self.get(&path).await?;
        response
            .media_container
            .metadata
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::ItemNotFound(rating_key.to_string()))
    }

    /// Get albums by an artist.
    pub async fn get_artist_albums(&self, artist_key: &str) -> Result<Vec<Album>, ApiError> {
        let path = format!("{}/{}/children", EP_LIBRARY_METADATA, artist_key);
        let response: AlbumsResponse = self.get(&path).await?;
        Ok(response.media_container.metadata)
    }

    /// Get all tracks by an artist (across all their albums).
    pub async fn get_artist_all_tracks(&self, artist_key: &str) -> Result<Vec<Track>, ApiError> {
        let path = format!("{}/{}/allLeaves", EP_LIBRARY_METADATA, artist_key);
        let response: TracksResponse = self.get(&path).await?;
        Ok(response.media_container.metadata)
    }

    // ========================================================================
    // Album Methods
    // ========================================================================

    /// Get albums in a library section with pagination.
    pub async fn get_albums_page(
        &self,
        library_key: &str,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<Album>, u32), ApiError> {
        let path = if limit > 0 {
            format!(
                "{}/{}/all?type={}&{}={}&{}={}",
                EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM,
                PARAM_CONTAINER_START, offset,
                PARAM_CONTAINER_SIZE, limit
            )
        } else {
            format!("{}/{}/all?type={}", EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM)
        };
        let response: AlbumsResponse = self.get(&path).await?;
        let total = response.media_container.total_size.unwrap_or(response.media_container.size);
        Ok((response.media_container.metadata, total))
    }

    /// Get all albums in a library section (loads everything - can be slow).
    pub async fn get_albums(&self, library_key: &str) -> Result<Vec<Album>, ApiError> {
        let (albums, _) = self.get_albums_page(library_key, 0, 0).await?;
        Ok(albums)
    }

    /// Get recently added albums.
    pub async fn get_recently_added_albums(
        &self,
        library_key: &str,
        limit: u32,
    ) -> Result<Vec<Album>, ApiError> {
        let path = format!(
            "{}/{}/recentlyAdded?type={}&{}={}",
            EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM, PARAM_CONTAINER_SIZE, limit
        );
        let response: AlbumsResponse = self.get(&path).await?;
        Ok(response.media_container.metadata)
    }

    /// Get recently played albums (sorted by last viewed time).
    pub async fn get_recently_played_albums(
        &self,
        library_key: &str,
        limit: u32,
    ) -> Result<Vec<Album>, ApiError> {
        // Use sort=lastViewedAt:desc to get albums ordered by when they were last played
        let path = format!(
            "{}/{}/all?type={}&sort=lastViewedAt:desc&{}={}",
            EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM, PARAM_CONTAINER_SIZE, limit
        );
        let response: AlbumsResponse = self.get(&path).await?;
        // Filter to only include albums that have actually been played (lastViewedAt > 0)
        let played_albums: Vec<Album> = response
            .media_container
            .metadata
            .into_iter()
            .filter(|a| a.last_viewed_at.map(|t| t > 0).unwrap_or(false))
            .collect();
        Ok(played_albums)
    }

    /// Get a specific album by rating key.
    pub async fn get_album(&self, rating_key: &str) -> Result<Album, ApiError> {
        let path = format!("{}/{}", EP_LIBRARY_METADATA, rating_key);
        let response: AlbumsResponse = self.get(&path).await?;
        response
            .media_container
            .metadata
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::ItemNotFound(rating_key.to_string()))
    }

    /// Get tracks on an album.
    pub async fn get_album_tracks(&self, album_key: &str) -> Result<Vec<Track>, ApiError> {
        let path = format!("{}/{}/children", EP_LIBRARY_METADATA, album_key);
        let response: TracksResponse = self.get(&path).await?;
        Ok(response.media_container.metadata)
    }

    // ========================================================================
    // Track Methods
    // ========================================================================

    /// Get all tracks in a library section.
    pub async fn get_tracks(&self, library_key: &str) -> Result<Vec<Track>, ApiError> {
        let path = format!("{}/{}/all?type={}", EP_LIBRARY_SECTIONS, library_key, TYPE_TRACK);
        let response: TracksResponse = self.get(&path).await?;
        Ok(response.media_container.metadata)
    }

    /// Get a specific track by rating key.
    pub async fn get_track(&self, rating_key: &str) -> Result<Track, ApiError> {
        let path = format!("{}/{}", EP_LIBRARY_METADATA, rating_key);
        let response: TracksResponse = self.get(&path).await?;
        response
            .media_container
            .metadata
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::ItemNotFound(rating_key.to_string()))
    }

    // ========================================================================
    // Playlist Methods
    // ========================================================================

    /// Get all playlists.
    ///
    /// Deduplicates smart playlists (like "❤️ Tracks", "Recently Added") which
    /// Plex creates once per music library with identical titles.
    pub async fn get_playlists(&self) -> Result<Vec<Playlist>, ApiError> {
        let response: PlaylistsResponse = self.get(EP_PLAYLISTS_AUDIO).await?;
        let mut playlists = response.media_container.metadata;

        // Deduplicate smart playlists by title (Plex creates one per library)
        let mut seen_smart_titles = std::collections::HashSet::new();
        playlists.retain(|p| {
            if p.smart {
                // Keep only the first smart playlist with each title
                seen_smart_titles.insert(p.title.clone())
            } else {
                // Keep all user-created playlists
                true
            }
        });

        Ok(playlists)
    }

    /// Get tracks in a playlist.
    pub async fn get_playlist_tracks(&self, playlist_key: &str) -> Result<Vec<Track>, ApiError> {
        let path = format!("{}/{}/items", EP_PLAYLISTS, playlist_key);
        let response: TracksResponse = self.get(&path).await?;
        Ok(response.media_container.metadata)
    }

    /// Create a new playlist from track keys.
    pub async fn create_playlist(
        &self,
        title: &str,
        track_keys: &[String],
        _library_key: &str,
    ) -> Result<(), ApiError> {
        let server = self.require_server()?;
        let token = self.require_token()?;

        let uri = track_keys
            .iter()
            .map(|key| {
                format!(
                    "server://{}/com.plexapp.plugins.library{}/{}",
                    self.client_info.client_identifier, EP_LIBRARY_METADATA, key
                )
            })
            .collect::<Vec<_>>()
            .join(",");

        let path = format!(
            "{}?type=audio&title={}&smart=0&uri={}&{}={}",
            EP_PLAYLISTS,
            urlencoding::encode(title),
            urlencoding::encode(&uri),
            HEADER_PLEX_TOKEN, token
        );

        let url = format!("{}{}", server, path);
        tracing::debug!("Creating playlist: POST {}", url);

        let response = self
            .http
            .post(&url)
            .headers(self.build_headers()?)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            tracing::error!("Failed to create playlist: {} - {}", status, message);
            return Err(ApiError::ServerError { status, message });
        }

        Ok(())
    }

    // ========================================================================
    // Genre Methods
    // ========================================================================

    /// Get all genres for a music library.
    pub async fn get_genres(&self, library_key: &str) -> Result<Vec<Genre>, ApiError> {
        use super::models::GenresResponse;
        let path = format!("{}/{}/genre?type={}", EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM);
        tracing::info!("get_genres: fetching from {}", path);
        let response: GenresResponse = self.get(&path).await?;
        tracing::info!("get_genres: found {} genres", response.media_container.directory.len());
        if let Some(first) = response.media_container.directory.first() {
            tracing::info!("get_genres: first genre key='{}', title='{}'", first.key, first.title);
        }
        Ok(response.media_container.directory)
    }

    /// Get tracks in a specific genre.
    pub async fn get_genre_tracks(
        &self,
        library_key: &str,
        genre_filter: &str,
    ) -> Result<Vec<Track>, ApiError> {
        tracing::info!("get_genre_tracks: library_key={}, genre_filter={}", library_key, genre_filter);

        if genre_filter.is_empty() {
            return Err(ApiError::ParseError("Empty genre filter".to_string()));
        }

        let path = if genre_filter.contains("genre=") {
            let genre_id = genre_filter
                .split("genre=")
                .nth(1)
                .and_then(|s| s.split('&').next())
                .unwrap_or(genre_filter);
            tracing::info!("get_genre_tracks: extracted genre_id={}", genre_id);
            format!("{}/{}/all?type={}&genre={}", EP_LIBRARY_SECTIONS, library_key, TYPE_TRACK, genre_id)
        } else if genre_filter.starts_with('/') {
            if genre_filter.contains("type=") {
                genre_filter.to_string()
            } else if genre_filter.contains('?') {
                format!("{}&type={}", genre_filter, TYPE_TRACK)
            } else {
                format!("{}?type={}", genre_filter, TYPE_TRACK)
            }
        } else {
            let encoded = urlencoding::encode(genre_filter);
            tracing::info!("get_genre_tracks: using genre_id={} (encoded={})", genre_filter, encoded);
            format!("{}/{}/all?type={}&genre={}", EP_LIBRARY_SECTIONS, library_key, TYPE_TRACK, encoded)
        };

        tracing::info!("get_genre_tracks: constructed path={}", path);
        let response: TracksResponse = self.get(&path).await?;
        Ok(response.media_container.metadata)
    }

    /// Get albums in a specific genre.
    pub async fn get_genre_albums(
        &self,
        library_key: &str,
        genre_filter: &str,
    ) -> Result<Vec<Album>, ApiError> {
        tracing::info!("get_genre_albums: library_key={}, genre_filter={}", library_key, genre_filter);

        if genre_filter.is_empty() {
            return Err(ApiError::ParseError("Empty genre filter".to_string()));
        }

        let genre_id = Self::extract_filter_id(genre_filter, "genre=");
        let encoded = urlencoding::encode(genre_id);
        let path = format!("{}/{}/all?type={}&genre={}", EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM, encoded);
        tracing::info!("get_genre_albums: fetching from {}", path);
        let response: AlbumsResponse = self.get(&path).await?;
        Ok(response.media_container.metadata)
    }

    // ========================================================================
    // Artist Genre Methods
    // ========================================================================

    /// Get Plex genres at artist level.
    pub async fn get_artist_genres(&self, library_key: &str) -> Result<Vec<Genre>, ApiError> {
        use super::models::GenresResponse;
        let path = format!("{}/{}/genre?type={}", EP_LIBRARY_SECTIONS, library_key, TYPE_ARTIST);
        tracing::info!("get_artist_genres: fetching from {}", path);
        let response: GenresResponse = self.get(&path).await?;
        tracing::info!("get_artist_genres: found {} genres", response.media_container.directory.len());
        Ok(response.media_container.directory)
    }

    /// Get albums in a specific artist genre.
    pub async fn get_artist_genre_albums(
        &self,
        library_key: &str,
        genre_filter: &str,
    ) -> Result<Vec<Album>, ApiError> {
        tracing::info!("get_artist_genre_albums: library_key={}, genre_filter={}", library_key, genre_filter);

        if genre_filter.is_empty() {
            return Err(ApiError::ParseError("Empty genre filter".to_string()));
        }

        let genre_id = Self::extract_filter_id(genre_filter, "genre=");
        let encoded = urlencoding::encode(genre_id);
        let path = format!("{}/{}/all?type={}&genre={}", EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM, encoded);
        tracing::info!("get_artist_genre_albums: fetching from {}", path);
        let response: AlbumsResponse = self.get(&path).await?;
        Ok(response.media_container.metadata)
    }

    // ========================================================================
    // Album Genre Methods
    // ========================================================================

    /// Get Plex genres at album level.
    pub async fn get_album_genres(&self, library_key: &str) -> Result<Vec<Genre>, ApiError> {
        use super::models::GenresResponse;
        let path = format!("{}/{}/genre?type={}", EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM);
        tracing::info!("get_album_genres: fetching from {}", path);
        let response: GenresResponse = self.get(&path).await?;
        tracing::info!("get_album_genres: found {} genres", response.media_container.directory.len());
        Ok(response.media_container.directory)
    }

    /// Get albums in a specific album genre.
    pub async fn get_album_genre_albums(
        &self,
        library_key: &str,
        genre_filter: &str,
    ) -> Result<Vec<Album>, ApiError> {
        tracing::info!("get_album_genre_albums: library_key={}, genre_filter={}", library_key, genre_filter);

        if genre_filter.is_empty() {
            return Err(ApiError::ParseError("Empty genre filter".to_string()));
        }

        let genre_id = Self::extract_filter_id(genre_filter, "genre=");
        let encoded = urlencoding::encode(genre_id);
        let path = format!("{}/{}/all?type={}&genre={}", EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM, encoded);
        tracing::info!("get_album_genre_albums: fetching from {}", path);
        let response: AlbumsResponse = self.get(&path).await?;
        Ok(response.media_container.metadata)
    }

    // ========================================================================
    // Mood Methods
    // ========================================================================

    /// Get all moods for a music library.
    pub async fn get_moods(&self, library_key: &str) -> Result<Vec<Genre>, ApiError> {
        use super::models::GenresResponse;
        let path = format!("{}/{}/mood?type={}", EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM);
        tracing::info!("get_moods: fetching from {}", path);
        let response: GenresResponse = self.get(&path).await?;
        tracing::info!("get_moods: found {} moods", response.media_container.directory.len());
        Ok(response.media_container.directory)
    }

    /// Get albums in a specific mood.
    pub async fn get_mood_albums(
        &self,
        library_key: &str,
        mood_filter: &str,
    ) -> Result<Vec<Album>, ApiError> {
        tracing::info!("get_mood_albums: library_key={}, mood_filter={}", library_key, mood_filter);

        if mood_filter.is_empty() {
            return Err(ApiError::ParseError("Empty mood filter".to_string()));
        }

        let mood_id = Self::extract_filter_id(mood_filter, "mood=");
        let encoded = urlencoding::encode(mood_id);
        let path = format!("{}/{}/all?type={}&mood={}", EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM, encoded);
        tracing::info!("get_mood_albums: fetching from {}", path);
        let response: AlbumsResponse = self.get(&path).await?;
        Ok(response.media_container.metadata)
    }

    // ========================================================================
    // Style Methods
    // ========================================================================

    /// Get all styles for a music library.
    pub async fn get_styles(&self, library_key: &str) -> Result<Vec<Genre>, ApiError> {
        use super::models::GenresResponse;
        let path = format!("{}/{}/style?type={}", EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM);
        tracing::info!("get_styles: fetching from {}", path);
        let response: GenresResponse = self.get(&path).await?;
        tracing::info!("get_styles: found {} styles", response.media_container.directory.len());
        Ok(response.media_container.directory)
    }

    /// Get albums in a specific style.
    pub async fn get_style_albums(
        &self,
        library_key: &str,
        style_filter: &str,
    ) -> Result<Vec<Album>, ApiError> {
        tracing::info!("get_style_albums: library_key={}, style_filter={}", library_key, style_filter);

        if style_filter.is_empty() {
            return Err(ApiError::ParseError("Empty style filter".to_string()));
        }

        let style_id = Self::extract_filter_id(style_filter, "style=");
        let encoded = urlencoding::encode(style_id);
        let path = format!("{}/{}/all?type={}&style={}", EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM, encoded);
        tracing::info!("get_style_albums: fetching from {}", path);
        let response: AlbumsResponse = self.get(&path).await?;
        Ok(response.media_container.metadata)
    }

    // ========================================================================
    // Folder Browsing Methods
    // ========================================================================

    /// Get root folders for a library section.
    pub async fn get_library_folders(&self, library_key: &str) -> Result<FolderResponse, ApiError> {
        let path = format!("{}/{}/folder", EP_LIBRARY_SECTIONS, library_key);
        self.get(&path).await
    }

    /// Get contents of a specific folder.
    pub async fn get_folder_contents(&self, folder_key: &str) -> Result<FolderResponse, ApiError> {
        self.get(folder_key).await
    }

    /// Get all tracks in a folder, suitable for playback.
    pub async fn get_folder_tracks(&self, folder_key: &str) -> Result<Vec<Track>, ApiError> {
        let response = self.get_folder_contents(folder_key).await?;

        let mut tracks: Vec<Track> = response
            .media_container
            .metadata
            .into_iter()
            .filter_map(|meta| {
                let rating_key = meta.rating_key?;
                Some(Track {
                    rating_key,
                    key: meta.key,
                    title: meta.title,
                    duration: meta.duration,
                    parent_title: meta.parent_title,
                    grandparent_title: meta.grandparent_title,
                    index: meta.index,
                    parent_rating_key: None,
                    grandparent_rating_key: None,
                    thumb: None,
                    parent_thumb: None,
                    grandparent_thumb: None,
                    media: meta
                        .media
                        .into_iter()
                        .map(|m| Media {
                            id: m.id,
                            duration: m.duration,
                            bitrate: m.bitrate,
                            audio_channels: m.audio_channels.map(|c| c as u8),
                            audio_codec: m.audio_codec,
                            container: m.container,
                            part: m
                                .parts
                                .into_iter()
                                .map(|p| MediaPart {
                                    id: p.id,
                                    key: p.key.unwrap_or_default(),
                                    duration: p.duration,
                                    file: p.file,
                                    size: p.size,
                                    container: p.container,
                                })
                                .collect(),
                        })
                        .collect(),
                })
            })
            .collect();

        tracks.sort_by(|a, b| a.title.cmp(&b.title));
        Ok(tracks)
    }

    // ========================================================================
    // Hub/Discovery Methods
    // ========================================================================

    /// Get home hubs (Mixes For You, Recently Played, etc.).
    pub async fn get_home_hubs(&self) -> Result<Vec<Hub>, ApiError> {
        let response: HubsResponse = self.get(EP_HUBS).await?;
        Ok(response.media_container.hub)
    }

    /// Get music-specific hubs for a library.
    pub async fn get_music_hubs(&self, library_key: &str) -> Result<Vec<Hub>, ApiError> {
        let path = format!("{}/sections/{}", EP_HUBS, library_key);
        let response: HubsResponse = self.get(&path).await?;
        Ok(response.media_container.hub)
    }

    /// Get children/sub-stations for a station category.
    pub async fn get_station_children(&self, station_key: &str) -> Result<Vec<Station>, ApiError> {
        tracing::debug!("Getting station children for key: {}", station_key);

        if station_key.contains("/mood") || station_key.contains("/style") || station_key.contains("/decade") {
            return self.get_category_items(station_key).await;
        }

        let raw = self.get_raw(station_key).await?;
        tracing::debug!("Station children raw response (first 1000 chars): {}", &raw[..raw.len().min(1000)]);

        let response: StationsResponse = serde_json::from_str(&raw).map_err(|e| {
            tracing::error!("Station children parse error: {} - Response: {}", e, &raw[..raw.len().min(500)]);
            ApiError::ParseError(format!("Station children parse error: {}", e))
        })?;

        for hub in &response.media_container.hub {
            let stations = hub.stations();
            if !stations.is_empty() {
                tracing::info!("Found {} child stations", stations.len());
                return Ok(stations);
            }
        }

        tracing::warn!("No child stations found in hub response");
        Ok(vec![])
    }

    /// Get category items (moods, styles, decades) and convert to Station objects.
    async fn get_category_items(&self, category_path: &str) -> Result<Vec<Station>, ApiError> {
        tracing::debug!("Getting category items for: {}", category_path);

        let raw = self.get_raw(category_path).await?;
        tracing::debug!("Category response (first 500 chars): {}", &raw[..raw.len().min(500)]);

        let response: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
            ApiError::ParseError(format!("Category parse error: {}", e))
        })?;

        let mut stations = Vec::new();
        let library_key = category_path.split('/').nth(3).unwrap_or("5");

        if let Some(directories) = response
            .get("MediaContainer")
            .and_then(|mc| mc.get("Directory"))
            .and_then(|d| d.as_array())
        {
            tracing::info!("Found {} category items", directories.len());

            for dir in directories {
                let title = dir.get("title").and_then(|t| t.as_str()).unwrap_or("Unknown");
                // Get the numeric key (e.g., "349383" for style, "209005" for mood)
                // This is required for filtering - Plex API uses numeric IDs, not names
                let numeric_key = dir.get("key").and_then(|k| k.as_str()).unwrap_or("");

                let (station_type, identifier) = if category_path.contains("/mood") {
                    ("mood", "mood")
                } else if category_path.contains("/style") {
                    ("style", "style")
                } else if category_path.contains("/decade") {
                    ("decade", "decade")
                } else {
                    ("category", "other")
                };

                // Include both numeric ID (for API) and title (for display)
                // Format: /library/sections/{lib}/stations/{type}?id={key}&title={title}
                let playable_key = format!(
                    "{}/{}/stations/{}?id={}&title={}",
                    EP_LIBRARY_SECTIONS, library_key, identifier,
                    numeric_key, urlencoding::encode(title)
                );

                stations.push(Station {
                    key: playable_key,
                    title: title.to_string(),
                    station_type: station_type.to_string(),
                    identifier: Some(identifier.to_string()),
                    thumb: dir.get("thumb").and_then(|t| t.as_str()).map(|s| s.to_string()),
                    art: None,
                    description: None,
                });
            }
        }

        tracing::info!("Converted {} category items to stations", stations.len());
        Ok(stations)
    }

    /// Get radio stations for a music library.
    /// Returns the 7 standard Plex station types.
    pub async fn get_stations(&self, library_key: &str) -> Result<Vec<Station>, ApiError> {
        // Plex has 7 standard station types for music libraries.
        // Rather than parse the hubs API (which returns expanded mood/style lists),
        // we construct these directly.
        let stations = vec![
            Station {
                key: format!("{}/{}/stations/library", EP_LIBRARY_SECTIONS, library_key),
                title: "Library Radio".to_string(),
                station_type: "station".to_string(),
                identifier: Some("library".to_string()),
                thumb: None,
                art: None,
                description: Some("Random tracks from your library".to_string()),
            },
            Station {
                key: format!("{}/{}/stations/deepCuts", EP_LIBRARY_SECTIONS, library_key),
                title: "Deep Cuts Radio".to_string(),
                station_type: "station".to_string(),
                identifier: Some("deepCuts".to_string()),
                thumb: None,
                art: None,
                description: Some("Less popular tracks from your library".to_string()),
            },
            Station {
                key: format!("{}/{}/stations/timeTravel", EP_LIBRARY_SECTIONS, library_key),
                title: "Time Travel Radio".to_string(),
                station_type: "station".to_string(),
                identifier: Some("timeTravel".to_string()),
                thumb: None,
                art: None,
                description: Some("Chronological journey through your library".to_string()),
            },
            Station {
                key: format!("{}/{}/stations/randomAlbum", EP_LIBRARY_SECTIONS, library_key),
                title: "Random Album Radio".to_string(),
                station_type: "station".to_string(),
                identifier: Some("randomAlbum".to_string()),
                thumb: None,
                art: None,
                description: Some("Play a random album from your library".to_string()),
            },
            Station {
                key: format!("{}/{}/mood", EP_LIBRARY_SECTIONS, library_key),
                title: "Mood Radio".to_string(),
                station_type: "station.category".to_string(),
                identifier: Some("mood".to_string()),
                thumb: None,
                art: None,
                description: Some("Play music by mood".to_string()),
            },
            Station {
                key: format!("{}/{}/style", EP_LIBRARY_SECTIONS, library_key),
                title: "Style Radio".to_string(),
                station_type: "station.category".to_string(),
                identifier: Some("style".to_string()),
                thumb: None,
                art: None,
                description: Some("Play music by style".to_string()),
            },
            Station {
                key: format!("{}/{}/decade", EP_LIBRARY_SECTIONS, library_key),
                title: "Decade Radio".to_string(),
                station_type: "station.category".to_string(),
                identifier: Some("decade".to_string()),
                thumb: None,
                art: None,
                description: Some("Play music from a specific decade".to_string()),
            },
        ];

        tracing::debug!("Returning {} standard station types", stations.len());
        Ok(stations)
    }

    // =============================================================================
    // STATION QUEUE IMPLEMENTATION GUIDELINES
    // =============================================================================
    //
    // Station queue creation can be called from the main event loop, so it MUST NOT
    // block for extended periods. Follow these rules:
    //
    // 1. PREFER PlayQueue API: Use create_station_queue_via_playqueue() when possible.
    //    This lets the Plex server build the queue, which is much faster.
    //
    // 2. LIMIT NETWORK CALLS: If manual fetching is required, limit to MAX 10 calls.
    //    Example: 3 decades × 2 albums × 1 track fetch = 7 calls max.
    //
    // 3. TIMEOUT PROTECTION: Event loop wraps station calls with 30-second timeout.
    //    Don't rely on this - design for <10 second completion.
    //
    // 4. FAIL GRACEFULLY: If a station partially fails, return what you have rather
    //    than failing entirely. Some tracks > no tracks.
    //
    // 5. LOG APPROPRIATELY: Use tracing::info for success, tracing::warn for issues
    //    users should know about, tracing::debug for internal details.
    //
    // Why these rules matter: The main event loop awaits station queue creation.
    // During this time, the UI cannot update. A 50-call station implementation that
    // takes 5 minutes will freeze the app for 5 minutes with no feedback.
    // =============================================================================

    /// Create a play queue from a station.
    pub async fn create_station_queue(&self, station_key: &str) -> Result<Vec<Track>, ApiError> {
        tracing::debug!("Creating station queue for key: {}", station_key);

        // Handle filtered stations (mood/style/decade with id/title parameters)
        // Format: /library/sections/{lib}/stations/{type}?id={key}&title={title}
        if station_key.contains("?id=") || station_key.contains("?title=") {
            return self.create_filtered_station_queue(station_key).await;
        }

        // Extract library key and station type from the key
        // Format: /library/sections/{lib}/stations/{type}
        let parts: Vec<&str> = station_key.split('/').collect();
        let library_key = parts.get(3).unwrap_or(&"5");
        let station_type = parts.last().unwrap_or(&"library");

        tracing::debug!("Station type: {}, library: {}", station_type, library_key);

        // Try to create tracks based on station type using direct queries
        // This is more reliable than the playQueue endpoint which may not work on all servers
        match *station_type {
            "library" => {
                // Library Radio: random tracks from library
                let path = format!(
                    "{}/{}/all?type={}&sort=random&limit={}",
                    EP_LIBRARY_SECTIONS, library_key, TYPE_TRACK, DEFAULT_SEARCH_LIMIT
                );
                let response: TracksResponse = self.get(&path).await?;
                tracing::info!("Library radio: {} tracks", response.media_container.metadata.len());
                Ok(response.media_container.metadata)
            }
            "deepCuts" => {
                // Deep Cuts: tracks with low play count, sorted by plays ascending then random
                let path = format!(
                    "{}/{}/all?type={}&sort=viewCount:asc,random&limit={}",
                    EP_LIBRARY_SECTIONS, library_key, TYPE_TRACK, DEFAULT_SEARCH_LIMIT
                );
                let response: TracksResponse = self.get(&path).await?;
                tracing::info!("Deep cuts radio: {} tracks", response.media_container.metadata.len());
                Ok(response.media_container.metadata)
            }
            "timeTravel" => {
                // Time Travel Radio: chronological journey through your library's history
                // Starts from earliest decades, plays a few tracks from each, then jumps forward
                return self.create_time_travel_queue(library_key).await;
            }
            "randomAlbum" => {
                // Random Album: get a random album and return its tracks
                let albums_path = format!(
                    "{}/{}/all?type={}&sort=random&limit=1",
                    EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM
                );
                let albums_resp: AlbumsResponse = self.get(&albums_path).await?;

                if let Some(album) = albums_resp.media_container.metadata.first() {
                    let tracks_path = format!(
                        "{}/{}/children",
                        EP_LIBRARY_METADATA, album.rating_key
                    );
                    let tracks_resp: TracksResponse = self.get(&tracks_path).await?;
                    tracing::info!("Random album radio: {} - {} tracks", album.title, tracks_resp.media_container.metadata.len());
                    Ok(tracks_resp.media_container.metadata)
                } else {
                    tracing::warn!("No albums found in library");
                    Ok(vec![])
                }
            }
            _ => {
                // For other station types, try the playQueue API as fallback
                self.create_station_queue_via_playqueue(station_key).await
            }
        }
    }

    /// Create station queue using the playQueue API (may not work on all servers).
    async fn create_station_queue_via_playqueue(&self, station_key: &str) -> Result<Vec<Track>, ApiError> {
        let server = self.require_server()?;
        let token = self.require_token()?;

        let machine_id = self.get_server_machine_id().await?;
        tracing::debug!("Server machine ID: {}", machine_id);

        let uri = format!(
            "server://{}/com.plexapp.plugins.library{}",
            machine_id,
            station_key
        );

        let path = format!(
            "{}?type=audio&uri={}&shuffle=0&repeat=0&continuous=1&includeChapters=1&includeMarkers=1&includeRelated=1",
            EP_PLAY_QUEUES,
            urlencoding::encode(&uri)
        );

        tracing::debug!("PlayQueue request path: {}", path);

        let url = format!("{}{}&{}={}", server, path, HEADER_PLEX_TOKEN, token);

        let response = self
            .http
            .post(&url)
            .headers(self.build_headers()?)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            tracing::error!("PlayQueue creation failed: {} - {}", status, message);
            return Err(ApiError::ServerError { status, message });
        }

        let text = response.text().await?;
        tracing::debug!("PlayQueue response (first 500 chars): {}", &text[..text.len().min(500)]);

        let queue: PlayQueueResponse = serde_json::from_str(&text).map_err(|e| {
            tracing::error!("PlayQueue parse error: {} - Response: {}", e, &text[..text.len().min(500)]);
            ApiError::ParseError(format!("PlayQueue parse error: {}", e))
        })?;

        tracing::info!("Station queue created with {} tracks", queue.media_container.metadata.len());
        Ok(queue.media_container.metadata)
    }

    /// Create a filtered play queue for mood/style/decade stations.
    /// Uses the playQueue API which is what Plexamp uses for these stations.
    async fn create_filtered_station_queue(&self, station_key: &str) -> Result<Vec<Track>, ApiError> {
        tracing::info!("Creating filtered station queue for: {}", station_key);

        // Try playQueue API first (this is what Plexamp uses)
        match self.create_station_queue_via_playqueue(station_key).await {
            Ok(tracks) if !tracks.is_empty() => {
                tracing::info!("PlayQueue returned {} tracks", tracks.len());
                return Ok(tracks);
            }
            Ok(_) => {
                tracing::debug!("PlayQueue returned no tracks, trying direct query");
            }
            Err(e) => {
                tracing::debug!("PlayQueue failed ({}), trying direct query", e);
            }
        }

        // Fallback: try direct query with filter
        // Parse station key format: /library/sections/{lib}/stations/{type}?id={key}&title={title}
        let parts: Vec<&str> = station_key.split('?').collect();
        let path_part = parts.first().unwrap_or(&"");
        let query_part = parts.get(1).unwrap_or(&"");

        let library_key = path_part.split('/').nth(3).unwrap_or("5");

        let is_decade = path_part.contains("/decade");
        let filter_type = if path_part.contains("/mood") {
            "mood"
        } else if path_part.contains("/style") {
            "style"
        } else {
            "decade"
        };

        // Parse query parameters: id={key}&title={title}
        let mut numeric_id = String::new();
        let mut title = String::new();
        for param in query_part.split('&') {
            if let Some(id) = param.strip_prefix("id=") {
                numeric_id = id.to_string();
            } else if let Some(t) = param.strip_prefix("title=") {
                title = urlencoding::decode(t).unwrap_or_default().to_string();
            }
        }

        tracing::info!("Fallback: Creating {} radio for '{}' (id={}) in library {}",
            filter_type, title, numeric_id, library_key);

        // Use numeric ID for filtering - Plex API requires numeric IDs for style/mood
        // For decade, use the numeric value from the ID (it's already numeric like "1980")
        let filter_value = if !numeric_id.is_empty() {
            numeric_id.clone()
        } else if is_decade {
            // Legacy fallback: extract numeric from title like "1980s" -> "1980"
            title.chars().filter(|c| c.is_ascii_digit()).collect()
        } else {
            // Legacy fallback: use title (may not work for styles)
            title.clone()
        };

        // Style and decade metadata is attached to albums, not tracks directly.
        // We need to get albums first, then get their tracks.
        if filter_type == "style" || filter_type == "decade" {
            return self.create_album_filter_radio_tracks(library_key, filter_type, &filter_value).await;
        }

        // Mood filters work directly on tracks
        let filter_path = format!(
            "{}/{}/all?type={}&{}={}&sort=random&limit={}",
            EP_LIBRARY_SECTIONS, library_key, TYPE_TRACK,
            filter_type, urlencoding::encode(&filter_value),
            DEFAULT_SEARCH_LIMIT
        );

        tracing::debug!("Filtered tracks path: {}", filter_path);

        let response: TracksResponse = self.get(&filter_path).await?;

        tracing::info!("Filtered station returned {} tracks", response.media_container.metadata.len());
        Ok(response.media_container.metadata)
    }

    /// Create radio tracks by first getting albums matching a filter, then getting their tracks.
    /// Style and decade metadata is attached to albums, not tracks, so we need this approach.
    async fn create_album_filter_radio_tracks(&self, library_key: &str, filter_type: &str, filter_value: &str) -> Result<Vec<Track>, ApiError> {
        // Get random albums matching this filter (style or decade)
        let albums_path = format!(
            "{}/{}/all?type={}&{}={}&sort=random&limit=10",
            EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM, filter_type, filter_value
        );

        tracing::debug!("{} radio albums path: {}", filter_type, albums_path);

        let albums_resp: AlbumsResponse = self.get(&albums_path).await?;

        if albums_resp.media_container.metadata.is_empty() {
            tracing::info!("No albums found for {} filter {}", filter_type, filter_value);
            return Ok(vec![]);
        }

        tracing::debug!("Found {} albums for {} filter", albums_resp.media_container.metadata.len(), filter_type);

        // Get tracks from these albums (collect tracks from multiple albums)
        let mut all_tracks = Vec::new();
        for album in albums_resp.media_container.metadata.iter().take(5) {
            let tracks_path = format!("{}/{}/children", EP_LIBRARY_METADATA, album.rating_key);
            if let Ok(tracks_resp) = self.get::<TracksResponse>(&tracks_path).await {
                all_tracks.extend(tracks_resp.media_container.metadata);
            }
        }

        // Shuffle the tracks
        use rand::seq::SliceRandom;
        all_tracks.shuffle(&mut rand::rng());

        // Limit to DEFAULT_SEARCH_LIMIT
        all_tracks.truncate(DEFAULT_SEARCH_LIMIT as usize);

        tracing::info!("{} radio returned {} tracks from {} albums",
            filter_type, all_tracks.len(), albums_resp.media_container.metadata.len().min(5));

        Ok(all_tracks)
    }

    /// Create Time Travel Radio queue using PlayQueue API.
    ///
    /// IMPORTANT: Station queues should ALWAYS use the PlayQueue API when possible.
    /// This lets the Plex server do the heavy lifting and avoids blocking the UI.
    ///
    /// If PlayQueue fails, falls back to a LIMITED manual approach starting from earliest decades.
    /// NEVER make more than 10 sequential network calls for any station.
    async fn create_time_travel_queue(&self, library_key: &str) -> Result<Vec<Track>, ApiError> {
        // Try PlayQueue API first (preferred - server does the work)
        let station_key = format!("{}/{}/stations/timeTravel", EP_LIBRARY_SECTIONS, library_key);

        match self.create_station_queue_via_playqueue(&station_key).await {
            Ok(tracks) if !tracks.is_empty() => {
                tracing::info!("Time Travel Radio: PlayQueue returned {} tracks", tracks.len());
                return Ok(tracks);
            }
            Ok(_) => tracing::debug!("Time Travel PlayQueue returned no tracks, trying fallback"),
            Err(e) => tracing::debug!("Time Travel PlayQueue failed ({}), trying fallback", e),
        }

        // Fallback: Start from the beginning chronologically
        // Get decades first, then fetch from index 0
        let decades = self.get_time_travel_decades(library_key).await?;
        if decades.is_empty() {
            return Ok(vec![]);
        }

        // Start from the beginning (index 0)
        self.fetch_time_travel_tracks_from_index(library_key, &decades, 0).await
    }

    /// Get sorted list of valid decades for Time Travel Radio.
    ///
    /// Returns decade values like ["1950", "1960", "1970", ...] sorted chronologically.
    pub async fn get_time_travel_decades(&self, library_key: &str) -> Result<Vec<String>, ApiError> {
        let decades_path = format!("{}/{}/decade", EP_LIBRARY_SECTIONS, library_key);
        let decade_items = self.get_category_items(&decades_path).await?;

        // Filter valid decades (1900-2030) and sort chronologically
        let mut decades: Vec<(i32, String)> = decade_items.iter()
            .filter_map(|s| {
                let year: i32 = s.title.chars()
                    .filter(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0);
                if year >= 1900 && year <= 2030 {
                    // Store decade value without 's' suffix (e.g., "1990" not "1990s")
                    Some((year, s.title.replace("s", "")))
                } else {
                    None
                }
            })
            .collect();

        decades.sort_by_key(|(y, _)| *y);

        let result: Vec<String> = decades.into_iter().map(|(_, v)| v).collect();
        tracing::info!("Time Travel Radio: found {} valid decades", result.len());
        Ok(result)
    }

    /// Fetch Time Travel tracks starting from a specific decade index.
    ///
    /// This enables chronological continuation - call with increasing indices
    /// to progress through time. Wraps around when reaching the end.
    ///
    /// LIMIT: Fetches from max 3 decades at a time (~7 network calls).
    pub async fn fetch_time_travel_tracks_from_index(
        &self,
        library_key: &str,
        decades: &[String],
        start_index: usize,
    ) -> Result<Vec<Track>, ApiError> {
        use rand::seq::SliceRandom;

        const MAX_DECADES_PER_FETCH: usize = 3;  // LIMIT: Max decades per fetch
        const ALBUMS_PER_DECADE: usize = 2;  // LIMIT: Albums per decade
        const TRACKS_PER_ALBUM: usize = 3;  // LIMIT: Tracks per album

        if decades.is_empty() {
            return Ok(vec![]);
        }

        let mut all_tracks = Vec::new();
        let total_decades = decades.len();

        // Fetch from up to MAX_DECADES_PER_FETCH decades, wrapping around if needed
        for i in 0..MAX_DECADES_PER_FETCH {
            let decade_idx = (start_index + i) % total_decades;
            let decade_value = &decades[decade_idx];

            let albums_path = format!(
                "{}/{}/all?type={}&decade={}&sort=rating:desc&limit={}",
                EP_LIBRARY_SECTIONS, library_key, TYPE_ALBUM, decade_value, ALBUMS_PER_DECADE
            );

            if let Ok(resp) = self.get::<AlbumsResponse>(&albums_path).await {
                for album in resp.media_container.metadata.iter().take(ALBUMS_PER_DECADE) {
                    let tracks_path = format!("{}/{}/children", EP_LIBRARY_METADATA, album.rating_key);
                    if let Ok(tracks_resp) = self.get::<TracksResponse>(&tracks_path).await {
                        let mut tracks = tracks_resp.media_container.metadata;
                        tracks.shuffle(&mut rand::rng());
                        all_tracks.extend(tracks.into_iter().take(TRACKS_PER_ALBUM));
                    }
                }
            }

            tracing::debug!("Time Travel: {}s (idx {}) -> {} tracks total",
                decade_value, decade_idx, all_tracks.len());
        }

        let end_index = (start_index + MAX_DECADES_PER_FETCH) % total_decades;
        tracing::info!("Time Travel Radio: fetched {} tracks from decades {}-{} (indices {}-{})",
            all_tracks.len(),
            &decades[start_index % total_decades],
            &decades[(start_index + MAX_DECADES_PER_FETCH - 1) % total_decades],
            start_index % total_decades,
            end_index);

        Ok(all_tracks)
    }

    /// Get the server's machine identifier (needed for playQueue URIs).
    async fn get_server_machine_id(&self) -> Result<String, ApiError> {
        let response: serde_json::Value = self.get("/").await?;
        response
            .get("MediaContainer")
            .and_then(|mc| mc.get("machineIdentifier"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| ApiError::ParseError("Could not get server machine ID".to_string()))
    }

    // ========================================================================
    // Search Methods
    // ========================================================================

    /// Search across all content.
    pub async fn search(&self, query: &str) -> Result<SearchResults, ApiError> {
        let path = format!(
            "{}?query={}&limit={}&{}=0&{}={}",
            EP_HUBS_SEARCH,
            urlencoding::encode(query),
            DEFAULT_SEARCH_LIMIT,
            PARAM_CONTAINER_START,
            PARAM_CONTAINER_SIZE, DEFAULT_SEARCH_LIMIT
        );
        let response: SearchResponse = self.get(&path).await?;

        let mut results = SearchResults::default();

        for hub in response.media_container.hub {
            if let Some(metadata) = hub.metadata {
                match hub.hub_type.as_str() {
                    "artist" => {
                        match serde_json::from_value::<Vec<Artist>>(metadata) {
                            Ok(artists) => results.artists = artists,
                            Err(e) => tracing::warn!("Failed to parse artists: {}", e),
                        }
                    }
                    "album" => {
                        match serde_json::from_value::<Vec<Album>>(metadata) {
                            Ok(albums) => results.albums = albums,
                            Err(e) => tracing::warn!("Failed to parse albums: {}", e),
                        }
                    }
                    "track" => {
                        match serde_json::from_value::<Vec<Track>>(metadata) {
                            Ok(tracks) => {
                                tracing::debug!("Search found {} tracks", tracks.len());
                                results.tracks = tracks;
                            }
                            Err(e) => tracing::warn!("Failed to parse tracks: {}", e),
                        }
                    }
                    "playlist" => {
                        match serde_json::from_value::<Vec<Playlist>>(metadata) {
                            Ok(playlists) => results.playlists = playlists,
                            Err(e) => tracing::warn!("Failed to parse playlists: {}", e),
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(results)
    }

    // ========================================================================
    // Sonic Similarity
    // ========================================================================

    /// Get sonically similar tracks using the sonicallySimilar filter.
    pub async fn get_similar_tracks(
        &self,
        rating_key: &str,
        limit: u32,
    ) -> Result<Vec<Track>, ApiError> {
        let section_id = self.get_library_section_id(rating_key).await?;

        let Some(section_id) = section_id else {
            tracing::warn!("Could not determine library section for track {}", rating_key);
            return Ok(Vec::new());
        };

        let path = format!(
            "{}/{}/all?type={}&sonicallySimilar={}&limit={}",
            EP_LIBRARY_SECTIONS, section_id, TYPE_TRACK, rating_key, limit
        );

        let response: TracksResponse = self.get(&path).await?;
        let tracks = response.media_container.metadata;

        tracing::info!(
            "get_similar_tracks for {} in section {} found {} tracks",
            rating_key, section_id, tracks.len()
        );

        Ok(tracks)
    }

    /// Get sonically similar albums using Plex's sonicallySimilar filter.
    pub async fn get_similar_albums(
        &self,
        rating_key: &str,
        limit: u32,
    ) -> Result<Vec<Album>, ApiError> {
        let section_id = self.get_library_section_id(rating_key).await?;

        let Some(section_id) = section_id else {
            tracing::warn!("Could not determine library section for album {}", rating_key);
            return Ok(Vec::new());
        };

        let path = format!(
            "{}/{}/all?type={}&sonicallySimilar={}&limit={}",
            EP_LIBRARY_SECTIONS, section_id, TYPE_ALBUM, rating_key, limit
        );

        let response: AlbumsResponse = self.get(&path).await?;
        let albums = response.media_container.metadata;

        tracing::info!(
            "get_similar_albums for {} in section {} found {} albums",
            rating_key, section_id, albums.len()
        );

        Ok(albums)
    }

    /// Get the library section ID for an item.
    async fn get_library_section_id(&self, rating_key: &str) -> Result<Option<u32>, ApiError> {
        let meta_path = format!("{}/{}", EP_LIBRARY_METADATA, rating_key);
        let raw = self.get::<serde_json::Value>(&meta_path).await?;
        Ok(raw
            .get("MediaContainer")
            .and_then(|mc| mc.get("librarySectionID"))
            .and_then(|v| v.as_u64())
            .map(|v| v as u32))
    }

    // ========================================================================
    // Streaming URLs
    // ========================================================================

    /// Get the direct stream URL for a track.
    pub fn get_stream_url(&self, track: &Track) -> Result<String, ApiError> {
        let part = track.stream_part().ok_or(ApiError::NoMediaAvailable)?;
        let token = self.require_token()?;
        let server = self.require_server()?;

        Ok(format!("{}{}?{}={}", server, part.key, HEADER_PLEX_TOKEN, token))
    }

    /// Get a transcoded stream URL (Plex converts to MP3).
    pub fn get_transcoded_stream_url(&self, track: &Track) -> Result<String, ApiError> {
        let token = self.require_token()?;
        let server = self.require_server()?;
        let session_id = uuid::Uuid::new_v4().to_string();

        Ok(format!(
            "{}{}?path={}&mediaIndex=0&partIndex=0&protocol=http&directPlay=0&directStream=0\
             &fastSeek=1&location=lan&session={}&{}={}&{}={}&{}={}&{}={}",
            server,
            EP_MUSIC_TRANSCODE,
            urlencoding::encode(&format!("{}/{}", EP_LIBRARY_METADATA, track.rating_key)),
            session_id,
            HEADER_PLEX_CLIENT_ID, urlencoding::encode(&self.client_info.client_identifier),
            HEADER_PLEX_PRODUCT, urlencoding::encode(&self.client_info.product),
            HEADER_PLEX_PLATFORM, urlencoding::encode(&self.client_info.platform),
            HEADER_PLEX_TOKEN, token
        ))
    }

    /// Get client identifier.
    pub fn client_identifier(&self) -> &str {
        &self.client_info.client_identifier
    }

    /// Get thumbnail URL with transcoding for size.
    pub fn get_thumb_url(&self, thumb_path: &str, width: u32, height: u32) -> Result<String, ApiError> {
        let token = self.require_token()?;
        let server = self.require_server()?;

        Ok(format!(
            "{}{}?width={}&height={}&minSize=1&upscale=1&url={}&{}={}",
            server, EP_PHOTO_TRANSCODE, width, height,
            urlencoding::encode(thumb_path),
            HEADER_PLEX_TOKEN, token
        ))
    }

    /// Get raw thumbnail URL without transcoding.
    pub fn get_thumb_url_raw(&self, thumb_path: &str) -> Result<String, ApiError> {
        let token = self.require_token()?;
        let server = self.require_server()?;

        Ok(format!("{}{}?{}={}", server, thumb_path, HEADER_PLEX_TOKEN, token))
    }

    /// Fetch artwork image data as bytes.
    pub async fn fetch_artwork(&self, thumb_path: &str, size: u32) -> Result<Vec<u8>, ApiError> {
        let url = self.get_thumb_url(thumb_path, size, size)?;

        let response = self
            .http
            .get(&url)
            .headers(self.build_headers()?)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(ApiError::ServerError {
                status: response.status().as_u16(),
                message: "Failed to fetch artwork".to_string(),
            });
        }

        Ok(response.bytes().await?.to_vec())
    }

    // ========================================================================
    // Playback Reporting
    // ========================================================================

    /// Report playback start to Plex (for "Continue Listening" etc.).
    pub async fn report_playback_start(&self, track: &Track, position_ms: u64, session_id: Option<&str>) -> Result<(), ApiError> {
        self.report_timeline(track, position_ms, "playing", None, session_id).await
    }

    /// Report playback progress.
    pub async fn report_playback_progress(&self, track: &Track, position_ms: u64, session_id: Option<&str>) -> Result<(), ApiError> {
        self.report_timeline(track, position_ms, "playing", None, session_id).await
    }

    /// Report playback stop with continuation flag.
    ///
    /// - `continuing=true`: Client is moving to another track (don't clear from Now Playing)
    /// - `continuing=false`: Playback truly ended (clear from Now Playing)
    pub async fn report_playback_stop(&self, track: &Track, position_ms: u64, continuing: bool, session_id: Option<&str>) -> Result<(), ApiError> {
        self.report_timeline(track, position_ms, "stopped", Some(continuing), session_id).await
    }

    /// Internal helper for timeline reporting.
    ///
    /// The `continuing` parameter is only meaningful when state is "stopped":
    /// - `Some(true)`: Moving to another track
    /// - `Some(false)`: Truly stopping playback
    /// - `None`: Not applicable (for playing state)
    ///
    /// The `session_id` is sent as `X-Plex-Session-Identifier` header to correlate
    /// all timeline reports for a single playback session.
    async fn report_timeline(
        &self,
        track: &Track,
        position_ms: u64,
        state: &str,
        continuing: Option<bool>,
        session_id: Option<&str>,
    ) -> Result<(), ApiError> {
        use reqwest::header::HeaderValue;

        let mut path = format!(
            "{}?ratingKey={}&key={}&state={}&time={}&duration={}&identifier=com.plexapp.plugins.library",
            EP_TIMELINE, track.rating_key, track.key, state, position_ms, track.duration_ms()
        );

        // Add continuing parameter when stopping (tells Plex whether to clear the session)
        if let Some(cont) = continuing {
            path.push_str(&format!("&continuing={}", if cont { 1 } else { 0 }));
        }

        let url = self.build_url(&path)?;
        let mut headers = self.build_headers()?;

        // Add session identifier header if provided (correlates all reports for this playback session)
        if let Some(sid) = session_id {
            headers.insert(
                HEADER_PLEX_SESSION_ID,
                HeaderValue::from_str(sid)
                    .map_err(|_| ApiError::InvalidHeader(HEADER_PLEX_SESSION_ID.to_string()))?,
            );
        }

        self.http
            .get(&url)
            .headers(headers)
            .send()
            .await?;

        Ok(())
    }

    /// Mark a track as played (scrobble).
    pub async fn scrobble(&self, rating_key: &str) -> Result<(), ApiError> {
        let path = format!("{}?key={}&identifier=com.plexapp.plugins.library", EP_SCROBBLE, rating_key);
        let url = self.build_url(&path)?;

        self.http
            .get(&url)
            .headers(self.build_headers()?)
            .send()
            .await?;

        Ok(())
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /// Extract filter ID from a filter string (e.g., "genre=123" -> "123").
    fn extract_filter_id<'a>(filter: &'a str, prefix: &str) -> &'a str {
        if filter.contains(prefix) {
            filter
                .split(prefix)
                .nth(1)
                .and_then(|s| s.split('&').next())
                .unwrap_or(filter)
        } else {
            filter
        }
    }
}

// ============================================================================
// Standalone Connection Testing
// ============================================================================

/// Test if a server URL is reachable with a short timeout.
/// Returns Ok(()) if reachable, Err with reason if not.
///
/// This is used to test multiple connection options before committing to one,
/// which is essential for remote access when local IPs are unreachable.
pub async fn test_connection(url: &str, token: &str) -> Result<(), ApiError> {
    let http = Client::builder()
        .timeout(Duration::from_secs(CONNECTION_TEST_TIMEOUT_SECS))
        .build()
        .map_err(ApiError::Http)?;

    let response = http
        .get(format!("{}/", url.trim_end_matches('/')))
        .header(HEADER_PLEX_TOKEN, token)
        .header("Accept", "application/json")
        .send()
        .await?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(ApiError::ServerError {
            status: response.status().as_u16(),
            message: "Server returned error".to_string(),
        })
    }
}
