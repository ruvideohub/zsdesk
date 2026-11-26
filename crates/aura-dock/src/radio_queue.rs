//! Pure policy for navigating the station list and detecting nested playlists.
pub fn step(index: usize, len: usize, forward: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let i = index.min(len - 1);
    Some(if forward {
        (i + 1) % len
    } else {
        (i + len - 1) % len
    })
}
/// HLS m3u8 is passed to Windows Media; ordinary M3U/PLS need a resolver.
pub fn nested_playlist(url: &str) -> bool {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    path.ends_with(".m3u") || path.ends_with(".pls")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_and_nested_urls() {
        assert_eq!(step(0, 0, true), None);
        assert_eq!(step(0, 3, false), Some(2));
        assert_eq!(step(2, 3, true), Some(0));
        assert!(nested_playlist("https://x/LIST.PLS?auth=1"));
        assert!(nested_playlist("http://x/radio.m3u"));
        assert!(!nested_playlist("https://x/live.m3u8?a=1"));
    }
}
