//! Radio widget state. Network work is owned by Hub; media is controlled on UI thread.
use crate::radio_player::{RadioPlayer, State};
use crate::radio_playlist::Station;

#[derive(Default)]
pub struct Radio {
    pub stations: Vec<Station>,
    pub index: usize,
    pub loading: bool,
    pub pending_play: bool,
    pub error: bool,
    pub import_error: bool,
    pub nested: bool,
    player: RadioPlayer,
}
impl Radio {
    pub fn name(&self) -> &str {
        self.stations
            .get(self.index)
            .map(|s| s.name.as_str())
            .unwrap_or("Online Radio")
    }
    pub fn state(&self) -> State {
        self.player.state()
    }
    pub fn caption(&self) -> &'static str {
        if self.loading {
            return "Loading stations…";
        }
        if self.import_error { return "Playlist import failed"; }
        if self.nested {
            return "Playlist stream unsupported";
        }
        if self.error || self.player.state() == State::Failed {
            return "Radio unavailable — retry";
        }
        match self.player.state() {
            State::Opening => "Connecting…",
            State::Playing => "Live radio",
            State::Paused => "Paused",
            _ => "Click play to load stations",
        }
    }
    /// Returns true when the catalogue must be fetched off the UI thread.
    pub fn toggle(&mut self) -> bool {
        if self.loading {
            self.pending_play = !self.pending_play;
            return false;
        }
        if self.stations.is_empty() {
            self.loading = true;
            self.pending_play = true;
            self.error = false;
            self.import_error = false;
            return true;
        }
        match self.player.state() {
            State::Playing | State::Opening => {
                if self.player.pause().is_err() {
                    self.error = true;
                }
            }
            State::Paused => {
                if self.player.resume().is_err() {
                    self.error = true;
                }
            }
            _ => self.play_selected(),
        }
        false
    }
    pub fn finish_loading(&mut self, result: Option<Vec<Station>>) {
        self.loading = false;
        match result {
            Some(s) if !s.is_empty() => {
                self.stations = s;
                self.index = 0;
                self.error = false;
                if self.pending_play {
                    self.play_selected();
                }
            }
            _ => self.error = true,
        }
        self.pending_play = false;
    }
    pub fn select(&mut self, index: usize) {
        if index < self.stations.len() { self.index = index; self.play_selected(); }
    }
    pub fn replace_catalogue(&mut self, stations: Vec<Station>) {
        self.player.stop();
        self.stations = stations;
        self.index = 0;
        self.loading = false;
        self.pending_play = false;
        self.error = false;
        self.import_error = false;
        self.nested = false;
    }
    pub fn next(&mut self, forward: bool) {
        if let Some(i) = crate::radio_queue::step(self.index, self.stations.len(), forward) {
            self.index = i;
            self.play_selected();
        }
    }
    fn play_selected(&mut self) {
        self.error = false;
        self.import_error = false;
        self.nested = false;
        let Some(s) = self.stations.get(self.index) else {
            return;
        };
        if crate::radio_queue::nested_playlist(&s.url) {
            self.player.stop();
            self.nested = true;
            return;
        }
        if self.player.start(&s.url).is_err() {
            self.error = true;
        }
    }
}
