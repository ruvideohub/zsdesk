//! Windows media backend for a single internet-radio stream.
//! Must be created and controlled on the Dock's WinRT-initialized thread.
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use windows::{
    core::{Result, HSTRING},
    Foundation::{TypedEventHandler, Uri},
    Media::{
        Core::MediaSource,
        Playback::{MediaPlaybackState, MediaPlayer},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Stopped,
    Opening,
    Playing,
    Paused,
    Failed,
}

pub struct RadioPlayer {
    player: Option<MediaPlayer>,
    // The low bit is the failure flag. Callbacks for an old player cannot poison
    // a newly selected station, even when they arrive after it was closed.
    generation: u64,
    status: Arc<AtomicU64>,
}

impl Default for RadioPlayer {
    fn default() -> Self {
        Self {
            player: None,
            generation: 0,
            status: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl RadioPlayer {
    /// Starts a web stream. A successful return means opening started, not that
    /// the URL, codec or HLS stream will actually play. Poll `state()` afterward.
    pub fn start(&mut self, url: &str) -> Result<()> {
        self.stop();
        let uri = Uri::CreateUri(&HSTRING::from(url))?;
        let scheme = uri.SchemeName()?.to_string_lossy().to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x80070057u32 as i32,
            )));
        }
        let source = MediaSource::CreateFromUri(&uri)?;
        let player = MediaPlayer::new()?;
        let expected = self.generation;
        let status = self.status.clone();
        let started = (|| {
            player.SetAutoPlay(false)?;
            player.MediaFailed(&TypedEventHandler::new(move |_, _| {
                crate::radio_generation::record_failure(&status, expected);
                Ok(())
            }))?;
            player.SetSource(&source)?;
            player.Play()?;
            Ok(())
        })();
        if let Err(error) = started {
            let _ = player.Close();
            return Err(error);
        }
        self.player = Some(player);
        Ok(())
    }
    pub fn pause(&self) -> Result<()> {
        if let Some(p) = &self.player {
            p.Pause()?;
        }
        Ok(())
    }
    pub fn resume(&self) -> Result<()> {
        if let Some(p) = &self.player {
            p.Play()?;
        }
        Ok(())
    }
    pub fn stop(&mut self) {
        self.generation = self.generation.wrapping_add(2) & !1;
        self.status.store(self.generation, Ordering::SeqCst);
        if let Some(p) = self.player.take() {
            let _ = p.Close();
        }
    }
    pub fn state(&self) -> State {
        let Some(p) = &self.player else {
            return State::Stopped;
        };
        if self.status.load(Ordering::SeqCst) == (self.generation | 1) {
            return State::Failed;
        }
        match p.PlaybackSession().and_then(|s| s.PlaybackState()) {
            Ok(MediaPlaybackState::Playing) => State::Playing,
            Ok(MediaPlaybackState::Paused) => State::Paused,
            Ok(
                MediaPlaybackState::Opening
                | MediaPlaybackState::Buffering
                | MediaPlaybackState::None,
            ) => State::Opening,
            _ => State::Failed,
        }
    }
}
impl Drop for RadioPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}
