//! Inline list filter service.
//!
//! Provides weighted, priority-based filtering for browse lists.
//! Used for real-time filtering as the user types.

/// Results from inline list filter.
#[derive(Debug, Clone, Default)]
pub struct ListFilterResults {
    /// Indices of matched items in the original list (in priority order)
    pub matched_indices: Vec<usize>,
    /// Total number of matches found
    pub total_matches: usize,
    /// Whether there are more results beyond the limit
    pub has_more: bool,
}

/// Maximum number of results to return by default.
pub const DEFAULT_MAX_RESULTS: usize = 100;

/// Fold accented/diacritical characters to ASCII equivalents.
fn fold_to_ascii(c: char) -> Option<char> {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => Some('a'),
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => Some('e'),
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ĭ' | 'į' => Some('i'),
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ō' | 'ŏ' | 'ő' | 'ø' => Some('o'),
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => Some('u'),
        'ñ' | 'ń' | 'ņ' | 'ň' => Some('n'),
        'ç' | 'ć' | 'ĉ' | 'č' => Some('c'),
        'ð' | 'ď' => Some('d'),
        'ý' | 'ÿ' => Some('y'),
        'ś' | 'ŝ' | 'ş' | 'š' => Some('s'),
        'ź' | 'ż' | 'ž' => Some('z'),
        'ł' => Some('l'),
        'ř' => Some('r'),
        'ť' => Some('t'),
        'þ' => Some('t'),
        'æ' => Some('a'), // ae ligature → a (close enough for search)
        'ß' => Some('s'), // eszett → s
        _ => None,
    }
}

/// Normalize a string for fuzzy comparison: fold accents to ASCII, strip punctuation.
fn normalize_for_search(s: &str) -> String {
    s.chars()
        .filter_map(|c| {
            if c.is_ascii_alphanumeric() || c.is_whitespace() {
                Some(c)
            } else if c.is_alphanumeric() {
                // Non-ASCII alphanumeric: try accent folding, keep original if no mapping
                Some(fold_to_ascii(c).unwrap_or(c))
            } else {
                None // strip punctuation
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
enum QueryExpr {
    Term(SearchText),
    Phrase(String),
    Not(Box<QueryExpr>),
    And(Box<QueryExpr>, Box<QueryExpr>),
    Or(Box<QueryExpr>, Box<QueryExpr>),
}

fn parse_query(query: &str) -> Option<QueryExpr> {
    let tokens = tokenize_query(query)?;
    let mut parser = QueryParser {
        tokens: &tokens,
        pos: 0,
    };
    let expression = parser.parse_or()?;
    (parser.pos == tokens.len()).then_some(expression)
}

#[cfg(test)]
fn query_matches(text: &str, query: &str) -> bool {
    parse_query(query).is_some_and(|expr| eval_query(&expr, &SearchText::new(text)))
}
fn is_boolean_query(query: &str) -> bool {
    query.contains('"')
        || query.contains('(')
        || query.contains(')')
        || query.split_whitespace().count() > 1
        || query
            .split_whitespace()
            .any(|w| matches!(w.to_ascii_uppercase().as_str(), "AND" | "OR" | "NOT"))
}

#[derive(Debug, Clone)]
enum QueryToken {
    Word(String),
    Phrase(String),
    And,
    Or,
    Not,
    LParen,
    RParen,
}

fn tokenize_query(query: &str) -> Option<Vec<QueryToken>> {
    let mut chars = query.chars().peekable();
    let mut out = Vec::new();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        match c {
            '(' => out.push(QueryToken::LParen),
            ')' => out.push(QueryToken::RParen),
            '"' => {
                let mut s = String::new();
                let mut closed = false;
                for x in chars.by_ref() {
                    if x == '"' {
                        closed = true;
                        break;
                    }
                    s.push(x);
                }
                if !closed || s.is_empty() {
                    return None;
                }
                out.push(QueryToken::Phrase(s));
            }
            _ => {
                let mut s = String::from(c);
                while chars
                    .peek()
                    .is_some_and(|x| !x.is_whitespace() && *x != '(' && *x != ')' && *x != '"')
                {
                    s.push(chars.next()?);
                }
                out.push(match s.to_ascii_uppercase().as_str() {
                    "AND" => QueryToken::And,
                    "OR" => QueryToken::Or,
                    "NOT" => QueryToken::Not,
                    _ => QueryToken::Word(s),
                });
            }
        }
    }
    Some(out)
}

struct QueryParser<'a> {
    tokens: &'a [QueryToken],
    pos: usize,
}
impl QueryParser<'_> {
    fn parse_or(&mut self) -> Option<QueryExpr> {
        let mut e = self.parse_and()?;
        while matches!(self.tokens.get(self.pos), Some(QueryToken::Or)) {
            self.pos += 1;
            e = QueryExpr::Or(Box::new(e), Box::new(self.parse_and()?));
        }
        Some(e)
    }
    fn parse_and(&mut self) -> Option<QueryExpr> {
        let mut e = self.parse_unary()?;
        loop {
            if matches!(self.tokens.get(self.pos), Some(QueryToken::And)) {
                self.pos += 1;
                e = QueryExpr::And(Box::new(e), Box::new(self.parse_unary()?));
            } else if matches!(
                self.tokens.get(self.pos),
                Some(
                    QueryToken::Word(_)
                        | QueryToken::Phrase(_)
                        | QueryToken::Not
                        | QueryToken::LParen
                )
            ) {
                e = QueryExpr::And(Box::new(e), Box::new(self.parse_unary()?));
            } else {
                break;
            }
        }
        Some(e)
    }
    fn parse_unary(&mut self) -> Option<QueryExpr> {
        if matches!(self.tokens.get(self.pos), Some(QueryToken::Not)) {
            self.pos += 1;
            return Some(QueryExpr::Not(Box::new(self.parse_unary()?)));
        }
        self.parse_atom()
    }
    fn parse_atom(&mut self) -> Option<QueryExpr> {
        let t = self.tokens.get(self.pos)?;
        self.pos += 1;
        match t {
            QueryToken::Word(s) => Some(QueryExpr::Term(SearchText::new(s))),
            QueryToken::Phrase(s) => Some(QueryExpr::Phrase(s.to_lowercase())),
            QueryToken::LParen => {
                let e = self.parse_or()?;
                if !matches!(self.tokens.get(self.pos), Some(QueryToken::RParen)) {
                    return None;
                }
                self.pos += 1;
                Some(e)
            }
            _ => None,
        }
    }
}
fn eval_query(expr: &QueryExpr, text: &SearchText) -> bool {
    match expr {
        QueryExpr::Term(query) => text.contains(query),
        QueryExpr::Phrase(phrase) => text.raw.contains(phrase),
        QueryExpr::Not(a) => !eval_query(a, text),
        QueryExpr::And(a, b) => eval_query(a, text) && eval_query(b, text),
        QueryExpr::Or(a, b) => eval_query(a, text) || eval_query(b, text),
    }
}

/// Parsed search syntax; malformed expressions never enter a catalog scan.
#[derive(Debug)]
enum QueryMode {
    Ranked,
    Boolean(QueryExpr),
}
impl QueryMode {
    fn parse(query: &str) -> Option<Self> {
        if is_boolean_query(query) {
            parse_query(query).map(Self::Boolean)
        } else {
            Some(Self::Ranked)
        }
    }
}

/// Lower-cased text paired with the punctuation/accent-folded form used by
/// every list search. Keeping the predicates here prevents each ranking loop
/// from rebuilding the same compound conditions.
#[derive(Debug, Clone)]
struct SearchText {
    raw: String,
    normalized: Option<String>,
}

impl SearchText {
    fn new(value: &str) -> Self {
        let raw = if value.is_ascii() {
            value.to_ascii_lowercase()
        } else {
            value.to_lowercase()
        };
        let normalized = (!raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte.is_ascii_whitespace()))
        .then(|| normalize_for_search(&raw));
        Self {
            raw,
            normalized: normalized.filter(|s| !s.trim().is_empty()),
        }
    }

    fn normalized(&self) -> &str {
        self.normalized.as_deref().unwrap_or(&self.raw)
    }

    fn exact(&self, query: &Self) -> bool {
        self.raw == query.raw
            || (self.normalized.is_some() || query.normalized.is_some())
                && self.normalized() == query.normalized()
    }

    fn starts_with(&self, query: &Self) -> bool {
        self.raw.starts_with(&query.raw)
            || (self.normalized.is_some() || query.normalized.is_some())
                && self.normalized().starts_with(query.normalized())
    }

    fn word_starts_with(&self, query: &Self) -> bool {
        self.raw
            .split_whitespace()
            .any(|word| word.starts_with(&query.raw))
            || (self.normalized.is_some() || query.normalized.is_some())
                && self
                    .normalized()
                    .split_whitespace()
                    .any(|word| word.starts_with(query.normalized()))
    }

    fn last_word_starts_with(&self, query: &Self) -> bool {
        self.raw
            .split_whitespace()
            .next_back()
            .unwrap_or("")
            .starts_with(&query.raw)
            || (self.normalized.is_some() || query.normalized.is_some())
                && self
                    .normalized()
                    .split_whitespace()
                    .next_back()
                    .unwrap_or("")
                    .starts_with(query.normalized())
    }

    fn contains(&self, query: &Self) -> bool {
        self.raw.contains(&query.raw)
            || (self.normalized.is_some() || query.normalized.is_some())
                && self.normalized().contains(query.normalized())
    }
}

#[derive(Debug)]
pub(super) struct SearchQuery {
    text: SearchText,
    mode: QueryMode,
    terms: Vec<SearchText>,
}
impl SearchQuery {
    pub(super) fn title_priority(&self, title: &str) -> usize {
        let title = SearchText::new(title);
        if title.exact(&self.text) {
            return 0;
        }
        let name = title.normalized();
        let query = self.text.normalized();
        if (query.chars().count() >= 4 && one_edit(name, query))
            || name
                .strip_prefix(query)
                .is_some_and(|suffix| suffix.starts_with([' ', '(', '-', '[', ':']))
        {
            return 1;
        }
        search_rank(&title, &self.text).map_or(8, |rank| 2 + rank as usize)
    }

    pub(super) fn anchor(&self) -> Option<&str> {
        self.terms
            .iter()
            .map(SearchText::normalized)
            .filter(|word| word.len() >= 6 && word.bytes().all(|b| b.is_ascii_alphanumeric()))
            .max_by_key(|word| word.len())
    }
    pub(super) fn new(value: &str) -> Option<Self> {
        if value.trim().is_empty() {
            return None;
        }
        let text = SearchText::new(value);
        Some(Self {
            mode: QueryMode::parse(&text.raw)?,
            terms: if text.raw.contains(['"', '(', ')'])
                || text
                    .raw
                    .split_whitespace()
                    .any(|w| matches!(w, "and" | "or" | "not"))
            {
                Vec::new()
            } else {
                text.raw.split_whitespace().map(SearchText::new).collect()
            },
            text,
        })
    }
}

#[derive(Debug)]
enum RecordKind {
    Name,
    Album,
    Track,
}

/// Normalized fields are reusable across queries over the same immutable catalog.
#[derive(Debug)]
pub(super) struct SearchRecord {
    kind: RecordKind,
    title: SearchText,
    artist: Option<SearchText>,
    combined: Option<SearchText>,
}
impl SearchRecord {
    pub(super) fn name(title: &str) -> Self {
        Self {
            kind: RecordKind::Name,
            title: SearchText::new(title),
            artist: None,
            combined: None,
        }
    }
    pub(super) fn album(album: &crate::library::models::Album) -> Self {
        Self {
            kind: RecordKind::Album,
            title: SearchText::new(&album.title),
            artist: None,
            combined: Some(SearchText::new(&format!(
                "{} {} {} {}",
                album.title,
                album.artist_name(),
                album.year.map(|year| year.to_string()).unwrap_or_default(),
                album
                    .genre
                    .iter()
                    .map(|genre| genre.tag.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            ))),
        }
    }
    pub(super) fn track(track: &crate::library::models::Track) -> Self {
        Self {
            kind: RecordKind::Track,
            title: SearchText::new(&track.title),
            artist: Some(SearchText::new(track.track_artist())),
            combined: Some(SearchText::new(&format!(
                "{} {} {} {}",
                track.title,
                track.track_artist(),
                track.album_name(),
                track.genres.join(" ")
            ))),
        }
    }
    pub(super) fn searchable(&self) -> &str {
        self.combined.as_ref().unwrap_or(&self.title).normalized()
    }

    fn fuzzy_rank(&self, query: &SearchQuery) -> Option<usize> {
        let field = self.combined.as_ref().unwrap_or(&self.title);
        (!query.terms.is_empty()
            && query.terms.iter().all(|term| {
                field.contains(term)
                    || (term.normalized().chars().count() >= 4
                        && field
                            .normalized()
                            .split_whitespace()
                            .any(|word| one_edit(word, term.normalized())))
            }))
        .then_some(7)
    }

    pub(super) fn rank(&self, query: &SearchQuery) -> Option<usize> {
        if self.title.raw == query.text.raw {
            return Some(0);
        }
        let boolean = match &query.mode {
            QueryMode::Boolean(expression) => {
                if !eval_query(expression, self.combined.as_ref().unwrap_or(&self.title)) {
                    return self.fuzzy_rank(query);
                }
                true
            }
            QueryMode::Ranked => {
                if !self
                    .combined
                    .as_ref()
                    .unwrap_or(&self.title)
                    .contains(&query.text)
                {
                    return self.fuzzy_rank(query);
                }
                false
            }
        };
        match self.kind {
            RecordKind::Name => {
                if boolean {
                    Some(4)
                } else {
                    search_rank(&self.title, &query.text).map(|rank| rank as usize)
                }
            }
            RecordKind::Album => {
                if boolean {
                    Some(4)
                } else {
                    search_rank(&self.title, &query.text)
                        .map(|rank| rank as usize)
                        .or_else(|| {
                            self.combined
                                .as_ref()
                                .filter(|text| text.contains(&query.text))
                                .map(|_| 5)
                        })
                }
            }
            RecordKind::Track => {
                let artist = self.artist.as_ref()?;
                let text = &query.text;
                if self.title.exact(text) {
                    Some(0)
                } else if self.title.starts_with(text) {
                    Some(1)
                } else if artist.starts_with(text) {
                    Some(2)
                } else if self.title.word_starts_with(text) {
                    Some(3)
                } else if artist.word_starts_with(text) {
                    Some(4)
                } else if self.title.contains(text) {
                    Some(5)
                } else if boolean
                    || self
                        .combined
                        .as_ref()
                        .is_some_and(|combined| combined.contains(text))
                {
                    Some(6)
                } else {
                    None
                }
            }
        }
    }
}

// Accept one insertion, deletion, substitution, or adjacent transposition.
fn one_edit(left: &str, right: &str) -> bool {
    let mut a = left.chars();
    let mut b = right.chars();
    loop {
        let left_before = a.as_str();
        let right_before = b.as_str();
        let x = a.next();
        let y = b.next();
        if x != y {
            return a.as_str() == b.as_str()
                || a.as_str() == right_before
                || left_before == b.as_str()
                || (x.is_some()
                    && y.is_some()
                    && a.next() == y
                    && b.next() == x
                    && a.as_str() == b.as_str());
        }
        if x.is_none() {
            return true;
        }
    }
}

/// Keep only candidates that can occur in the final result, in stable catalog order.
pub(super) fn ranked_indices<I, R>(
    records: I,
    query: &SearchQuery,
    limit: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Vec<usize>
where
    I: IntoIterator<Item = R>,
    R: std::borrow::Borrow<SearchRecord>,
{
    rank_indexed(
        records.into_iter().enumerate(),
        query,
        limit,
        &mut cancelled,
    )
}

pub(super) fn rank_indexed<R: std::borrow::Borrow<SearchRecord>>(
    records: impl Iterator<Item = (usize, R)>,
    query: &SearchQuery,
    limit: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Vec<usize> {
    if limit == 0 {
        return vec![];
    }
    let mut buckets: [Vec<usize>; 8] = Default::default();
    for (visited, (index, record)) in records.enumerate() {
        let record = record.borrow();
        if visited % 256 == 0 && cancelled() {
            return vec![];
        }
        if let Some(rank) = record.rank(query)
            && buckets[rank].len() < limit
        {
            buckets[rank].push(index);
        }
        if buckets[0].len() == limit {
            break;
        }
    }
    if cancelled() {
        return vec![];
    }
    buckets.into_iter().flatten().take(limit).collect()
}

#[derive(Clone, Copy)]
enum FilterRank {
    StartsWith = 0,
    WordStartsWith = 1,
    Contains = 2,
}

fn filter_rank(text: &SearchText, query: &SearchText, short_query: bool) -> Option<FilterRank> {
    if text.starts_with(query) {
        Some(FilterRank::StartsWith)
    } else if short_query || !text.contains(query) {
        None
    } else if text.word_starts_with(query) {
        Some(FilterRank::WordStartsWith)
    } else {
        Some(FilterRank::Contains)
    }
}

#[derive(Clone, Copy)]
enum SearchRank {
    Exact = 0,
    LastWordStartsWith = 1,
    StartsWith = 2,
    WordStartsWith = 3,
    Contains = 4,
}

fn search_rank(text: &SearchText, query: &SearchText) -> Option<SearchRank> {
    if text.exact(query) {
        Some(SearchRank::Exact)
    } else if text.starts_with(query) {
        Some(SearchRank::StartsWith)
    } else if !text.contains(query) {
        None
    } else if text.last_word_starts_with(query) {
        Some(SearchRank::LastWordStartsWith)
    } else if text.word_starts_with(query) {
        Some(SearchRank::WordStartsWith)
    } else {
        Some(SearchRank::Contains)
    }
}

fn finish_filter<const N: usize>(
    buckets: [Vec<usize>; N],
    max_results: usize,
) -> ListFilterResults {
    let mut matched_indices: Vec<_> = buckets.into_iter().flatten().collect();
    let total_matches = matched_indices.len();
    let has_more = total_matches > max_results;
    matched_indices.truncate(max_results);

    ListFilterResults {
        matched_indices,
        total_matches,
        has_more,
    }
}

/// Filter items with priority-based matching.
///
/// Returns indices of matching items in priority order:
/// 1. Items where name starts with query
/// 2. Items where any word starts with query (skipped for queries < 2 chars)
/// 3. Items where name contains query (skipped for queries < 2 chars)
///
/// # Arguments
/// * `items` - The items to filter
/// * `query` - The search query
/// * `get_title` - Function to extract the title from an item
/// * `max_results` - Maximum number of results to return
pub fn filter_with_priority<T, F>(
    items: &[T],
    query: &str,
    get_title: F,
    max_results: usize,
) -> ListFilterResults
where
    F: Fn(&T) -> &str,
{
    filter_with_priority_until(items, query, get_title, max_results, || false).unwrap_or_default()
}

/// Filter a list while allowing an obsolete worker to stop between small batches.
pub fn filter_with_priority_until<T, F>(
    items: &[T],
    query: &str,
    get_title: F,
    max_results: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Option<ListFilterResults>
where
    F: Fn(&T) -> &str,
{
    if cancelled() {
        return None;
    }
    if query.is_empty() {
        return Some(ListFilterResults::default());
    }

    let short_query = query.len() < 2;
    let query = SearchText::new(query);
    let mut buckets: [Vec<usize>; 3] = Default::default();

    for (idx, item) in items.iter().enumerate() {
        if idx % 64 == 0 && cancelled() {
            return None;
        }
        let title = SearchText::new(get_title(item));
        if let Some(rank) = filter_rank(&title, &query, short_query) {
            buckets[rank as usize].push(idx);
        }
    }

    (!cancelled()).then(|| finish_filter(buckets, max_results))
}

/// Lightweight projection used to move large columns to a filtering worker
/// without cloning complete tracks, albums, artwork paths, and metadata.
#[derive(Debug, Clone)]
pub struct BrowseFilterRecord {
    pub title: String,
    pub search_text: Option<String>,
    pub album_year: Option<u16>,
    pub artist_key: Option<String>,
    pub is_compilations: bool,
}

fn artist_alias_matches(
    artist_key: &str,
    aliases_by_artist: &std::collections::HashMap<String, std::collections::HashSet<String>>,
    query: &str,
) -> bool {
    aliases_by_artist.get(artist_key).is_some_and(|aliases| {
        let query = crate::services::artist_alias_service::normalize_artist_name(query);
        !query.is_empty()
            && aliases.iter().any(|alias| {
                let alias = crate::services::artist_alias_service::normalize_artist_name(alias);
                alias.starts_with(&query) || alias.contains(&query)
            })
    })
}

/// Filter lightweight browse records with year and artist-alias matching.
pub fn filter_browse_records(
    items: &[BrowseFilterRecord],
    query: &str,
    max_results: usize,
    artist_aliases: &std::collections::HashMap<String, std::collections::HashSet<String>>,
    compilation_artist_keys: &std::collections::HashSet<String>,
) -> ListFilterResults {
    filter_browse_records_until(
        items,
        query,
        max_results,
        artist_aliases,
        compilation_artist_keys,
        || false,
    )
    .unwrap_or_default()
}

/// Filter browse metadata without publishing a partial result after cancellation.
pub fn filter_browse_records_until(
    items: &[BrowseFilterRecord],
    query: &str,
    max_results: usize,
    artist_aliases: &std::collections::HashMap<String, std::collections::HashSet<String>>,
    compilation_artist_keys: &std::collections::HashSet<String>,
    mut cancelled: impl FnMut() -> bool,
) -> Option<ListFilterResults> {
    if cancelled() {
        return None;
    }
    if query.is_empty() {
        return Some(ListFilterResults::default());
    }

    let query_lower = query.to_lowercase();
    let query_text = SearchText::new(&query_lower);
    let Some(mode) = QueryMode::parse(query) else {
        return Some(ListFilterResults::default());
    };
    let short_query = query.len() < 2;
    let mut result = ListFilterResults::default();
    let mut compilation_artist_matched = false;
    let mut compilation_row = None;

    for (idx, item) in items.iter().enumerate() {
        if idx % 64 == 0 && cancelled() {
            return None;
        }
        let matches = match &mode {
            QueryMode::Boolean(expression) => eval_query(
                expression,
                &SearchText::new(item.search_text.as_deref().unwrap_or(&item.title)),
            ),
            QueryMode::Ranked => {
                let matches_text = |text: &str| {
                    let field = SearchText::new(text);
                    if short_query {
                        field.starts_with(&query_text)
                    } else {
                        field.contains(&query_text)
                    }
                };
                matches_text(&item.title)
                    || item.search_text.as_deref().is_some_and(matches_text)
                    || item
                        .album_year
                        .is_some_and(|year| year.to_string().contains(&query_lower))
                    || (!short_query
                        && item.artist_key.as_deref().is_some_and(|key| {
                            artist_alias_matches(key, artist_aliases, &query_lower)
                        }))
            }
        };
        if item
            .artist_key
            .as_ref()
            .is_some_and(|key| compilation_artist_keys.contains(key))
        {
            compilation_artist_matched |= matches;
        } else if matches {
            result.total_matches += 1;
            if result.matched_indices.len() < max_results {
                result.matched_indices.push(idx);
            }
        } else if item.is_compilations {
            compilation_row = Some(idx);
        }
    }
    if compilation_artist_matched && let Some(index) = compilation_row {
        result.total_matches += 1;
        let position = result.matched_indices.partition_point(|&row| row < index);
        if position < max_results {
            result.matched_indices.insert(position, index);
            result.matched_indices.truncate(max_results);
        }
    }
    result.has_more = result.total_matches > result.matched_indices.len();
    (!cancelled()).then_some(result)
}

/// Rank command labels and their hidden aliases with the catalog's fuzzy matcher.
pub(crate) struct CommandSearch {
    text: String,
    query: Option<SearchQuery>,
    abbreviation: fuzzy_matcher::skim::SkimMatcherV2,
}

impl CommandSearch {
    pub(crate) fn new(text: &str) -> Self {
        let mut text = text
            .replace('_', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        while text.contains("::") {
            text = text.replace("::", ":");
        }
        Self {
            query: SearchQuery::new(&text),
            text,
            abbreviation: fuzzy_matcher::skim::SkimMatcherV2::default().ignore_case(),
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn score<'a>(
        &self,
        label: &str,
        aliases: impl IntoIterator<Item = &'a str>,
    ) -> Option<(std::cmp::Reverse<usize>, bool, i64)> {
        use fuzzy_matcher::FuzzyMatcher;
        let rank = |text: &str, visible| {
            if let Some(rank) = self
                .query
                .as_ref()
                .and_then(|query| SearchRecord::name(text).rank(query))
            {
                return Some((std::cmp::Reverse(rank), visible, 0));
            }
            // Keep abbreviated commands such as "nwpl" reachable.
            self.abbreviation
                .fuzzy_match(text, &self.text)
                .map(|score| (std::cmp::Reverse(8), visible, score))
        };
        std::iter::once(rank(label, true))
            .chain(aliases.into_iter().map(|alias| rank(alias, false)))
            .flatten()
            .max()
    }

    #[cfg(feature = "desktop")]
    pub(crate) fn label_positions(&self, label: &str) -> Vec<usize> {
        use fuzzy_matcher::FuzzyMatcher;
        self.abbreviation
            .fuzzy_indices(label, &self.text)
            .map_or_else(Vec::new, |(_, positions)| positions)
    }
}

/// Search items with priority-based ranking and last-name prioritization.
///
/// Returns cloned items in priority order:
/// 1. Exact match (title == query)
/// 2. Last word starts with query (last-name heuristic: "J.S. Bach" for "bach")
/// 3. Title starts with query
/// 4. Any word starts with query
/// 5. Contains query as substring
/// 6. Normalized contains (punctuation-stripped)
pub fn search_with_ranking<T: Clone, F>(
    items: &[T],
    query: &str,
    get_title: F,
    max_results: usize,
) -> Vec<T>
where
    F: Fn(&T) -> &str,
{
    let Some(query) = SearchQuery::new(query) else {
        return vec![];
    };
    let records = items.iter().map(|item| SearchRecord::name(get_title(item)));
    ranked_indices(records, &query, max_results, || false)
        .into_iter()
        .map(|index| items[index].clone())
        .collect()
}

/// Search albums with priority-based ranking, including year matching.
///
/// Same priorities as `search_with_ranking`, plus:
/// 7. Year matches query
pub fn search_albums_with_ranking(
    albums: &[crate::library::models::Album],
    query: &str,
    max_results: usize,
) -> Vec<crate::library::models::Album> {
    let Some(query) = SearchQuery::new(query) else {
        return vec![];
    };
    let records = albums.iter().map(SearchRecord::album);
    ranked_indices(records, &query, max_results, || false)
        .into_iter()
        .map(|index| albums[index].clone())
        .collect()
}

/// Search tracks with multi-field priority-based ranking.
///
/// Returns cloned tracks in priority order across title and artist fields:
/// 1. Exact title match
/// 2. Title starts with query
/// 3. Artist name starts with query
/// 4. Any word in title starts with query
/// 5. Any word in artist name starts with query
/// 6. Title contains query
/// 7. Artist name contains query
/// 8. Normalized contains (either field)
pub fn search_tracks_with_ranking(
    tracks: &[crate::library::models::Track],
    query: &str,
    max_results: usize,
) -> Vec<crate::library::models::Track> {
    search_tracks_with_ranking_until(tracks, query, max_results, || false)
}

/// A cancelled scan stops without allocating the remaining match buckets.
pub fn search_tracks_with_ranking_until(
    tracks: &[crate::library::models::Track],
    query: &str,
    max_results: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Vec<crate::library::models::Track> {
    let Some(query) = SearchQuery::new(query) else {
        return vec![];
    };
    let records = tracks.iter().map(SearchRecord::track);
    ranked_indices(records, &query, max_results, &mut cancelled)
        .into_iter()
        .map(|index| tracks[index].clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsolete_filter_stops_before_scanning_or_publishing_the_remaining_rows() {
        let visited = std::cell::Cell::new(0);
        let checks = std::cell::Cell::new(0);
        let items = vec!["artist"; 10_000];
        let result = filter_with_priority_until(
            &items,
            "art",
            |item| {
                visited.set(visited.get() + 1);
                item
            },
            100,
            || {
                checks.set(checks.get() + 1);
                checks.get() > 2
            },
        );
        assert!(result.is_none());
        assert!(visited.get() <= 64);
    }
    #[test]
    fn catalog_boolean_search_matches_across_metadata_and_rejects_malformed_queries() {
        use crate::library::models::{Album, Artist, Track};
        let artists = [
            Artist {
                title: "Björk Live".into(),
                ..Default::default()
            },
            Artist {
                title: "Björk Studio".into(),
                ..Default::default()
            },
        ];
        let albums = [Album {
            title: "Kind of Blue".into(),
            parent_title: Some("Miles Davis".into()),
            year: Some(1959),
            ..Default::default()
        }];
        let tracks = [
            Track {
                title: "So What".into(),
                grandparent_title: Some("Miles Davis".into()),
                parent_title: Some("Kind of Blue".into()),
                ..Default::default()
            },
            Track {
                title: "Blue Monday".into(),
                grandparent_title: Some("New Order".into()),
                ..Default::default()
            },
        ];
        assert_eq!(
            search_with_ranking(&artists, "bjork NOT studio", |artist| &artist.title, 10)[0].title,
            "Björk Live"
        );
        assert_eq!(
            search_albums_with_ranking(&albums, "blue AND 1959", 10).len(),
            1
        );
        assert_eq!(
            search_tracks_with_ranking(&tracks, "blue AND davis", 10)[0].title,
            "So What"
        );
        assert_eq!(
            search_tracks_with_ranking(&tracks, "blue NOT davis", 10)[0].title,
            "Blue Monday"
        );
        assert_eq!(
            search_tracks_with_ranking(
                &tracks,
                "\"So What\" AND \"Miles Davis\" NOT \"New Order\"",
                10
            )
            .len(),
            1
        );
        let phrase = search_tracks_with_ranking(&tracks, "\"blue monday\"", 10);
        assert_eq!(phrase.len(), 1);
        assert_eq!(phrase[0].title, "Blue Monday");
        for query in ["(blue OR", "blue AND", "\"blue"] {
            assert!(search_with_ranking(&artists, query, |artist| &artist.title, 10).is_empty());
            assert!(search_albums_with_ranking(&albums, query, 10).is_empty());
            assert!(search_tracks_with_ranking(&tracks, query, 10).is_empty());
        }
    }

    #[test]
    fn cancelled_catalog_search_discards_partial_matches() {
        let tracks = vec![
            crate::library::models::Track {
                title: "Blue Monday".into(),
                ..Default::default()
            };
            600
        ];
        let mut polls = 0;
        let results = search_tracks_with_ranking_until(&tracks, "blue", 100, || {
            polls += 1;
            polls == 2
        });
        assert_eq!(polls, 2);
        assert!(results.is_empty());
    }

    #[test]
    fn boolean_search_uses_standard_precedence_and_phrases() {
        assert!(query_matches("A B", "(A AND B) OR (C AND D)"));
        assert!(query_matches("C D", "(A AND B) OR (C AND D)"));
        assert!(!query_matches("A C", "(A AND B) OR (C AND D)"));
        assert!(query_matches("Blue Monday", "\"Blue Monday\""));
        assert!(!query_matches("Blue New Monday", "\"Blue Monday\""));
        assert!(query_matches("Blue Monday", "Blue NOT Tuesday"));
        assert!(!query_matches("Blue Tuesday", "Blue NOT Tuesday"));
        assert!(!query_matches("Blue", "(Blue OR"));
    }

    #[test]
    fn normalized_filter_preserves_word_and_substring_priority() {
        let items = [
            "Unrelated",
            "An AC/DC concert",
            "ACDC",
            "xxacdc",
            "Björk",
            "東京",
        ];
        assert_eq!(
            filter_with_priority(&items, "acdc", |s| s, 100).matched_indices,
            [2, 1, 3]
        );
        assert_eq!(
            filter_with_priority(&items, "AC/DC", |s| s, 100).matched_indices,
            [2, 1, 3]
        );
        assert_eq!(
            filter_with_priority(&items, "bjork", |s| s, 100).matched_indices,
            [4]
        );
        assert_eq!(
            filter_with_priority(&items, "東京", |s| s, 100).matched_indices,
            [5]
        );
    }

    #[test]
    fn test_filter_starts_with() {
        let items = vec!["Apple", "Banana", "Apricot", "Cherry"];
        let results = filter_with_priority(&items, "ap", |s| s, 100);

        // Should match "Apple" and "Apricot" (starts with "ap")
        assert_eq!(results.matched_indices, vec![0, 2]);
        assert_eq!(results.total_matches, 2);
        assert!(!results.has_more);
    }

    #[test]
    fn test_filter_priority_order() {
        let items = vec!["Beethoven", "The Beatles", "Beach Boys"];
        let results = filter_with_priority(&items, "be", |s| s, 100);

        // "Beethoven" and "Beach Boys" start with "be"
        // "The Beatles" has "Beatles" starting with "be"
        assert_eq!(results.matched_indices.len(), 3);
        assert_eq!(results.matched_indices[0], 0); // Beethoven first
        assert_eq!(results.matched_indices[1], 2); // Beach Boys second
        assert_eq!(results.matched_indices[2], 1); // The Beatles third (word match)
    }

    #[test]
    fn test_filter_short_query() {
        let items = vec!["Beethoven", "The Beatles", "Bach"];
        let results = filter_with_priority(&items, "b", |s| s, 100);

        // Short query: only "starts with" matches
        // "Beethoven" and "Bach" start with "b"
        // "The Beatles" does NOT match (word boundary check skipped for short queries)
        assert_eq!(results.matched_indices, vec![0, 2]);
    }

    #[test]
    fn test_filter_contains() {
        let items = vec!["Abbey Road", "Let It Be", "Rubber Soul"];
        let results = filter_with_priority(&items, "ber", |s| s, 100);

        // "Rubber Soul" contains "ber" (not at start or word boundary)
        assert_eq!(results.matched_indices, vec![2]);
    }

    #[test]
    fn test_filter_max_results() {
        let items: Vec<String> = (0..200).map(|i| format!("Item {}", i)).collect();
        let results = filter_with_priority(&items, "item", |s| s, 50);

        assert_eq!(results.matched_indices.len(), 50);
        assert_eq!(results.total_matches, 200);
        assert!(results.has_more);
    }

    #[test]
    fn test_filter_empty_query() {
        let items = vec!["Apple", "Banana"];
        let results = filter_with_priority(&items, "", |s| s, 100);

        assert!(results.matched_indices.is_empty());
        assert_eq!(results.total_matches, 0);
    }

    #[test]
    fn search_ranking_preserves_last_word_priority() {
        let items = vec!["Bach Ensemble", "Johann Sebastian Bach", "Bach"];
        let results = search_with_ranking(&items, "bach", |item| *item, 100);

        assert_eq!(
            results,
            vec!["Bach", "Johann Sebastian Bach", "Bach Ensemble"]
        );
    }

    #[test]
    fn album_search_falls_back_to_year() {
        let albums = vec![
            crate::library::models::Album {
                title: "Older Album".to_string(),
                year: Some(1999),
                ..Default::default()
            },
            crate::library::models::Album {
                title: "Newer Album".to_string(),
                year: Some(2024),
                ..Default::default()
            },
        ];

        let results = search_albums_with_ranking(&albums, "1999", 100);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Older Album");
    }

    #[test]
    fn track_search_ranks_title_before_artist() {
        let tracks = vec![
            crate::library::models::Track {
                title: "Something Else".to_string(),
                grandparent_title: Some("Blue Train".to_string()),
                ..Default::default()
            },
            crate::library::models::Track {
                title: "Blue Train".to_string(),
                grandparent_title: Some("John Coltrane".to_string()),
                ..Default::default()
            },
        ];

        let results = search_tracks_with_ranking(&tracks, "blue", 100);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Blue Train");
        assert_eq!(results[1].title, "Something Else");
    }
}

#[cfg(test)]
mod track_phrase_ranking_tests {
    use super::*;
    #[test]
    fn exact_multiword_track_titles_rank_ahead_of_metadata_matches() {
        use crate::library::models::Track;
        let tracks = vec![
            Track {
                rating_key: "metadata".into(),
                title: "Another Track".into(),
                grandparent_title: Some("Blue".into()),
                parent_title: Some("Moon".into()),
                ..Default::default()
            },
            Track {
                rating_key: "title".into(),
                title: "Blue Moon".into(),
                ..Default::default()
            },
        ];
        assert_eq!(
            search_tracks_with_ranking(&tracks, "Blue Moon", 10)[0].rating_key,
            "title"
        );
        assert!(search_tracks_with_ranking(&tracks, "Blue NOT Moon", 10).is_empty());
    }
}
