//! Album release-date grouping over normalized, cached metadata.
use crate::library::models::{Album, Genre};
use std::collections::BTreeMap;

pub fn decades(albums: &[Album]) -> Vec<Genre> {
    let mut counts = BTreeMap::<u16, u32>::new();
    for year in albums
        .iter()
        .filter_map(|album| album.year)
        .filter(|year| *year > 0)
    {
        *counts.entry(year / 10 * 10).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(decade, count)| Genre {
            key: decade.to_string(),
            title: format!("{decade}s"),
            count: Some(count),
        })
        .collect()
}

pub fn matches_decade(album: &Album, key: &str) -> bool {
    key.parse::<u16>()
        .ok()
        .filter(|decade| decade % 10 == 0)
        .zip(album.year.filter(|year| *year > 0))
        .is_some_and(|(decade, year)| year / 10 * 10 == decade)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_decades_include_boundaries_and_exclude_unknown_dates() {
        let albums: Vec<_> = [
            None,
            Some(0),
            Some(1969),
            Some(1970),
            Some(1979),
            Some(1980),
        ]
        .into_iter()
        .map(|year| Album {
            year,
            ..Default::default()
        })
        .collect();
        let decades = decades(&albums);
        assert_eq!(
            decades
                .iter()
                .map(|row| row.title.as_str())
                .collect::<Vec<_>>(),
            ["1960s", "1970s", "1980s"]
        );
        assert_eq!(decades[1].count, Some(2));
        assert!(matches_decade(&albums[3], "1970"));
        assert!(matches_decade(&albums[4], "1970"));
        assert!(!matches_decade(&albums[5], "1970"));
        assert!(!matches_decade(&albums[0], "1970"));
    }
}
