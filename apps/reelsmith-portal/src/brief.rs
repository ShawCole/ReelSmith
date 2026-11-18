//! Upload brief: final type, target length, version count, B-roll, and the aspect rule.
//!
//! The output aspect matches the raw footage unless the client profile names one, or the person
//! presses Change format. There is no under-60-second rule unless the target length asks for it.
//! B-roll never shares a file with the raw A-roll.

use crate::error::Error;
use crate::model::{AspectSource, FinalType, MAX_NAME, Mark, MediaTick, Snapshot, clean_name};
use reelsmith_time::TICKS_PER_SECOND;
use serde::{Deserialize, Serialize};

pub const MAX_VERSIONS: u32 = 12;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawFile {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub duration: MediaTick,
    pub role: crate::model::MediaRole,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LengthTarget {
    Unspecified,
    Exact { ticks: MediaTick },
    Range { min: MediaTick, max: MediaTick },
}

impl LengthTarget {
    pub fn label(&self) -> String {
        match self {
            Self::Unspecified => "Profile default".to_string(),
            Self::Exact { ticks } => format!("{}s", ticks.0 / TICKS_PER_SECOND),
            Self::Range { min, max } => format!("{}–{}s", min.0 / TICKS_PER_SECOND, max.0 / TICKS_PER_SECOND),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadBrief {
    pub project_name: String,
    pub final_type: FinalType,
    #[serde(default)]
    pub target: Option<LengthTarget>,
    /// Omitted means 1.
    #[serde(default = "one")]
    pub version_count: u32,
    /// Difference labels, one per version. A single blank label becomes the primary cut.
    #[serde(default)]
    pub differences: Vec<String>,
    #[serde(default)]
    pub use_broll: bool,
    #[serde(default)]
    pub broll_library_ids: Vec<String>,
    #[serde(default)]
    pub broll_uploads: Vec<RawFile>,
    #[serde(default)]
    pub raw_files: Vec<RawFile>,
    #[serde(default)]
    pub change_format: bool,
    #[serde(default)]
    pub explicit_width: Option<u32>,
    #[serde(default)]
    pub explicit_height: Option<u32>,
    /// From the client profile, when it names an output aspect.
    #[serde(default)]
    pub profile_width: Option<u32>,
    #[serde(default)]
    pub profile_height: Option<u32>,
}

fn one() -> u32 {
    1
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AspectRatio {
    pub width: u32,
    pub height: u32,
}

impl AspectRatio {
    pub fn reduced(width: u32, height: u32) -> Result<Self, Error> {
        if width == 0 || height == 0 || width > 7680 || height > 7680 {
            return Err(Error::invalid("footage width and height have to be between 1 and 7680"));
        }
        let divisor = gcd(width, height);
        Ok(Self { width: width / divisor, height: height / divisor })
    }

    pub fn label(&self) -> String {
        format!("{}:{}", self.width, self.height)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AspectDecision {
    pub ratio: AspectRatio,
    pub width: u32,
    pub height: u32,
    pub source: AspectSource,
    pub odd_clips: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionDefault {
    On,
    Off,
    Optional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pacing {
    Tight,
    Loose,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeDefaults {
    pub end_card: bool,
    pub captions: CaptionDefault,
    pub pacing: Pacing,
    pub optional_feed_aspect: bool,
    pub thumbnails: bool,
    pub chapters: bool,
    pub lower_third: bool,
    /// Nothing publishes, and nothing adds a 60-second cap on its own.
    pub length_cap: Option<MediaTick>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrollChoice {
    None,
    AgentPicks,
    Selected(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedVersion {
    pub difference: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefFlag {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefPlan {
    pub project_name: String,
    pub final_type: FinalType,
    pub target: LengthTarget,
    pub versions: Vec<PlannedVersion>,
    pub aspect: AspectDecision,
    pub defaults: TypeDefaults,
    pub broll: BrollChoice,
    pub raw_files: Vec<RawFile>,
    pub broll_uploads: Vec<RawFile>,
    pub flags: Vec<BriefFlag>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

pub fn type_defaults(final_type: FinalType) -> TypeDefaults {
    match final_type {
        FinalType::Reel => TypeDefaults {
            end_card: false,
            captions: CaptionDefault::On,
            pacing: Pacing::Tight,
            optional_feed_aspect: false,
            thumbnails: false,
            chapters: false,
            lower_third: false,
            length_cap: None,
        },
        FinalType::Ad => TypeDefaults {
            end_card: true,
            captions: CaptionDefault::On,
            pacing: Pacing::Tight,
            optional_feed_aspect: true,
            thumbnails: true,
            chapters: false,
            lower_third: false,
            length_cap: None,
        },
        FinalType::LongForm => TypeDefaults {
            end_card: false,
            captions: CaptionDefault::Optional,
            pacing: Pacing::Loose,
            optional_feed_aspect: false,
            thumbnails: false,
            chapters: true,
            lower_third: true,
            length_cap: None,
        },
    }
}

pub fn validate_brief(brief: &UploadBrief, known_broll: &[String]) -> Result<BriefPlan, Vec<FieldError>> {
    let mut errors = Vec::new();
    let project_name = match clean_name(&brief.project_name, "project name") {
        Ok(name) => name,
        Err(error) => {
            errors.push(field("project_name", error.message));
            String::new()
        }
    };
    if brief.version_count == 0 || brief.version_count > MAX_VERSIONS {
        errors.push(field("version_count", format!("version count must be from 1 to {MAX_VERSIONS}")));
    }
    let versions = match version_labels(brief) {
        Ok(versions) => versions,
        Err(error) => {
            errors.push(error);
            Vec::new()
        }
    };
    let target = brief.target.clone().unwrap_or(LengthTarget::Unspecified);
    if let Err(error) = check_target(&target) {
        errors.push(field("target", error.message));
    }
    if let Err(error) = check_broll(brief) {
        errors.push(error);
    }
    for id in &brief.broll_library_ids {
        if !known_broll.iter().any(|known| known == id) {
            errors.push(field("broll_library_ids", format!("unknown B-roll clip {id}")));
        }
    }
    if let Err(error) = check_files(&brief.raw_files, "raw_files") {
        errors.push(error);
    }
    if let Err(error) = check_files(&brief.broll_uploads, "broll_uploads") {
        errors.push(error);
    }
    let aspect = match decide_aspect(brief) {
        Ok(aspect) => Some(aspect),
        Err(error) => {
            errors.push(field("aspect", error.message));
            None
        }
    };
    if !errors.is_empty() {
        return Err(errors);
    }
    let aspect = aspect.ok_or_else(|| vec![field("aspect", "aspect was not decided")])?;
    let mut flags = Vec::new();
    if !aspect.odd_clips.is_empty() {
        flags.push(BriefFlag {
            code: "mixed_aspect".to_string(),
            message: format!("these clips do not match the output aspect: {}", aspect.odd_clips.join(", ")),
        });
    }
    if let Some(message) = raw_cannot_hit(&target, &brief.raw_files) {
        flags.push(BriefFlag { code: "raw_short".to_string(), message });
    }
    let broll = if !brief.use_broll {
        BrollChoice::None
    } else if brief.broll_library_ids.is_empty() && brief.broll_uploads.is_empty() {
        BrollChoice::AgentPicks
    } else {
        BrollChoice::Selected(brief.broll_library_ids.clone())
    };
    Ok(BriefPlan {
        project_name,
        final_type: brief.final_type,
        target,
        versions,
        aspect,
        defaults: type_defaults(brief.final_type),
        broll,
        raw_files: brief.raw_files.clone(),
        broll_uploads: brief.broll_uploads.clone(),
        flags,
    })
}

fn version_labels(brief: &UploadBrief) -> Result<Vec<PlannedVersion>, FieldError> {
    if brief.version_count == 0 || brief.version_count > MAX_VERSIONS {
        return Ok(Vec::new());
    }
    let mut labels = Vec::new();
    for index in 0..brief.version_count {
        let raw = brief.differences.get(index as usize).map(String::as_str).unwrap_or("").trim();
        let difference = if raw.is_empty() {
            if brief.version_count == 1 {
                primary_label(brief.final_type)
            } else {
                return Err(field("differences", "each version needs a difference label, not a letter"));
            }
        } else if is_letter_label(raw) {
            return Err(field("differences", "name the difference (hook, pacing, B-roll), not A, B or C"));
        } else {
            match clean_name(raw, "version label") {
                Ok(label) => label,
                Err(error) => return Err(field("differences", error.message)),
            }
        };
        if labels.iter().any(|existing: &PlannedVersion| existing.difference.eq_ignore_ascii_case(&difference)) {
            return Err(field("differences", "version labels have to be different from each other"));
        }
        labels.push(PlannedVersion { difference });
    }
    Ok(labels)
}

fn primary_label(final_type: FinalType) -> String {
    match final_type {
        FinalType::Reel => "Primary reel".to_string(),
        FinalType::Ad => "Primary ad".to_string(),
        FinalType::LongForm => "Primary cut".to_string(),
    }
}

fn is_letter_label(value: &str) -> bool {
    matches!(value.to_ascii_uppercase().as_str(), "A" | "B" | "C" | "A_FULL" | "B_TRIM" | "C_60")
}

fn check_target(target: &LengthTarget) -> Result<(), Error> {
    match target {
        LengthTarget::Unspecified => Ok(()),
        LengthTarget::Exact { ticks } => {
            if ticks.0 <= 0 {
                Err(Error::invalid("target length has to be greater than zero"))
            } else {
                Ok(())
            }
        }
        LengthTarget::Range { min, max } => {
            if min.0 <= 0 || max.0 <= 0 {
                Err(Error::invalid("target range has to be greater than zero"))
            } else if min.0 > max.0 {
                Err(Error::invalid("target range is backwards"))
            } else {
                Ok(())
            }
        }
    }
}

fn check_broll(brief: &UploadBrief) -> Result<(), FieldError> {
    if !brief.use_broll && (!brief.broll_library_ids.is_empty() || !brief.broll_uploads.is_empty()) {
        return Err(field("use_broll", "B-roll was included but Use B-roll is off"));
    }
    for upload in &brief.broll_uploads {
        if brief.raw_files.iter().any(|raw| raw.name.eq_ignore_ascii_case(&upload.name)) {
            return Err(field("broll_uploads", "B-roll is a separate upload and cannot use the same file as the raw footage"));
        }
    }
    Ok(())
}

fn check_files(files: &[RawFile], field_name: &str) -> Result<(), FieldError> {
    if field_name == "raw_files" && files.is_empty() {
        return Err(field(field_name, "add at least one raw file"));
    }
    if files.len() > 200 {
        return Err(field(field_name, "too many files in one upload"));
    }
    for file in files {
        if file.role == crate::model::MediaRole::Broll && field_name == "raw_files" {
            return Err(field(field_name, "a raw upload is A-roll; put B-roll in the B-roll upload"));
        }
        if file.role == crate::model::MediaRole::Aroll && field_name == "broll_uploads" {
            return Err(field(field_name, "the B-roll upload cannot contain A-roll"));
        }
        if clean_name(&file.name, "file name").is_err() || file.name.chars().count() > MAX_NAME {
            return Err(field(field_name, "a file name is missing or too long"));
        }
        if file.duration.0 <= 0 {
            return Err(field(field_name, format!("{} has no duration", file.name)));
        }
        if AspectRatio::reduced(file.width, file.height).is_err() {
            return Err(field(field_name, format!("{} has no usable picture size", file.name)));
        }
    }
    Ok(())
}

pub fn decide_aspect(brief: &UploadBrief) -> Result<AspectDecision, Error> {
    if brief.change_format {
        let width = brief.explicit_width.ok_or_else(|| Error::invalid("Change format needs a width and a height"))?;
        let height = brief.explicit_height.ok_or_else(|| Error::invalid("Change format needs a width and a height"))?;
        let ratio = AspectRatio::reduced(width, height)?;
        return Ok(AspectDecision { odd_clips: odd_clips(&brief.raw_files, &ratio), ratio, width, height, source: AspectSource::ChangeFormat });
    }
    if let (Some(width), Some(height)) = (brief.profile_width, brief.profile_height) {
        let ratio = AspectRatio::reduced(width, height)?;
        return Ok(AspectDecision { odd_clips: odd_clips(&brief.raw_files, &ratio), ratio, width, height, source: AspectSource::Profile });
    }
    let (width, height) = majority_raw(&brief.raw_files)?;
    let ratio = AspectRatio::reduced(width, height)?;
    Ok(AspectDecision { odd_clips: odd_clips(&brief.raw_files, &ratio), ratio, width, height, source: AspectSource::Raw })
}

fn majority_raw(files: &[RawFile]) -> Result<(u32, u32), Error> {
    let mut groups: Vec<(AspectRatio, i64, u32, u32)> = Vec::new();
    for file in files.iter().filter(|file| file.role == crate::model::MediaRole::Aroll) {
        let ratio = AspectRatio::reduced(file.width, file.height)?;
        if let Some(group) = groups.iter_mut().find(|group| group.0 == ratio) {
            group.1 = group.1.saturating_add(file.duration.0);
        } else {
            groups.push((ratio, file.duration.0, file.width, file.height));
        }
    }
    // A tie keeps the earlier group. A later group replaces it only when it is strictly longer.
    let mut best: Option<(i64, u32, u32)> = None;
    for group in &groups {
        let replace = match best {
            None => true,
            Some((duration, _, _)) => group.1 > duration,
        };
        if replace {
            best = Some((group.1, group.2, group.3));
        }
    }
    let (_, width, height) = best.ok_or_else(|| Error::invalid("add at least one raw file"))?;
    Ok((width, height))
}

fn odd_clips(files: &[RawFile], chosen: &AspectRatio) -> Vec<String> {
    files
        .iter()
        .filter(|file| file.role == crate::model::MediaRole::Aroll)
        .filter_map(|file| {
            let ratio = AspectRatio::reduced(file.width, file.height).ok()?;
            if ratio == *chosen { None } else { Some(file.name.clone()) }
        })
        .collect()
}

fn raw_cannot_hit(target: &LengthTarget, files: &[RawFile]) -> Option<String> {
    let total = files.iter().filter(|file| file.role == crate::model::MediaRole::Aroll).fold(0i64, |sum, file| sum.saturating_add(file.duration.0));
    let short = match target {
        LengthTarget::Unspecified => return None,
        LengthTarget::Exact { ticks } => total < ticks.0,
        LengthTarget::Range { min, .. } => total < min.0,
    };
    if short { Some("the raw footage is shorter than the target length".to_string()) } else { None }
}

fn field(name: &str, message: impl Into<String>) -> FieldError {
    FieldError { field: name.to_string(), message: message.into() }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let next = a % b;
        a = b;
        b = next;
    }
    a.max(1)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QcFlag {
    pub level: String,
    pub message: String,
}

pub fn qc_flags(plan: &BriefPlan, snapshot: &Snapshot) -> Vec<QcFlag> {
    let mut flags = Vec::new();
    let duration = snapshot.timeline.duration();
    match &plan.target {
        LengthTarget::Unspecified => {}
        LengthTarget::Exact { ticks } => {
            if duration.0 > ticks.0 {
                flags.push(QcFlag { level: "warn".to_string(), message: "the cut is longer than the target".to_string() });
            } else if duration.0 < ticks.0 {
                flags.push(QcFlag { level: "warn".to_string(), message: "the cut is shorter than the target".to_string() });
            }
        }
        LengthTarget::Range { min, max } => {
            if duration.0 < min.0 || duration.0 > max.0 {
                flags.push(QcFlag { level: "warn".to_string(), message: "the cut is outside the target length".to_string() });
            }
        }
    }
    let defaults = type_defaults(snapshot.final_type);
    let captions_on = snapshot.timeline.captions.iter().any(|caption| !caption.text.trim().is_empty());
    match defaults.captions {
        CaptionDefault::On if !captions_on => {
            flags.push(QcFlag { level: "warn".to_string(), message: "this format defaults to captions, and the timeline has none".to_string() });
        }
        CaptionDefault::Off if captions_on => {
            flags.push(QcFlag { level: "note".to_string(), message: "captions are on, and this format leaves them optional".to_string() });
        }
        _ => {}
    }
    let has_end = snapshot.timeline.clips.iter().any(|clip| clip.kind == crate::model::ClipKind::EndCard);
    if defaults.end_card && !has_end {
        flags.push(QcFlag { level: "note".to_string(), message: "an ad defaults to an end card and a CTA; none is on the timeline yet".to_string() });
    }
    if !defaults.end_card && has_end {
        flags.push(QcFlag { level: "note".to_string(), message: "an end card is on the timeline; this format leaves it off unless you add one".to_string() });
    }
    let output = AspectRatio::reduced(snapshot.timeline.width, snapshot.timeline.height).ok();
    if let Some(output) = output {
        for clip in &snapshot.timeline.clips {
            if !clip.kind.is_picture() || clip.source_width == 0 || clip.source_height == 0 {
                continue;
            }
            if let Ok(ratio) = AspectRatio::reduced(clip.source_width, clip.source_height)
                && ratio != output
            {
                flags.push(QcFlag { level: "warn".to_string(), message: format!("{} is {} and the sequence is {}", clip.name, ratio.label(), output.label()) });
            }
        }
    }
    flags
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackFile {
    pub schema: String,
    pub client: String,
    pub project: String,
    pub final_type: FinalType,
    pub versions: Vec<VersionFeedback>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionFeedback {
    /// The shape `feedback.py --marks` already expects, one object per version.
    pub approval: Option<ApprovalJson>,
    pub difference: String,
    pub comments: Vec<CommentExport>,
    pub client_edits: Vec<EditExport>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalJson {
    pub version: String,
    pub mark: String,
    pub note: String,
    pub by: String,
    pub at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommentExport {
    pub body: String,
    pub start_ticks: String,
    pub end_ticks: String,
    pub region: Option<crate::model::Region>,
    pub resolved: bool,
    pub fixed_in: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditExport {
    pub action: String,
    pub targets: Vec<String>,
    pub summary: String,
    pub at: String,
    /// Clip or caption lengths before the edit, as frame counts. These are the style signal.
    pub before_frames: Vec<String>,
    pub after_frames: Vec<String>,
}

pub fn approval_json(difference: &str, snapshot: &Snapshot) -> Option<ApprovalJson> {
    let approval = snapshot.approval.as_ref()?;
    Some(ApprovalJson {
        version: difference.to_string(),
        mark: approval.mark.as_str().to_string(),
        note: approval.note.clone(),
        by: approval.by.clone(),
        at: approval.at.clone(),
    })
}

pub fn mark_name(mark: Mark) -> &'static str {
    mark.as_str()
}
