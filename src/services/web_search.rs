//! Browser search preferences and encoded, user-initiated queries.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Google,
    DuckDuckGo,
}
impl Provider {
    pub const ALL: [Self; 2] = [Self::Google, Self::DuckDuckGo];
    pub fn name(self) -> &'static str {
        match self {
            Self::Google => "Google",
            Self::DuckDuckGo => "DuckDuckGo",
        }
    }
    pub fn mark(self) -> &'static str {
        match self {
            Self::Google => "G",
            Self::DuckDuckGo => "DDG",
        }
    }
    pub fn search_label(self) -> &'static str {
        match self {
            Self::Google => "Search Google",
            Self::DuckDuckGo => "Search DuckDuckGo",
        }
    }
    pub fn artist_url(self, artist: &str) -> String {
        let query = format!("{} music artist biography", artist.trim());
        self.url(&query)
    }
    pub fn url(self, query: &str) -> String {
        let base = match self {
            Self::Google => "https://www.google.com/search",
            Self::DuckDuckGo => "https://duckduckgo.com/",
        };
        format!("{base}?q={}", urlencoding::encode(query))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_queries_preserve_unicode_and_query_punctuation() {
        for provider in Provider::ALL {
            let url = reqwest::Url::parse(&provider.artist_url("  Björk & A+B?  ")).unwrap();
            assert_eq!(url.scheme(), "https");
            assert_eq!(
                url.query_pairs().collect::<Vec<_>>(),
                vec![("q".into(), "Björk & A+B? music artist biography".into())]
            );
        }
        let config: crate::config::Config = toml::from_str("").unwrap();
        assert_eq!(config.ui.web_search_provider, Provider::Google);
    }
}
