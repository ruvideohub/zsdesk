//! Size-limited streaming response collection, independent of WinHTTP.
use std::io;

const MAX_BODY: usize = 16 * 1024 * 1024;

/// A read error is not an end-of-file: partial responses must never be
/// returned as successful. The caller maps network errors without logging URLs.
pub(crate) fn read_bounded(mut read: impl FnMut(&mut [u8]) -> io::Result<usize>) -> io::Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut buffer = [0u8; 32 * 1024];
    loop {
        let size = read(&mut buffer)?;
        if size == 0 { return Ok(result); }
        if size > buffer.len() || size > MAX_BODY.saturating_sub(result.len()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "response too large or invalid"));
        }
        result.extend_from_slice(&buffer[..size]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collects_chunks_until_eof() {
        let mut chunks: &[&[u8]] = &[b"first", b" second", b""];
        let result = read_bounded(|buffer| {
            let item = chunks[0]; chunks = &chunks[1..];
            buffer[..item.len()].copy_from_slice(item); Ok(item.len())
        }).unwrap();
        assert_eq!(result, b"first second");
    }
    #[test]
    fn read_error_after_partial_data_is_not_success() {
        let mut first = true;
        let result = read_bounded(|buffer| {
            if first { first = false; buffer[..3].copy_from_slice(b"abc"); Ok(3) }
            else { Err(io::Error::new(io::ErrorKind::TimedOut, "injected timeout")) }
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }
    #[test]
    fn rejects_body_above_limit_and_impossible_read_count() {
        let mut remaining = MAX_BODY + 1;
        let result = read_bounded(|buffer| {
            let size = remaining.min(buffer.len()); remaining -= size; Ok(size)
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert!(read_bounded(|buffer| Ok(buffer.len() + 1)).is_err());
    }
    #[test]
    fn accepts_body_exactly_at_limit() {
        let mut remaining = MAX_BODY;
        let result = read_bounded(|buffer| {
            let size = remaining.min(buffer.len()); remaining -= size; Ok(size)
        }).unwrap();
        assert_eq!(result.len(), MAX_BODY);
    }
}
