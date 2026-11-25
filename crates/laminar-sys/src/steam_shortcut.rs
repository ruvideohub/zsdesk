//! Recognise local Steam Internet shortcuts without treating ordinary web links as apps.
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub struct Shortcut { pub appid: String, pub icon_file: Option<PathBuf>, pub icon_index: i32 }

pub fn parse(text: &str) -> Option<Shortcut> {
    let mut section = false; let mut url = None; let mut icon = None; let mut index = 0;
    for line in text.lines() {
        let line=line.trim().trim_start_matches('\u{feff}');
        if line.starts_with('[') && line.ends_with(']') { section=line.eq_ignore_ascii_case("[InternetShortcut]"); continue; }
        if !section { continue; }
        let Some((key,value))=line.split_once('=') else {continue}; let value=value.trim().trim_matches('"');
        if key.trim().eq_ignore_ascii_case("URL") { url=Some(value); }
        else if key.trim().eq_ignore_ascii_case("IconFile") && !value.is_empty() { icon=Some(PathBuf::from(value)); }
        else if key.trim().eq_ignore_ascii_case("IconIndex") { index=value.parse().unwrap_or(0); }
    }
    let url=url?; let lower=url.to_ascii_lowercase();
    let appid=lower.strip_prefix("steam://rungameid/").or_else(||lower.strip_prefix("steam://run/"))?;
    if appid.is_empty() || !appid.bytes().all(|c|c.is_ascii_digit()) { return None; }
    Some(Shortcut{appid:appid.to_owned(),icon_file:icon,icon_index:index})
}

/// Cap shortcut size and accept UTF-8 or Windows UTF-16LE INI files.
pub fn read(path:&Path)->Option<Shortcut>{
    if !path.extension().is_some_and(|ext|ext.eq_ignore_ascii_case("url")) { return None; }
    let meta=std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len()>128*1024 { return None; }
    let bytes=std::fs::read(path).ok()?;
    let text=if bytes.starts_with(&[0xff,0xfe]) {
        if bytes.len()%2!=0{return None;}
        String::from_utf16(&bytes[2..].chunks_exact(2).map(|p|u16::from_le_bytes([p[0],p[1]])).collect::<Vec<_>>()).ok()?
    } else {String::from_utf8_lossy(&bytes).into_owned()};
    parse(&text)
}

/// Only use a local ICO explicitly named by Steam, never arbitrary URLs or network paths.
pub fn local_icon(shortcut:&Shortcut)->Option<&Path>{
    let path=shortcut.icon_file.as_deref()?;
    if shortcut.icon_index!=0 || !path.is_absolute() || path.to_string_lossy().starts_with("\\\\")
        || !path.extension().is_some_and(|e|e.eq_ignore_ascii_case("ico")) || !path.is_file() { return None; }
    Some(path)
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn only_game_links_are_accepted(){
        let s=parse("[InternetShortcut]\nURL=steam://rungameid/570\nIconFile=C:\\Steam\\570.ico\nIconIndex=0\n").unwrap();
        assert_eq!(s.icon_index,0);assert_eq!(s.icon_file.unwrap(),PathBuf::from(r"C:\Steam\570.ico"));
        for input in ["[InternetShortcut]\nURL=https://example.com", "[InternetShortcut]\nURL=steam://rungameid/../1", "[Other]\nURL=steam://rungameid/1", "[InternetShortcut]\nURL=steam://open/settings"] { assert!(parse(input).is_none()); }
    }
}
