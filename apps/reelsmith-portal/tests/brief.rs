//! Upload-form rules: versions, B-roll separation, aspect, and the lack of a 60-second cap.

use reelsmith_portal::brief::{CaptionDefault, LengthTarget, UploadBrief, type_defaults, validate_brief};
use reelsmith_portal::model::{AspectSource, FinalType, MediaRole, MediaTick};
use reelsmith_time::TICKS_PER_SECOND;

fn ticks(seconds: i64) -> MediaTick {
    MediaTick(seconds * TICKS_PER_SECOND)
}

fn file(name: &str, width: u32, height: u32, seconds: i64, role: MediaRole) -> reelsmith_portal::brief::RawFile {
    reelsmith_portal::brief::RawFile { name: name.to_string(), width, height, duration: ticks(seconds), role }
}

fn brief() -> UploadBrief {
    UploadBrief {
        project_name: "Spring workshop".to_string(),
        final_type: FinalType::Reel,
        target: None,
        end_card: None,
        version_count: 1,
        differences: vec![],
        use_broll: false,
        broll_library_ids: vec![],
        broll_uploads: vec![],
        raw_files: vec![file("Hook", 1080, 1920, 8, MediaRole::Aroll), file("Interview", 1080, 1920, 20, MediaRole::Aroll)],
        change_format: false,
        explicit_width: None,
        explicit_height: None,
        profile_width: None,
        profile_height: None,
    }
}

fn messages(errors: &[reelsmith_portal::brief::FieldError]) -> String {
    errors.iter().map(|error| error.message.as_str()).collect::<Vec<_>>().join(" | ")
}

#[test]
fn a_single_version_gets_a_primary_label_and_the_raw_aspect() {
    let plan = validate_brief(&brief(), &[]).expect("valid");
    assert_eq!(plan.versions.len(), 1);
    assert_eq!(plan.versions[0].difference, "Primary reel");
    assert_eq!(plan.aspect.source, AspectSource::Raw);
    assert_eq!((plan.aspect.width, plan.aspect.height), (1080, 1920));
    assert_eq!(plan.aspect.ratio.label(), "9:16");
    assert!(plan.flags.is_empty());
    assert!(!plan.defaults.end_card);
    assert_eq!(plan.defaults.captions, CaptionDefault::On);
    assert!(plan.defaults.length_cap.is_none());
}

#[test]
fn versions_need_difference_labels_and_reject_letters() {
    let mut form = brief();
    form.version_count = 0;
    assert!(messages(&validate_brief(&form, &[]).expect_err("zero")).contains("1 to 12"));
    form.version_count = 2;
    form.differences = vec!["A".to_string(), "B".to_string()];
    assert!(messages(&validate_brief(&form, &[]).expect_err("letters")).contains("not A, B or C"));
    form.differences = vec!["tighter hook".to_string(), "tighter hook".to_string()];
    assert!(messages(&validate_brief(&form, &[]).expect_err("dup")).contains("different"));
    form.differences = vec!["tighter hook".to_string(), "more B-roll".to_string()];
    let plan = validate_brief(&form, &[]).expect("named");
    assert_eq!(plan.versions[1].difference, "more B-roll");
}

#[test]
fn broll_stays_separate_from_raw() {
    let mut form = brief();
    form.broll_uploads = vec![file("cutaway", 1920, 1080, 3, MediaRole::Broll)];
    assert!(messages(&validate_brief(&form, &[]).expect_err("off")).contains("Use B-roll is off"));
    form.use_broll = true;
    form.broll_uploads = vec![file("Hook", 1920, 1080, 3, MediaRole::Broll)];
    assert!(messages(&validate_brief(&form, &[]).expect_err("mixed")).contains("same file"));
    form.broll_uploads = vec![file("cutaway", 1920, 1080, 3, MediaRole::Broll)];
    form.broll_library_ids = vec!["missing".to_string()];
    assert!(messages(&validate_brief(&form, &["known".to_string()]).expect_err("id")).contains("unknown"));
    form.broll_library_ids.clear();
    let plan = validate_brief(&form, &[]).expect("broll");
    assert!(matches!(plan.broll, reelsmith_portal::brief::BrollChoice::Selected(_)));
    form.broll_uploads.clear();
    let plan = validate_brief(&form, &[]).expect("agent picks");
    assert!(matches!(plan.broll, reelsmith_portal::brief::BrollChoice::AgentPicks));
}

#[test]
fn aspect_follows_raw_unless_profile_or_change_format() {
    let mut form = brief();
    form.raw_files.push(file("wide", 1920, 1080, 4, MediaRole::Aroll));
    let plan = validate_brief(&form, &[]).expect("majority");
    assert_eq!(plan.aspect.ratio.label(), "9:16");
    assert_eq!(plan.aspect.odd_clips, vec!["wide".to_string()]);
    assert!(plan.flags.iter().any(|flag| flag.code == "mixed_aspect"));

    form.profile_width = Some(1080);
    form.profile_height = Some(1350);
    let plan = validate_brief(&form, &[]).expect("profile");
    assert_eq!(plan.aspect.source, AspectSource::Profile);
    assert_eq!(plan.aspect.ratio.label(), "4:5");

    form.change_format = true;
    form.explicit_width = Some(1920);
    form.explicit_height = Some(1080);
    let plan = validate_brief(&form, &[]).expect("format");
    assert_eq!(plan.aspect.source, AspectSource::ChangeFormat);
    assert_eq!((plan.aspect.width, plan.aspect.height), (1920, 1080));

    form.explicit_width = None;
    assert!(validate_brief(&form, &[]).is_err());
}

#[test]
fn equal_raw_durations_keep_the_earlier_aspect() {
    let mut form = brief();
    form.raw_files = vec![file("first", 1920, 1080, 10, MediaRole::Aroll), file("second", 1080, 1920, 10, MediaRole::Aroll)];
    let plan = validate_brief(&form, &[]).expect("tie");
    assert_eq!(plan.aspect.ratio.label(), "16:9");
    assert_eq!(plan.aspect.odd_clips, vec!["second".to_string()]);
}

#[test]
fn there_is_no_under_60_rule_unless_the_target_asks() {
    let mut form = brief();
    form.raw_files = vec![file("short", 1080, 1920, 12, MediaRole::Aroll)];
    let plan = validate_brief(&form, &[]).expect("short raw is fine");
    assert!(plan.flags.iter().all(|flag| flag.code != "raw_short"));
    form.target = Some(LengthTarget::Exact { ticks: ticks(60) });
    let plan = validate_brief(&form, &[]).expect("flagged");
    assert!(plan.flags.iter().any(|flag| flag.code == "raw_short"));
    form.target = Some(LengthTarget::Range { min: ticks(30), max: ticks(10) });
    assert!(messages(&validate_brief(&form, &[]).expect_err("range")).contains("backwards"));
}

#[test]
fn final_type_defaults_do_not_invent_a_length_cap() {
    let reel = type_defaults(FinalType::Reel);
    assert!(!reel.end_card && reel.captions == CaptionDefault::On && reel.length_cap.is_none());
    let ad = type_defaults(FinalType::Ad);
    assert!(ad.end_card && ad.optional_feed_aspect && ad.thumbnails && ad.length_cap.is_none());
    let long = type_defaults(FinalType::LongForm);
    assert!(!long.end_card && long.captions == CaptionDefault::Optional && long.chapters && long.length_cap.is_none());
}

#[test]
fn same_pixel_shape_is_one_aspect() {
    let mut form = brief();
    form.raw_files = vec![file("hd", 1920, 1080, 5, MediaRole::Aroll), file("small", 1280, 720, 5, MediaRole::Aroll)];
    let plan = validate_brief(&form, &[]).expect("same shape");
    assert!(plan.aspect.odd_clips.is_empty());
    assert_eq!(plan.aspect.ratio.label(), "16:9");
}
