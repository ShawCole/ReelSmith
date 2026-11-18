//! First cut after an upload: A-roll laid end to end, optional captions, B-roll listed but not cut in.

use crate::brief::{BriefPlan, BrollChoice, CaptionDefault};
use crate::error::Error;
use crate::model::{AspectSource, Caption, Clip, ClipKind, Gain, LibraryItem, MediaRole, MediaTick, Norm, Snapshot, Timeline};
use crate::persist::BrollRecord;

pub fn initial_snapshot(plan: &BriefPlan, library: &[BrollRecord]) -> Result<Snapshot, Error> {
    let mut timeline = Timeline::blank(plan.aspect.width, plan.aspect.height, plan.aspect.source)?;
    timeline.snap_words = true;
    let captions_on = matches!(plan.defaults.captions, CaptionDefault::On);
    let mut cursor = MediaTick::ZERO;
    for file in plan.raw_files.iter().filter(|file| file.role == MediaRole::Aroll) {
        let frames = timeline.tick_to_frame(file.duration)?.max(1);
        let duration = timeline.frame_to_tick(frames)?;
        // Five seconds of unused source on the tail, so a later trim can lengthen the shot.
        let source_duration = duration.checked_add(timeline.frame_to_tick(150)?)?;
        let video_id = timeline.alloc_id("v");
        let audio_id = timeline.alloc_id("a");
        timeline.clips.push(Clip {
            id: video_id.clone(),
            track: "V1".to_string(),
            name: file.name.clone(),
            kind: ClipKind::Video,
            start: cursor,
            duration,
            source_in: MediaTick::ZERO,
            source_duration,
            gain_permille: Gain::UNITY,
            muted: false,
            linked: Some(audio_id.clone()),
            text: String::new(),
            source_width: file.width,
            source_height: file.height,
        });
        timeline.clips.push(Clip {
            id: audio_id,
            track: "A1".to_string(),
            name: format!("{} dialogue", file.name),
            kind: ClipKind::Audio,
            start: cursor,
            duration,
            source_in: MediaTick::ZERO,
            source_duration,
            gain_permille: Gain::UNITY,
            muted: false,
            linked: Some(video_id.clone()),
            text: String::new(),
            source_width: 0,
            source_height: 0,
        });
        if captions_on {
            let caption_id = timeline.alloc_id("cap");
            timeline.captions.push(Caption { id: caption_id, start: cursor, duration, text: file.name.clone(), anchor_y: Norm::new(820)? });
        }
        cursor = cursor.checked_add(duration)?;
    }
    if cursor.0 > 0 {
        let music_id = timeline.alloc_id("m");
        timeline.clips.push(Clip {
            id: music_id,
            track: "A2".to_string(),
            name: "Music bed".to_string(),
            kind: ClipKind::Music,
            start: MediaTick::ZERO,
            duration: cursor,
            source_in: MediaTick::ZERO,
            source_duration: cursor,
            gain_permille: Gain::new(250)?,
            muted: false,
            linked: None,
            text: String::new(),
            source_width: 0,
            source_height: 0,
        });
    }
    let selected: Vec<&str> = match &plan.broll {
        BrollChoice::Selected(ids) => ids.iter().map(String::as_str).collect(),
        _ => Vec::new(),
    };
    for item in library {
        timeline.library.push(LibraryItem {
            id: item.id.clone(),
            name: item.name.clone(),
            role: MediaRole::Broll,
            width: item.width,
            height: item.height,
            duration: item.duration,
            selected: selected.iter().any(|id| *id == item.id),
        });
    }
    if timeline.aspect_source == AspectSource::ChangeFormat {
        timeline.reframed = true;
    }
    Ok(Snapshot::new(timeline, plan.final_type))
}
