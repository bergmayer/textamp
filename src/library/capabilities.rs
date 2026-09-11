//! Backend feature contract. These describe supported operations, not whether a
//! server is reachable, an index is ready, or a library contains matching data.
//! Shared UI and orchestration must consult this contract, not provider names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    Catalog,
    Genres,
    Moods,
    ReleaseYears,
    ReleaseDates,
    PlayCounts,
    ArtistRadio,
    LibraryRadio,
    AlbumRadio,
    SavePlaylist,
    RelatedArtists,
    SonicSimilarity,
    SonicPaths,

    Transcoding,
}

#[derive(Debug, Clone, Copy)]
pub struct Capabilities(pub &'static [Feature]);
impl Capabilities {
    pub fn supports(self, feature: Feature) -> bool {
        self.0.contains(&feature)
    }
}

// A new adapter declares its operations here (or supplies its own static slice).
// It does not gain an operation merely by advertising it: provider dispatch is
// exhaustive and rejects missing implementations instead of falling into server.

pub const NAVIDROME: Capabilities = Capabilities(&[
    Feature::Catalog,
    Feature::Genres,
    Feature::Moods,
    Feature::ReleaseYears,
    Feature::ReleaseDates,
    Feature::PlayCounts,
    Feature::ArtistRadio,
    Feature::LibraryRadio,
    Feature::AlbumRadio,
    Feature::SavePlaylist,
    Feature::RelatedArtists,
    Feature::SonicSimilarity,
    Feature::SonicPaths,
    Feature::Transcoding,
]);
pub const FOLDERS: Capabilities = Capabilities(&[Feature::LibraryRadio, Feature::AlbumRadio]);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn station_ui_uses_declared_features_not_provider_names() {
        let simple = Capabilities(&[Feature::LibraryRadio, Feature::AlbumRadio]);
        let stations = crate::library::station::standard_stations("new-server", simple);
        assert_eq!(
            stations.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(),
            ["new-server/library", "new-server/randomAlbum"]
        );
        let metadata = Capabilities(&[
            Feature::LibraryRadio,
            Feature::AlbumRadio,
            Feature::ArtistRadio,
            Feature::Catalog,
            Feature::Genres,
            Feature::PlayCounts,
        ]);
        let stations = crate::library::station::standard_stations("another-server", metadata);
        assert!(stations.iter().any(|s| s.key.ends_with("/randomArtist")));
        assert!(stations.iter().any(|s| s.key.ends_with("/deepCuts")));
        assert!(!stations
            .iter()
            .any(|s| s.key.ends_with("/sonic") || s.key.ends_with("/albumMix")));
        assert_eq!(
            crate::library::station::standard_stations("nav", NAVIDROME).len(),
            12
        );
    }
}
