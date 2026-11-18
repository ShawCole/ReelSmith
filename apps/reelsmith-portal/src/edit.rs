//! Timeline edits on a [`Snapshot`]. Each function mutates one snapshot; the history log stores
//! the before and after. Linked picture and audio clips move together.

use crate::error::Error;
use crate::model::{
    Actor, Approval, AspectSource, Caption, Clip, ClipKind, Comment, Edge, FinalType, Gain, MAX_CAPTIONS, MAX_CLIPS, MAX_COMMENTS, MAX_TEXT, Mark, MediaTick,
    Norm, Region, Reply, Snapshot, Status, Timeline, Word, clean_text,
};

const TRACKS_PICTURE: &[&str] = &["V1", "V2"];
const TRACKS_SOUND: &[&str] = &["A1", "A2", "A3"];

pub fn trim(snapshot: &mut Snapshot, clip_id: &str, edge: Edge, delta_frames: i64) -> Result<Vec<String>, Error> {
    if delta_frames == 0 {
        return Err(Error::invalid("trim delta is zero"));
    }
    let delta = frames_to_delta(&snapshot.timeline, delta_frames)?;
    let primary = snapshot.timeline.clip(clip_id)?.clone();
    let snapped = snap_edge(&snapshot.timeline, &primary, edge, delta)?;
    apply_delta(snapshot, &linked_ids(&snapshot.timeline, clip_id)?, edge, snapped)
}

pub fn set_length(snapshot: &mut Snapshot, clip_id: &str, frames: i64) -> Result<Vec<String>, Error> {
    if frames <= 0 {
        return Err(Error::invalid("a clip needs at least one frame"));
    }
    let frame = snapshot.timeline.frame_ticks()?;
    let clip = snapshot.timeline.clip(clip_id)?;
    let current = clip.duration.0 / frame;
    let delta = frames.saturating_sub(current);
    if delta == 0 {
        return Ok(vec![clip_id.to_string()]);
    }
    trim(snapshot, clip_id, Edge::Out, delta)
}

pub fn razor(snapshot: &mut Snapshot, clip_id: &str, at_frame: i64) -> Result<Vec<String>, Error> {
    let at = snapshot.timeline.frame_to_tick(at_frame)?;
    let ids = linked_ids(&snapshot.timeline, clip_id)?;
    let mut created = Vec::new();
    for id in ids {
        let clip = snapshot.timeline.clip(&id)?.clone();
        if at <= clip.start || at >= clip.end() {
            return Err(Error::invalid("the razor has to land inside the clip"));
        }
        let left = at.checked_sub(clip.start)?;
        let right_duration = clip.duration.checked_sub(left)?;
        let right_in = clip.source_in.checked_add(left)?;
        if left.0 <= 0 || right_duration.0 <= 0 {
            return Err(Error::invalid("the razor has to leave frames on both sides"));
        }
        let new_id = snapshot.timeline.alloc_id("c");
        snapshot.timeline.clip_mut(&id)?.duration = left;
        let right = Clip { id: new_id.clone(), start: at, duration: right_duration, source_in: right_in, linked: None, ..clip };
        push_clip(&mut snapshot.timeline, right)?;
        created.push(id);
        created.push(new_id);
    }
    relink_splits(&mut snapshot.timeline, &created)?;
    Ok(created)
}

pub fn move_clip(snapshot: &mut Snapshot, clip_id: &str, frame: i64, track: Option<&str>) -> Result<Vec<String>, Error> {
    if frame < 0 {
        return Err(Error::invalid("a clip cannot start before the first frame"));
    }
    let new_start = snapshot.timeline.frame_to_tick(frame)?;
    let clip = snapshot.timeline.clip(clip_id)?.clone();
    let delta = new_start.checked_sub(clip.start)?;
    let ids = linked_ids(&snapshot.timeline, clip_id)?;
    if let Some(track) = track {
        check_track(clip.kind, track)?;
        snapshot.timeline.clip_mut(clip_id)?.track = track.to_string();
    }
    for id in &ids {
        let item = snapshot.timeline.clip_mut(id)?;
        item.start = item.start.checked_add(delta)?;
    }
    Ok(ids)
}

pub fn lift(snapshot: &mut Snapshot, clip_id: &str) -> Result<Vec<String>, Error> {
    let ids = linked_ids(&snapshot.timeline, clip_id)?;
    snapshot.timeline.clips.retain(|clip| !ids.iter().any(|id| id == &clip.id));
    Ok(ids)
}

pub fn gain(snapshot: &mut Snapshot, clip_id: &str, permille: u16, muted: Option<bool>) -> Result<Vec<String>, Error> {
    let gain = Gain::new(permille)?;
    let ids = linked_ids(&snapshot.timeline, clip_id)?;
    for id in &ids {
        let clip = snapshot.timeline.clip_mut(id)?;
        if clip.kind.is_sound() || clip.id == clip_id {
            clip.gain_permille = gain;
            if let Some(muted) = muted {
                clip.muted = muted;
            }
        }
    }
    Ok(vec![clip_id.to_string()])
}

pub fn caption_text(snapshot: &mut Snapshot, caption_id: &str, text: &str) -> Result<Vec<String>, Error> {
    let text = clean_text(text, 500, "caption")?;
    let caption = caption_mut(snapshot, caption_id)?;
    caption.text = text;
    Ok(vec![caption_id.to_string()])
}

pub fn caption_move(snapshot: &mut Snapshot, caption_id: &str, frame: i64, frames: Option<i64>) -> Result<Vec<String>, Error> {
    if frame < 0 {
        return Err(Error::invalid("a caption cannot start before the first frame"));
    }
    let start = snapshot.timeline.frame_to_tick(frame)?;
    let duration = match frames {
        Some(frames) if frames <= 0 => return Err(Error::invalid("a caption needs at least one frame")),
        Some(frames) => Some(snapshot.timeline.frame_to_tick(frames)?),
        None => None,
    };
    let caption = caption_mut(snapshot, caption_id)?;
    caption.start = start;
    if let Some(duration) = duration {
        caption.duration = duration;
    }
    Ok(vec![caption_id.to_string()])
}

pub fn set_snap(snapshot: &mut Snapshot, enabled: bool) -> Result<Vec<String>, Error> {
    snapshot.timeline.snap_words = enabled;
    Ok(vec!["sequence".to_string()])
}

/// Change format is a recorded step: the picture size changes and caption anchors move when the
/// orientation changes.
pub fn change_format(snapshot: &mut Snapshot, width: u32, height: u32) -> Result<Vec<String>, Error> {
    if width == 0 || height == 0 || width > 7680 || height > 7680 {
        return Err(Error::invalid("picture size must be between 1 and 7680 on each side"));
    }
    let old_w = snapshot.timeline.width;
    let old_h = snapshot.timeline.height;
    snapshot.timeline.width = width;
    snapshot.timeline.height = height;
    snapshot.timeline.aspect_source = AspectSource::ChangeFormat;
    snapshot.timeline.reframed = true;
    let mut targets = vec!["sequence".to_string()];
    for caption in &mut snapshot.timeline.captions {
        let next = reposition(caption.anchor_y, old_w, old_h, width, height)?;
        if next != caption.anchor_y {
            caption.anchor_y = next;
            targets.push(caption.id.clone());
        }
    }
    Ok(targets)
}

pub fn reposition(anchor: Norm, old_w: u32, old_h: u32, new_w: u32, new_h: u32) -> Result<Norm, Error> {
    let old = orientation(old_w, old_h);
    let new = orientation(new_w, new_h);
    if old == new {
        return Ok(anchor);
    }
    // Portrait captions sit a little higher than landscape captions so they clear a lower third.
    let y = if new_h > new_w { 780 } else { 860 };
    Norm::new(y)
}

pub fn add_comment(snapshot: &mut Snapshot, author: &str, start_frame: i64, end_frame: i64, region: Option<Region>, body: &str) -> Result<Vec<String>, Error> {
    if snapshot.comments.len() >= MAX_COMMENTS {
        return Err(Error::invalid("the comment list is full"));
    }
    if end_frame < start_frame || start_frame < 0 {
        return Err(Error::invalid("a comment's time range is backwards"));
    }
    let body = clean_text(body, MAX_TEXT, "comment")?;
    let id = snapshot.timeline.alloc_id("n");
    snapshot.comments.push(Comment {
        id: id.clone(),
        author: author.to_string(),
        start: snapshot.timeline.frame_to_tick(start_frame)?,
        end: snapshot.timeline.frame_to_tick(end_frame)?,
        region,
        body,
        resolved: false,
        fixed_in: None,
        replies: Vec::new(),
    });
    Ok(vec![id])
}

pub fn resolve_comment(snapshot: &mut Snapshot, comment_id: &str, resolved: bool, fixed_in: Option<String>) -> Result<Vec<String>, Error> {
    let comment = snapshot.comments.iter_mut().find(|comment| comment.id == comment_id).ok_or_else(|| Error::not_found("no comment"))?;
    comment.resolved = resolved;
    comment.fixed_in = if resolved { fixed_in.filter(|value| !value.trim().is_empty()) } else { None };
    Ok(vec![comment_id.to_string()])
}

pub fn reply(snapshot: &mut Snapshot, comment_id: &str, author: &str, at: &str, body: &str) -> Result<Vec<String>, Error> {
    let body = clean_text(body, MAX_TEXT, "reply")?;
    let comment = snapshot.comments.iter_mut().find(|comment| comment.id == comment_id).ok_or_else(|| Error::not_found("no comment"))?;
    if comment.replies.len() >= 100 {
        return Err(Error::invalid("this thread is full"));
    }
    comment.replies.push(Reply { author: author.to_string(), at: at.to_string(), body });
    Ok(vec![comment_id.to_string()])
}

pub fn set_status(snapshot: &mut Snapshot, status: Status) -> Result<Vec<String>, Error> {
    snapshot.status = status;
    Ok(vec!["status".to_string()])
}

pub fn approve(snapshot: &mut Snapshot, by: &str, at: &str, note: &str) -> Result<Vec<String>, Error> {
    let note = if note.trim().is_empty() { "Approved".to_string() } else { clean_text(note, MAX_TEXT, "note")? };
    snapshot.approval = Some(Approval { mark: Mark::Good, note, by: by.to_string(), at: at.to_string() });
    snapshot.status = Status::Approved;
    Ok(vec!["approval".to_string()])
}

pub fn request_changes(snapshot: &mut Snapshot, by: &str, at: &str, note: &str) -> Result<Vec<String>, Error> {
    let note = if note.trim().is_empty() { "Changes requested".to_string() } else { clean_text(note, MAX_TEXT, "note")? };
    snapshot.approval = Some(Approval { mark: Mark::NotGood, note, by: by.to_string(), at: at.to_string() });
    snapshot.status = Status::ChangesRequested;
    Ok(vec!["approval".to_string()])
}

pub fn set_decisions(snapshot: &mut Snapshot, text: &str) -> Result<Vec<String>, Error> {
    if text.chars().count() > 20_000 {
        return Err(Error::invalid("decisions notes are longer than 20000 characters"));
    }
    if text.chars().any(|ch| ch == '\0') {
        return Err(Error::invalid("decisions notes contain a control character"));
    }
    snapshot.decisions = text.trim().to_string();
    Ok(vec!["decisions".to_string()])
}

pub fn set_final_type(snapshot: &mut Snapshot, final_type: FinalType) -> Result<Vec<String>, Error> {
    snapshot.final_type = final_type;
    Ok(vec!["final_type".to_string()])
}

pub fn place_broll(snapshot: &mut Snapshot, item_id: &str) -> Result<Vec<String>, Error> {
    let item =
        snapshot.timeline.library.iter().find(|item| item.id == item_id).cloned().ok_or_else(|| Error::not_found("that B-roll clip is not in the library"))?;
    if item.role != crate::model::MediaRole::Broll {
        return Err(Error::invalid("only B-roll from the library can be placed on V2"));
    }
    let start = snapshot.timeline.duration();
    let id = snapshot.timeline.alloc_id("b");
    push_clip(
        &mut snapshot.timeline,
        Clip {
            id: id.clone(),
            track: "V2".to_string(),
            name: item.name,
            kind: ClipKind::Broll,
            start,
            duration: item.duration,
            source_in: MediaTick::ZERO,
            source_duration: item.duration,
            gain_permille: Gain::UNITY,
            muted: true,
            linked: None,
            text: String::new(),
            source_width: item.width,
            source_height: item.height,
        },
    )?;
    Ok(vec![id])
}

fn apply_delta(snapshot: &mut Snapshot, ids: &[String], edge: Edge, delta: MediaTick) -> Result<Vec<String>, Error> {
    for id in ids {
        let clip = snapshot.timeline.clip(id)?.clone();
        let updated = plan_trim(&clip, edge, delta)?;
        let slot = snapshot.timeline.clip_mut(id)?;
        slot.start = updated.start;
        slot.duration = updated.duration;
        slot.source_in = updated.source_in;
    }
    Ok(ids.to_vec())
}

struct Planned {
    start: MediaTick,
    duration: MediaTick,
    source_in: MediaTick,
}

fn plan_trim(clip: &Clip, edge: Edge, delta: MediaTick) -> Result<Planned, Error> {
    let (start, duration, source_in) = match edge {
        Edge::Out => (clip.start, clip.duration.checked_add(delta)?, clip.source_in),
        Edge::In => {
            let start = clip.start.checked_add(delta)?;
            let source_in = clip.source_in.checked_add(delta)?;
            let duration = clip.duration.checked_sub(delta)?;
            if start.0 < 0 || source_in.0 < 0 {
                return Err(Error::invalid("the trim runs past the start of the clip"));
            }
            (start, duration, source_in)
        }
    };
    if duration.0 <= 0 {
        return Err(Error::invalid("the trim would remove the clip"));
    }
    let source_end = source_in.checked_add(duration)?;
    if source_end > clip.source_duration {
        return Err(Error::invalid("the trim runs past the end of the source"));
    }
    Ok(Planned { start, duration, source_in })
}

fn snap_edge(timeline: &Timeline, clip: &Clip, edge: Edge, delta: MediaTick) -> Result<MediaTick, Error> {
    let unsnapped = plan_trim(clip, edge, delta)?;
    if !timeline.snap_words || timeline.words.is_empty() {
        return Ok(delta);
    }
    let edge_tick = match edge {
        Edge::Out => unsnapped.start.saturating_add(unsnapped.duration),
        Edge::In => unsnapped.start,
    };
    let frame = MediaTick(timeline.frame_ticks()?);
    let threshold = MediaTick(frame.0.saturating_mul(2));
    let mut best: Option<(i64, MediaTick)> = None;
    for word in &timeline.words {
        for boundary in [word.start, word.end] {
            let distance = (boundary.0 - edge_tick.0).saturating_abs();
            if distance > threshold.0 || distance == 0 {
                continue;
            }
            let candidate_delta = match edge {
                Edge::Out => delta.checked_add(MediaTick(boundary.0 - edge_tick.0))?,
                Edge::In => delta.checked_add(MediaTick(boundary.0 - edge_tick.0))?,
            };
            if plan_trim(clip, edge, candidate_delta).is_err() {
                continue;
            }
            let replace = match best {
                None => true,
                Some((best_distance, _)) => distance < best_distance,
            };
            if replace {
                best = Some((distance, candidate_delta));
            }
        }
    }
    Ok(best.map(|(_, delta)| delta).unwrap_or(delta))
}

fn frames_to_delta(timeline: &Timeline, frames: i64) -> Result<MediaTick, Error> {
    timeline.frame_to_tick(frames)
}

fn linked_ids(timeline: &Timeline, clip_id: &str) -> Result<Vec<String>, Error> {
    let clip = timeline.clip(clip_id)?;
    let mut ids = vec![clip.id.clone()];
    if let Some(partner) = &clip.linked
        && timeline.clip(partner).is_ok()
        && !ids.iter().any(|id| id == partner)
    {
        ids.push(partner.clone());
    }
    for other in &timeline.clips {
        if other.linked.as_deref() == Some(clip_id) && !ids.iter().any(|id| id == &other.id) {
            ids.push(other.id.clone());
        }
    }
    Ok(ids)
}

fn relink_splits(timeline: &mut Timeline, created: &[String]) -> Result<(), Error> {
    // `created` is [left, right, left, right, ...] in linked order. Pair rights together
    // and keep the lefts linked as they already were.
    if created.len() < 4 {
        return Ok(());
    }
    let right_a = created.get(1).cloned().ok_or_else(|| Error::invalid("split did not create a clip"))?;
    let right_b = created.get(3).cloned().ok_or_else(|| Error::invalid("split did not create a partner"))?;
    if let Ok(clip) = timeline.clip_mut(&right_a) {
        clip.linked = Some(right_b.clone());
    }
    if let Ok(clip) = timeline.clip_mut(&right_b) {
        clip.linked = Some(right_a);
    }
    Ok(())
}

fn push_clip(timeline: &mut Timeline, clip: Clip) -> Result<(), Error> {
    if timeline.clips.len() >= MAX_CLIPS {
        return Err(Error::invalid("the timeline is full"));
    }
    timeline.clips.push(clip);
    Ok(())
}

fn caption_mut<'a>(snapshot: &'a mut Snapshot, id: &str) -> Result<&'a mut Caption, Error> {
    snapshot.timeline.captions.iter_mut().find(|caption| caption.id == id).ok_or_else(|| Error::not_found("no caption"))
}

fn check_track(kind: ClipKind, track: &str) -> Result<(), Error> {
    let allowed = if kind.is_picture() { TRACKS_PICTURE } else { TRACKS_SOUND };
    if allowed.contains(&track) { Ok(()) } else { Err(Error::invalid(format!("{track} is not a track for this clip"))) }
}

fn orientation(width: u32, height: u32) -> i8 {
    match width.cmp(&height) {
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
    }
}

pub fn push_caption(timeline: &mut Timeline, caption: Caption) -> Result<(), Error> {
    if timeline.captions.len() >= MAX_CAPTIONS {
        return Err(Error::invalid("the caption list is full"));
    }
    timeline.captions.push(caption);
    Ok(())
}

/// Used by the agent-reason tests to build a client-looking actor without the HTTP layer.
pub fn client_actor(name: &str) -> Actor {
    Actor::Client { client_id: "client".to_string(), name: name.to_string() }
}

pub fn word(timeline: &Timeline, start_frame: i64, end_frame: i64, text: &str) -> Result<Word, Error> {
    Ok(Word { start: timeline.frame_to_tick(start_frame)?, end: timeline.frame_to_tick(end_frame)?, text: text.to_string() })
}
