//! Review-portal records: who did what, versions' status and approval, comment threads and the
//! B-roll library. The cut itself is the engine's `.fcproj`; nothing here models a timeline.
//! Times use [`MediaTick`], a [`reelsmith_time::Tick`] stored as a decimal string so a JSON
//! number cannot round it.

use crate::error::Error;
use reelsmith_time::Tick;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_COMMENTS: usize = 2_000;
pub const MAX_ACTIONS: usize = 200_000;
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

/// A box on the picture in thousandths of the frame (0..=1000 on each axis).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Region {
    pub fn new(x: u16, y: u16, w: u16, h: u16) -> Result<Self, Error> {
        Self { x, y, w, h }.checked()
    }

    pub fn checked(self) -> Result<Self, Error> {
        if self.w == 0 || self.h == 0 {
            return Err(Error::invalid("a frame region needs a width and a height"));
        }
        if self.x > 1000 || self.y > 1000 || self.w > 1000 || self.h > 1000 {
            return Err(Error::invalid("a frame fraction must be between 0 and 1000"));
        }
        if u32::from(self.x) + u32::from(self.w) > 1000 || u32::from(self.y) + u32::from(self.h) > 1000 {
            return Err(Error::invalid("a frame region has to stay inside the picture"));
        }
        Ok(self)
    }
}

/// Who did something, as the server knows it from the link token (never from the page).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Client,
    Agent,
    Admin,
    Watcher,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub kind: ActorKind,
    pub id: String,
    pub name: String,
}

impl Actor {
    pub fn client(id: &str, name: &str) -> Self {
        Self { kind: ActorKind::Client, id: id.to_string(), name: name.to_string() }
    }

    pub fn agent() -> Self {
        Self { kind: ActorKind::Agent, id: "agent".to_string(), name: "Editor".to_string() }
    }

    pub fn watcher() -> Self {
        Self { kind: ActorKind::Watcher, id: "watcher".to_string(), name: "Watcher".to_string() }
    }

    pub fn is_client(&self) -> bool {
        self.kind == ActorKind::Client
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
pub enum AspectSource {
    /// Majority of the raw footage, by duration.
    Raw,
    /// The client profile named an output aspect.
    Profile,
    /// Someone pressed Change format. Reframing and caption moves are part of that event.
    ChangeFormat,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Draft,
    AgentWorking,
    Ready,
    ChangesRequested,
    Approved,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::AgentWorking => "agent_working",
            Self::Ready => "ready",
            Self::ChangesRequested => "changes_requested",
            Self::Approved => "approved",
        }
    }

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
pub struct Reply {
    pub author: Actor,
    pub at: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub author: Actor,
    pub at: String,
    pub start_tick: MediaTick,
    pub end_tick: MediaTick,
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
