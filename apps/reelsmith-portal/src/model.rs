//! Review-portal documents. Edit math uses [`MediaTick`], which is a
//! [`reelsmith_time::Tick`] stored as a decimal string so a JSON number cannot round it.

use crate::error::Error;
use reelsmith_time::{TICKS_PER_SECOND, Tick};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_CLIPS: usize = 2_000;
pub const MAX_CAPTIONS: usize = 2_000;
pub const MAX_COMMENTS: usize = 2_000;
pub const MAX_EVENTS: usize = 50_000;
pub const MAX_TEXT: usize = 2_000;
pub const MAX_NAME: usize = 120;

/// Integer media time at [`TICKS_PER_SECOND`]. Serialized as a string.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MediaTick(pub i64);

impl Default for MediaTick {
    fn default() -> Self {
        Self::ZERO
    }
}

impl MediaTick {
    pub const ZERO: Self = Self(0);

    pub fn from_tick(tick: Tick) -> Self {
        Self(tick.0)
    }

    pub fn to_tick(self) -> Tick {
        Tick(self.0)
    }

    pub fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    pub fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    pub fn checked_add(self, other: Self) -> Result<Self, Error> {
        self.0.checked_add(other.0).map(Self).ok_or_else(|| Error::invalid("time overflow"))
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, Error> {
        self.0.checked_sub(other.0).map(Self).ok_or_else(|| Error::invalid("time underflow"))
    }
}

impl fmt::Debug for MediaTick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MediaTick({})", self.0)
    }
}

impl Serialize for MediaTick {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for MediaTick {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit;
        impl<'de> serde::de::Visitor<'de> for Visit {
            type Value = MediaTick;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a tick count as a string or integer")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<MediaTick, E> {
                value.parse::<i64>().map(MediaTick).map_err(E::custom)
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<MediaTick, E> {
                Ok(MediaTick(value))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<MediaTick, E> {
                i64::try_from(value).map(MediaTick).map_err(E::custom)
            }
        }
        deserializer.deserialize_any(Visit)
    }
}

/// 0..=1000, a fraction of the frame width or height.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Norm(pub u16);

impl Norm {
    pub fn new(value: u16) -> Result<Self, Error> {
        if value > 1000 { Err(Error::invalid("a frame fraction must be between 0 and 1000")) } else { Ok(Self(value)) }
    }
}

/// Linear gain in thousandths. 1000 is unity. The cap is 4.000.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gain(pub u16);

impl Gain {
    pub const UNITY: Self = Self(1000);

    pub fn new(value: u16) -> Result<Self, Error> {
        if value > 4000 { Err(Error::invalid("gain must be between 0 and 4000 permille")) } else { Ok(Self(value)) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: Norm,
    pub y: Norm,
    pub w: Norm,
    pub h: Norm,
}

impl Region {
    pub fn new(x: u16, y: u16, w: u16, h: u16) -> Result<Self, Error> {
        if w == 0 || h == 0 {
            return Err(Error::invalid("a frame region needs a width and a height"));
        }
        let x = Norm::new(x)?;
        let y = Norm::new(y)?;
        let w = Norm::new(w)?;
        let h = Norm::new(h)?;
        if u32::from(x.0) + u32::from(w.0) > 1000 || u32::from(y.0) + u32::from(h.0) > 1000 {
            return Err(Error::invalid("a frame region has to stay inside the picture"));
        }
        Ok(Self { x, y, w, h })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Actor {
    Client { client_id: String, name: String },
    Agent,
    Admin,
}

impl Actor {
    pub fn label(&self) -> String {
        match self {
            Self::Client { name, .. } => name.clone(),
            Self::Agent => "Agent".to_string(),
            Self::Admin => "Admin".to_string(),
        }
    }

    pub fn is_agent(&self) -> bool {
        matches!(self, Self::Agent)
    }

    pub fn is_client(&self) -> bool {
        matches!(self, Self::Client { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalType {
    Reel,
    Ad,
    LongForm,
}

impl FinalType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reel => "reel",
            Self::Ad => "ad",
            Self::LongForm => "long_form",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Reel => "Reel",
            Self::Ad => "Ad",
            Self::LongForm => "Long-form",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipKind {
    Video,
    Audio,
    Music,
    Broll,
    Title,
    EndCard,
}

impl ClipKind {
    pub fn is_picture(self) -> bool {
        matches!(self, Self::Video | Self::Broll | Self::Title | Self::EndCard)
    }

    pub fn is_sound(self) -> bool {
        matches!(self, Self::Audio | Self::Music)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AspectSource {
    /// Majority of the raw footage, by duration.
    Raw,
    /// The client profile named an output aspect.
    Profile,
    /// Someone pressed Change format. Reframing and caption moves are part of that event.
    ChangeFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Draft,
    AgentWorking,
    Ready,
    ChangesRequested,
    Approved,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Draft => "Draft",
            Self::AgentWorking => "Editor working",
            Self::Ready => "Ready for review",
            Self::ChangesRequested => "Changes requested",
            Self::Approved => "Approved",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Good,
    NotGood,
}

impl Mark {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::NotGood => "not_good",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub mark: Mark,
    pub note: String,
    pub by: String,
    pub at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    pub track: String,
    pub name: String,
    pub kind: ClipKind,
    pub start: MediaTick,
    pub duration: MediaTick,
    pub source_in: MediaTick,
    pub source_duration: MediaTick,
    pub gain_permille: Gain,
    pub muted: bool,
    pub linked: Option<String>,
    pub text: String,
    pub source_width: u32,
    pub source_height: u32,
}

impl Clip {
    pub fn end(&self) -> MediaTick {
        self.start.saturating_add(self.duration)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caption {
    pub id: String,
    pub start: MediaTick,
    pub duration: MediaTick,
    pub text: String,
    /// Thousandths down the frame. 0 is the top.
    pub anchor_y: Norm,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Word {
    pub start: MediaTick,
    pub end: MediaTick,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    pub author: String,
    pub at: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub start: MediaTick,
    pub end: MediaTick,
    pub region: Option<Region>,
    pub body: String,
    pub resolved: bool,
    pub fixed_in: Option<String>,
    pub replies: Vec<Reply>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryItem {
    pub id: String,
    pub name: String,
    pub role: MediaRole,
    pub width: u32,
    pub height: u32,
    pub duration: MediaTick,
    /// Selected on this project's brief. Reviewers do not receive the library.
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaRole {
    Aroll,
    Broll,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timeline {
    pub fps_num: u32,
    pub fps_den: u32,
    pub width: u32,
    pub height: u32,
    pub aspect_source: AspectSource,
    pub snap_words: bool,
    pub reframed: bool,
    pub next_seq: u64,
    pub clips: Vec<Clip>,
    pub captions: Vec<Caption>,
    pub words: Vec<Word>,
    pub library: Vec<LibraryItem>,
}

impl Timeline {
    pub fn blank(width: u32, height: u32, source: AspectSource) -> Result<Self, Error> {
        if width == 0 || height == 0 || width > 7680 || height > 7680 {
            return Err(Error::invalid("picture size must be between 1 and 7680 on each side"));
        }
        let timeline = Self {
            fps_num: 30,
            fps_den: 1,
            width,
            height,
            aspect_source: source,
            snap_words: true,
            reframed: source == AspectSource::ChangeFormat,
            next_seq: 1,
            clips: Vec::new(),
            captions: Vec::new(),
            words: Vec::new(),
            library: Vec::new(),
        };
        timeline.frame_ticks()?;
        Ok(timeline)
    }

    pub fn frame_ticks(&self) -> Result<i64, Error> {
        if self.fps_num == 0 || self.fps_den == 0 {
            return Err(Error::invalid("frame rate cannot be zero"));
        }
        let num = i64::from(self.fps_num);
        let den = i64::from(self.fps_den);
        let ticks = TICKS_PER_SECOND.checked_mul(den).ok_or_else(|| Error::invalid("frame rate overflow"))?;
        if ticks % num != 0 {
            return Err(Error::invalid("frame rate does not divide the time base"));
        }
        let frame = ticks / num;
        if frame <= 0 {
            return Err(Error::invalid("frame duration is empty"));
        }
        Ok(frame)
    }

    pub fn frame_to_tick(&self, frame: i64) -> Result<MediaTick, Error> {
        let size = self.frame_ticks()?;
        frame.checked_mul(size).map(MediaTick).ok_or_else(|| Error::invalid("frame overflow"))
    }

    pub fn tick_to_frame(&self, tick: MediaTick) -> Result<i64, Error> {
        let size = self.frame_ticks()?;
        Ok(tick.0 / size)
    }

    pub fn alloc_id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}{}", self.next_seq);
        self.next_seq = self.next_seq.saturating_add(1);
        id
    }

    pub fn clip(&self, id: &str) -> Result<&Clip, Error> {
        self.clips.iter().find(|clip| clip.id == id).ok_or_else(|| Error::not_found(format!("no clip {id}")))
    }

    pub fn clip_mut(&mut self, id: &str) -> Result<&mut Clip, Error> {
        self.clips.iter_mut().find(|clip| clip.id == id).ok_or_else(|| Error::not_found(format!("no clip {id}")))
    }

    pub fn duration(&self) -> MediaTick {
        self.clips.iter().map(Clip::end).max().unwrap_or(MediaTick::ZERO)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub timeline: Timeline,
    pub comments: Vec<Comment>,
    pub status: Status,
    pub approval: Option<Approval>,
    pub decisions: String,
    pub final_type: FinalType,
}

impl Snapshot {
    pub fn new(timeline: Timeline, final_type: FinalType) -> Self {
        Self { timeline, comments: Vec::new(), status: Status::Draft, approval: None, decisions: String::new(), final_type }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditEvent {
    pub id: u64,
    pub at: String,
    pub actor: Actor,
    pub action: String,
    pub targets: Vec<String>,
    pub summary: String,
    pub reason: Option<String>,
    /// The state this event was applied on top of. `None` is the base snapshot.
    pub based_on: Option<u64>,
    pub relates_to: Option<u64>,
    pub before: Snapshot,
    pub after: Snapshot,
}

pub fn clean_text(value: &str, limit: usize, label: &str) -> Result<String, Error> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(Error::invalid(format!("{label} is empty")));
    }
    if trimmed.chars().count() > limit {
        return Err(Error::invalid(format!("{label} is longer than {limit} characters")));
    }
    if trimmed.chars().any(|ch| ch == '\0' || (ch.is_control() && ch != '\n' && ch != '\t')) {
        return Err(Error::invalid(format!("{label} contains a control character")));
    }
    Ok(trimmed.to_string())
}

pub fn clean_name(value: &str, label: &str) -> Result<String, Error> {
    clean_text(value, MAX_NAME, label)
}

/// Actions that change a clip, a caption, or the sequence itself.
pub fn is_picture_edit(action: &str) -> bool {
    matches!(
        action,
        action::TRIM
            | action::RAZOR
            | action::MOVE
            | action::LIFT
            | action::GAIN
            | action::LENGTH
            | action::CAPTION
            | action::CAPTION_MOVE
            | action::FORMAT
            | action::ASSEMBLE
            | action::PLACE_BROLL
    )
}

pub mod action {
    pub const TRIM: &str = "timeline.trim";
    pub const RAZOR: &str = "timeline.razor";
    pub const MOVE: &str = "timeline.move";
    pub const LIFT: &str = "timeline.lift";
    pub const GAIN: &str = "clip.gain";
    pub const LENGTH: &str = "timeline.length";
    pub const CAPTION: &str = "caption.edit";
    pub const CAPTION_MOVE: &str = "caption.move";
    pub const SNAP: &str = "portal.snap";
    pub const FORMAT: &str = "portal.change_format";
    pub const COMMENT: &str = "portal.comment";
    pub const RESOLVE: &str = "portal.resolve_comment";
    pub const REPLY: &str = "portal.reply";
    pub const APPROVE: &str = "portal.approve";
    pub const REQUEST: &str = "portal.request_changes";
    pub const DECISIONS: &str = "portal.decisions";
    pub const FINAL_TYPE: &str = "portal.final_type";
    pub const STATUS: &str = "portal.status";
    pub const UNDO: &str = "portal.undo";
    pub const REDO: &str = "portal.redo";
    pub const RESTORE: &str = "portal.restore";
    pub const ASSEMBLE: &str = "portal.assemble";
    pub const PLACE_BROLL: &str = "portal.place_broll";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    In,
    Out,
}
