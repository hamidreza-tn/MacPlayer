//! macOS Now Playing integration: publishes playback state to the system
//! Control Center surface and wires its remote commands back into the player.
//!
//! The system media keys (and the Control Center widget) route through
//! `MPRemoteCommandCenter`; commands arrive here as [`NowPlayingCommand`]s on
//! a channel the UI thread drains, so nothing touches egui state off-frame.
//! When this integration is enabled it owns the hardware media keys — the
//! in-app F7/F8/F9 keymap arms step aside to avoid double toggles.

#[cfg(target_os = "macos")]
pub use macos::{NowPlaying, NowPlayingInfo}; // used by player.rs

/// Commands delivered from the system Now Playing surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NowPlayingCommand {
    TogglePlayPause,
    NextTrack,
    PreviousTrack,
    SeekTo(f64),
}

#[cfg(not(target_os = "macos"))]
pub use stub::{NowPlaying, NowPlayingInfo};

#[cfg(not(target_os = "macos"))]
mod stub {
    use super::NowPlayingCommand;
    use std::sync::mpsc::Sender;

    #[derive(Default)]
    pub struct NowPlaying;

    #[derive(Debug, Clone, Default)]
    pub struct NowPlayingInfo {
        pub title: String,
        pub duration: f64,
        pub elapsed: f64,
        pub playing: bool,
    }

    impl NowPlaying {
        pub fn new(_tx: Sender<NowPlayingCommand>) -> Option<Self> {
            None
        }
        pub fn update(&self, _info: &NowPlayingInfo) {}
        pub fn clear(&self) {}
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos {
    use super::NowPlayingCommand;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSDictionary, NSNumber, NSString};
    use objc2_media_player::{
        MPChangePlaybackPositionCommandEvent, MPNowPlayingInfoCenter, MPNowPlayingPlaybackState,
        MPRemoteCommand, MPRemoteCommandCenter, MPRemoteCommandHandlerStatus,
    };
    use std::sync::mpsc::Sender;

    #[derive(Debug, Clone, Default)]
    pub struct NowPlayingInfo {
        pub title: String,
        pub duration: f64,
        pub elapsed: f64,
        pub playing: bool,
    }

    /// Owns the registered remote-command targets so they stay alive for the
    /// whole session.
    pub struct NowPlaying {
        _targets: Vec<Retained<AnyObject>>,
        center: Retained<MPNowPlayingInfoCenter>,
    }

    impl NowPlaying {
        /// Register the remote command handlers. Returns `None` if the
        /// MediaPlayer surface is unavailable (never on a normal macOS box).
        pub fn new(tx: Sender<NowPlayingCommand>) -> Option<Self> {
            unsafe {
                let center = MPNowPlayingInfoCenter::defaultCenter();
                let commands = MPRemoteCommandCenter::sharedCommandCenter();

                let toggle_tx = tx.clone();
                let toggle = register(&commands.togglePlayPauseCommand(), move |_event| {
                    let _ = toggle_tx.send(NowPlayingCommand::TogglePlayPause);
                    MPRemoteCommandHandlerStatus::Success
                });
                let next_tx = tx.clone();
                let next = register(&commands.nextTrackCommand(), move |_event| {
                    let _ = next_tx.send(NowPlayingCommand::NextTrack);
                    MPRemoteCommandHandlerStatus::Success
                });
                let prev_tx = tx.clone();
                let prev = register(&commands.previousTrackCommand(), move |_event| {
                    let _ = prev_tx.send(NowPlayingCommand::PreviousTrack);
                    MPRemoteCommandHandlerStatus::Success
                });
                let seek_tx = tx;
                let seek = register(&commands.changePlaybackPositionCommand(), move |event| {
                    // The event is only meaningful for position commands.
                    let event = event.cast::<MPChangePlaybackPositionCommandEvent>();
                    let position = event.as_ref().positionTime();
                    let _ = seek_tx.send(NowPlayingCommand::SeekTo(position));
                    MPRemoteCommandHandlerStatus::Success
                });

                Some(Self {
                    _targets: vec![toggle, next, prev, seek],
                    center,
                })
            }
        }

        /// Publish the current playback state. Cheap enough to call about
        /// once a second (the player throttles).
        pub fn update(&self, info: &NowPlayingInfo) {
            unsafe {
                self.center.setPlaybackState(if info.playing {
                    MPNowPlayingPlaybackState::Playing
                } else {
                    MPNowPlayingPlaybackState::Paused
                });
                let key_title = NSString::from_str("MPMediaItemPropertyTitle");
                let key_rate = NSString::from_str("MPNowPlayingInfoPropertyPlaybackRate");
                let key_duration = NSString::from_str("MPMediaItemPropertyPlaybackDuration");
                let key_elapsed = NSString::from_str("MPNowPlayingInfoPropertyElapsedPlaybackTime");
                let title = NSString::from_str(&info.title);
                let rate = NSNumber::numberWithDouble(if info.playing { 1.0 } else { 0.0 });
                let duration = NSNumber::numberWithDouble(info.duration.max(0.0));
                let elapsed = NSNumber::numberWithDouble(info.elapsed.max(0.0));
                // Upcast every value to the root object type for the dictionary.
                let title_any: Retained<AnyObject> = title.into_super().into_super();
                let rate_any: Retained<AnyObject> = rate.into_super().into_super().into();
                let duration_any: Retained<AnyObject> = duration.into_super().into_super().into();
                let elapsed_any: Retained<AnyObject> = elapsed.into_super().into_super().into();
                let dict_keys: [&NSString; 4] =
                    [&key_title, &key_rate, &key_duration, &key_elapsed];
                let dict_values: [&AnyObject; 4] =
                    [&title_any, &rate_any, &duration_any, &elapsed_any];
                let dict = NSDictionary::from_slices(&dict_keys, &dict_values);
                self.center.setNowPlayingInfo(Some(&dict));
            }
        }

        /// Wipe the Now Playing card (stop, quit, feature disabled).
        pub fn clear(&self) {
            unsafe {
                self.center.setNowPlayingInfo(None);
                self.center
                    .setPlaybackState(MPNowPlayingPlaybackState::Stopped);
            }
        }
    }

    /// Register a handler block on a remote command and keep the returned
    /// target token alive.
    unsafe fn register(
        command: &MPRemoteCommand,
        handler: impl Fn(NonNull<MPRemoteCommandEvent>) -> MPRemoteCommandHandlerStatus
        + Send
        + Sync
        + 'static,
    ) -> Retained<AnyObject> {
        use block2::RcBlock;
        let block = RcBlock::new(handler);
        unsafe { command.addTargetWithHandler(&block) }
    }

    use objc2_media_player::MPRemoteCommandEvent;
    use std::ptr::NonNull;
}

/// Command channel for the integration (both platform variants).
pub fn channel() -> (
    std::sync::mpsc::Sender<NowPlayingCommand>,
    std::sync::mpsc::Receiver<NowPlayingCommand>,
) {
    std::sync::mpsc::channel()
}
