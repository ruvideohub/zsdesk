//! Result ordering shared by the index, the scan and the search panel.
//!
//! This used to live in the Everything client; the panel and the native index only ever
//! needed the four orderings, never the backend.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchSort { #[default] NameAsc, NameDesc, ModifiedDesc, SizeDesc }
