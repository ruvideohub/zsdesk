//! Bounded parsing of station catalogues. This does not fetch or play URLs.
//! A station may itself be a playlist or HLS URL: the caller must not assume
//! that every URL is directly playable, or log URLs containing access tokens.
use std::collections::HashSet;

pub const MAX_PLAYLIST_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_STATIONS: usize = 10_000;
const MAX_URL_BYTES: usize = 8192;
const MAX_NAME_CHARS: usize = 256;

#[derive(Clone, PartialEq, Eq)]
pub struct Station {
    pub name: String,
    pub url: String,
}

// Deliberately omit URLs from Debug: station URLs often carry short-lived tokens.
impl std::fmt::Debug for Station {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Station")
            .field("name", &self.name)
            .field("url", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    TooLarge,
    InvalidUtf8,
    NotStationPlaylist,
    TooManyStations,
}

/// Parse an extended M3U station directory, preserving order and exact URLs.
/// Deduplicate identical URLs only: labels may repeat for genuinely different streams.
/// Reject HLS media/master playlists rather than presenting segments as stations.
pub fn parse(bytes: &[u8]) -> Result<Vec<Station>, Error> {
    if bytes.len() > MAX_PLAYLIST_BYTES {
        return Err(Error::TooLarge);
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::InvalidUtf8)?
        .trim_start_matches('\u{feff}');
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("#EXTM3U") {
        return Err(Error::NotStationPlaylist);
    }
    let mut found_hls = false;
    let mut name: Option<String> = None;
    let mut seen = HashSet::new();
    let mut stations = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.starts_with("#EXT-X-") {
            found_hls = true;
            break;
        }
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            name = extinf_title(info);
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let title = name.take();
        let Some(title) = title else {
            continue;
        };
        if !is_web_url(line) || !seen.insert(line.to_owned()) {
            continue;
        }
        if stations.len() == MAX_STATIONS {
            return Err(Error::TooManyStations);
        }
        stations.push(Station {
            name: title,
            url: line.to_owned(),
        });
    }
    if found_hls {
        return Err(Error::NotStationPlaylist);
    }
    Ok(stations)
}

/// A local user-selected M3U may be plain (no header or EXTINF names).
/// Never expose query strings as labels, and do not treat HLS segments as stations.
pub fn parse_user(bytes: &[u8]) -> Result<Vec<Station>, Error> {
    if bytes.len() > MAX_PLAYLIST_BYTES { return Err(Error::TooLarge); }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::InvalidUtf8)?.trim_start_matches('\u{feff}');
    let mut name: Option<String> = None;
    let mut seen = HashSet::new();
    let mut stations = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("#EXT-X-") { return Err(Error::NotStationPlaylist); }
        if let Some(info) = line.strip_prefix("#EXTINF:") { name = extinf_title(info); continue; }
        if line.is_empty() || line.starts_with('#') { continue; }
        let title = name.take();
        if !is_web_url(line) || !seen.insert(line.to_owned()) { continue; }
        if stations.len() == MAX_STATIONS { return Err(Error::TooManyStations); }
        let host = line.split_once("://").map(|(_, s)| s.split(['/', '?', '#']).next().unwrap_or("Station")).unwrap_or("Station");
        stations.push(Station { name: title.unwrap_or_else(|| host.chars().take(MAX_NAME_CHARS).collect()), url: line.to_owned() });
    }
    if stations.is_empty() { return Err(Error::NotStationPlaylist); }
    Ok(stations)
}

/// The title follows the first comma outside quoted attributes.
fn extinf_title(info: &str) -> Option<String> {
    let mut quoted = false;
    let mut comma = None;
    for (i, c) in info.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                comma = Some(i);
                break;
            }
            _ => {}
        }
    }
    let title = info.get(comma? + 1..)?.trim();
    if title.is_empty() {
        return None;
    }
    Some(
        title
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_NAME_CHARS)
            .collect(),
    )
}

fn is_web_url(s: &str) -> bool {
    if s.len() > MAX_URL_BYTES || s.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    let Some(rest) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    // No URL credentials; they can be leaked in diagnostics or redirected.
    !host.is_empty() && !host.contains('@') && !host.starts_with(':')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_titles_quotes_exact_url_dedup_and_order() {
        let text = "\u{feff}#EXTM3U\r\n#EXTINF:-1 group-title=\"One, Two\", Радио Ваня\r\nhttps://example.test/live?token=a\r\n#EXTINF:-1, Duplicate\r\nhttps://example.test/live?token=a\r\n#EXTINF:-1, Радио Ваня\r\nhttp://example.test/other\r\n";
        let s = parse(text.as_bytes()).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].name, "Радио Ваня");
        assert_eq!(s[0].url, "https://example.test/live?token=a");
        assert_eq!(s[1].url, "http://example.test/other");
        assert!(!format!("{:?}", s[0]).contains("token=a"));
    }
    #[test]
    fn rejects_hls_and_untrusted_schemes_or_credentials() {
        assert_eq!(
            parse(b"#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6,\nhttps://x/seg.ts"),
            Err(Error::NotStationPlaylist)
        );
        let stations = parse(b"#EXTM3U\n#EXTINF:-1,Local\nfile:///a\n#EXTINF:-1,Auth\nhttp://u:p@host/a\n#EXTINF:-1,No host\nhttp:///a\n").unwrap();
        assert!(stations.is_empty());
    }
    #[test]
    fn malformed_data_is_bounded_and_never_uses_stale_title() {
        assert_eq!(
            parse(&vec![0; MAX_PLAYLIST_BYTES + 1]),
            Err(Error::TooLarge)
        );
        assert_eq!(parse(b"#EXTM3U\n\xff"), Err(Error::InvalidUtf8));
        assert_eq!(parse(b"no header"), Err(Error::NotStationPlaylist));
        assert!(
            parse(b"#EXTM3U\n#EXTINF:-1,One\nfile:///bad\nhttps://x/unlabelled\n")
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn imported_plain_playlists_and_hls_safety() {
        let s = parse_user(b"https://one.example/live?token=secret\n#EXTINF:-1,Named\nhttp://two.example/radio\nhttps://one.example/live?token=secret\n").unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].name, "one.example");
        assert_eq!(s[1].name, "Named");
        assert!(!s[0].name.contains("secret"));
        assert_eq!(parse_user(b"#EXTM3U\n#EXT-X-TARGETDURATION:6\nhttps://x/segment.ts"), Err(Error::NotStationPlaylist));
        assert_eq!(parse_user(b"not a playlist"), Err(Error::NotStationPlaylist));
    }
    #[test]
    fn too_many_stations_is_an_error_not_silent_truncation() {
        let mut data = String::from("#EXTM3U\n");
        for i in 0..=MAX_STATIONS {
            data.push_str(&format!("#EXTINF:-1,Name\nhttps://x/{i}\n"));
        }
        assert_eq!(parse(data.as_bytes()), Err(Error::TooManyStations));
    }
}
