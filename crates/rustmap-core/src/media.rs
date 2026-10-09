use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Stable identifier of a media item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MediaId(pub u32);

/// Where a media item's pixels come from. Video, cameras, Syphon and shaders
/// join in later milestones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MediaSource {
    Image {
        path: PathBuf,
    },
    /// A live NDI source, by its full network name, e.g. `"STUDIO (Resolume)"`.
    Ndi {
        source: String,
    },
    Video {
        path: PathBuf,
        #[serde(default)]
        playback: Playback,
    },
}

/// How a video plays. Saved with the show, so a production file starts
/// playing the way it was set up.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Playback {
    pub paused: bool,
    pub looping: bool,
    /// 1.0 = normal speed.
    pub speed: f32,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            paused: false,
            looping: true,
            speed: 1.0,
        }
    }
}

impl MediaSource {
    /// The part that decides what has to be (re)loaded: changing playback
    /// settings must not restart the decoder.
    pub fn load_key(&self) -> MediaSource {
        match self {
            MediaSource::Video { path, .. } => MediaSource::Video {
                path: path.clone(),
                playback: Playback::default(),
            },
            other => other.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Media {
    pub id: MediaId,
    pub name: String,
    pub source: MediaSource,
}

impl Media {
    /// File path for file-based media.
    pub fn path(&self) -> Option<&std::path::Path> {
        match &self.source {
            MediaSource::Image { path } | MediaSource::Video { path, .. } => Some(path),
            MediaSource::Ndi { .. } => None,
        }
    }

    pub fn path_mut(&mut self) -> Option<&mut PathBuf> {
        match &mut self.source {
            MediaSource::Image { path } | MediaSource::Video { path, .. } => Some(path),
            MediaSource::Ndi { .. } => None,
        }
    }

    pub fn is_video(&self) -> bool {
        matches!(self.source, MediaSource::Video { .. })
    }

    /// Where it comes from, for tooltips.
    pub fn origin(&self) -> String {
        match &self.source {
            MediaSource::Image { path } | MediaSource::Video { path, .. } => {
                path.display().to_string()
            }
            MediaSource::Ndi { source } => format!("NDI: {source}"),
        }
    }

    pub fn is_live(&self) -> bool {
        matches!(self.source, MediaSource::Ndi { .. })
    }
}
