use std::path::Path;
use sha2::{Digest, Sha256};

pub fn key(profile: &Path) -> String {
    let path = profile.to_string_lossy().replace('/', "\\").to_lowercase();
    format!("{:x}", Sha256::digest(path.as_bytes()))
}

/// Quotes one argument using the Windows command-line parsing rules.
pub fn quote_argument(value: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;
    for ch in value.chars() {
        if ch == '\\' { slashes += 1; continue; }
        result.extend(std::iter::repeat_n('\\', if ch == '"' { slashes * 2 + 1 } else { slashes }));
        result.push(ch);
        slashes = 0;
    }
    result.extend(std::iter::repeat_n('\\', slashes * 2));
    result.push('"');
    result
}
