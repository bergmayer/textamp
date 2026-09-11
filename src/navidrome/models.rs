use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MusicFolder {
    #[serde(deserialize_with = "string_id")]
    pub id: String,
    pub name: String,
}
fn string_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    use serde::de::Error;
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        _ => Err(D::Error::custom("Invalid music folder ID")),
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub cover_art: Option<String>,
    #[serde(default)]
    pub album: Vec<Album>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Genre {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    #[serde(default)]
    pub is_compilation: bool,
    #[serde(default)]
    pub release_types: Vec<String>,
    #[serde(default)]
    pub moods: Vec<String>,
    pub original_release_date: Option<ItemDate>,
    pub release_date: Option<ItemDate>,
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub artist_id: Option<String>,
    pub cover_art: Option<String>,
    pub year: Option<u16>,
    pub genre: Option<String>,
    /// OpenSubsonic's complete genre list; the legacy field holds only one tag.
    #[serde(default)]
    pub genres: Vec<Genre>,
    pub song_count: Option<u32>,
    pub duration: Option<u64>,
    #[serde(default)]
    pub song: Vec<Song>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    pub play_count: Option<u64>,
    pub id: String,
    pub title: String,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub artist: Option<String>,
    pub artist_id: Option<String>,
    pub album_artist: Option<String>,
    pub cover_art: Option<String>,
    pub track: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<u16>,
    pub duration: Option<u64>,
    pub suffix: Option<String>,
    pub bit_rate: Option<u32>,
    pub size: Option<u64>,
    pub path: Option<String>,
    pub genre: Option<String>,
    pub starred: Option<String>,
    pub user_rating: Option<u8>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct ItemDate {
    pub year: Option<u16>,
    pub month: Option<u8>,
    pub day: Option<u8>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub cover_art: Option<String>,
    pub song_count: Option<u32>,
    pub duration: Option<u64>,
    pub owner: Option<String>,
    #[serde(default)]
    pub entry: Vec<Song>,
}
