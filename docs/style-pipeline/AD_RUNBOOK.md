# ReelSmith ad-video runbook (any client)

How to edit a client's video ads in ReelSmith, start to finish: audio, transcript and alignment, cut planning, live build, captions, end card, 4:5, review page, delivery, thumbnails and Meta handoff.

This was generalised from a real client project. Anything not confirmed in that project is marked **unverified**. This file contains no secret values. Secrets are named only by where they live.

---

## 0. Project variables

Fill these in at the start of each project. They are used everywhere below.

| Placeholder | Meaning | Example |
|---|---|---|
| `<Client>` | Short client name with no spaces, used in folder names | `AcmeCo` |
| `<Speaker>` | The on-camera person's full name, spelled the way the client wants it | `Jane Doe` |
| `<SpeakerSurname>` | Speaker's last name (used by the intro rule) | `Doe` |
| `<Project>` | The campaign or event the ads promote | `Spring Workshop` |
| `<EventDate>` | Event date as it appears on screen and in speech | `March 3` |
| `<EventPlace>` | Event location as it appears on screen and in speech | `Austin, TX` |
| `<ClientRoot>` | Project root | `~/StylePipe/<Client>_Ads/` |
| `<SourceFolders>` | Read-only folders holding the raw takes (originals plus any remuxed take folders) | `Original-Videos/`, `<Client>-Ads_*/` |
| `<SyncRoot>` | Optional sync folder (iCloud, Drive, or another copy) | `{client_root}/../_sync/<Client>_Ads_Finals/` |
| `<ReviewRoot>` | Working folder for review pages, captions and notes | `~/StylePipe/review/<client>/` |
| `<Take>` | One recorded ad take | `Ad1-Long`, `Ad2-Short-A` |
| `<RefTake>` | The approved take used as the reference for loudness, B-roll picks, authority section and CTA | `Ad1-Long` |
| `<ReviewSite>` | The client's review page URL (Netlify) | `https://<client>-ads-in-review.netlify.app` |
| `<ApprovalForm>` | Netlify form name for approvals | `<client>-approval` |
| `<MetaNotes>` | Where the client's Meta account, page and ad IDs are written down | `<ReviewRoot>/<Client>_Meta_notes.md` |
| `<MetaCaptions>` | Live file of Meta post captions | `<ReviewRoot>/<Client>_Meta_Captions_v1.md` |
| `<StyleSheet>` | The per-client style sheet (section 15) | `<ClientRoot>/pipeline/style.md` |

**Versions per take** (general convention, keep unless the client wants otherwise): A = full, B = trim, C = under 60 s.

Keep a list of take names and whether each take is vertical (9:16) or landscape-only in `<ClientRoot>/pipeline/takes.md`.

---

## 1. Setup and access

1. ReelSmith runs on the edit machine. When someone is watching, build in the open app at a watchable pace (`build_take.py --pace 0.25`), not in the background.
2. Take the shell, ffmpeg location and other tools from `machine.yaml`. Always run ffmpeg with `-nostdin`.
3. Live control: start ReelSmith with `reelsmith --control <port>`. It needs a 64-hex control token in `REELSMITH_CONTROL_TOKEN`. Scripts read it from the token file named in the machine profile (for example `~/.config/reelsmith/control_token`). Never print, copy or move that token. The usual local bridge is `127.0.0.1:9876`. (How the app was first launched with `--control 9876` is **unverified**; the scripts assume it is already running.)
4. Scripts talk to the app through `pipeline/rsmcp.py`, a small MCP client that runs `reelsmith-cli mcp --bridge 127.0.0.1:9876`. In code: `ReelSmith(bridge='127.0.0.1:9876')`, then `fc.tool('project_inspect')` and `fc.run('<command id>', ...)`.
5. Headless alternative (no live window): `reelsmith-cli --project <copy.fcproj> ...`. Useful subcommands: `inspect`, `exec <id> key=value`, `run <script.jsonl>`, `export <out> --preset ...`, `probe`, `render --seconds S --out f.png`, `commands`, `describe`. `--save` / `--save-as` write the project back. Use headless when nobody needs to watch.
6. Audio work, transcripts, review pages and the publishing handoff can run on another machine. The edit happens where ReelSmith is running.
7. Before any live edit, check `project_inspect`: the open path is the one you expect and it is not `dirty`. If there are unsaved live changes, save them as they are and back them up first (`_useredits`). Caption scripts act on whichever project is open, so open the target first.
8. Check free space against `min_free_gb` before renders and copies, and don't leave extra copies.

## 2. Source and safety rules

1. Never modify source takes, anything in `<SourceFolders>`, or the isolated audio folder (`even_better_audio/`). Never touch another version's folder.
2. Work only on new project copies: `Projects/<Take>_final/` or a new versioned folder. Back up before every edit (`<name>_pre<Change>.fcproj/.srt`, for example `_prePause500`).
3. Never delete. Move replaced finals into `Acceptable/<Take>/superseded_before_<change>/`.
4. Never re-render, trim or hack finished mp4s on the box. Every change (words, captions, end card, 4:5) is made in the real fcproj on the Mac, then rendered from ReelSmith.
5. Nothing goes into the the user's downloads folder unless the reviewer asks. Temporary frame grabs are for checking only; delete them afterwards.
6. Save, render and refresh the SRT after every edit.
7. Never print the ReelSmith control token, the ElevenLabs key, the Netlify token or `META_ACCESS_TOKEN`. Don't copy them anywhere new.

## 3. Folder layout (on the Mac, `<ClientRoot>`)

- `<SourceFolders>`: source takes (read-only). Typically `Original-Videos/` plus one folder per ad with the cleaned audio remuxed in.
- `even_better_audio/<take>_isolated.wav`: isolated voice tracks. `Clean-Audio/`, `Enhanced-Audio/`, `Denoise-Pairing-Index.csv`: audio comparisons (optional).
- `Projects/<Take>_final/`: working projects, `work/` plans, logs, backups.
- `Acceptable/<Take>/`: finals for A, B and C (mp4, srt, fcproj) plus `DECISIONS.md`, `4x5/` and `superseded_before_*/`.
- `PitchFix/`: word-swap comparison material (for example `PitchFix/<take>_<word>/`).
- `Authority_Landscape/`: authority-section work for landscape takes (only if needed).
- `pipeline/`: scripts, `PRINCIPLES.md`, `style.json`, `style.md` (the style sheet), `takes.md`, `directions/<take>.json`, `picks/`, `broll_library.json`.
- iCloud copy: `<SyncRoot><Take>/`.

## 4. Audio pipeline

1. Extract 48 kHz mono WAV from the take: `ffmpeg -nostdin -i take.mp4 -vn -ac 1 -ar 48000 take.wav`.
2. Run ElevenLabs Voice Isolator on the WAV (key is an env/secret, never printed). It beat DeepFilterNet and resemble-enhance in an earlier comparison. Output: `even_better_audio/<take>_isolated.wav`, remuxed into the per-ad source take mp4s.
3. Optionally run resemble-enhance locally on CPU. If it sounds over-processed, keep the isolated version. (On the previous project's finals, the isolated track was used.)
4. Sync-check the isolated track against the camera audio (length and offset; expect differences of a few hundredths of a second).
5. Loudness: about -24 LUFS integrated (within 1 LU of the `<RefTake>` final), LRA about 4 LU, true peak at or below -2 dBTP. Only apply clip gain if a take is more than 1 LU off. No limiter, no compressor. Check with `ffmpeg -nostdin -i f -map 0:a -af ebur128=peak=true -f null -`.
6. Put a 2-frame constant-power crossfade, centred, on every audio cut. (Whole-frame crossfades only; this is a ReelSmith limit.)
7. Keep camera audio only for "before" comparisons.
8. Watch for joins between different recording times that sound brighter or darker; note them in DECISIONS.md.

## 5. Transcript, forced alignment and cut planning

1. Transcribe with Whisper large-v3-turbo (ReelSmith has no built-in transcription). Whisper often mis-hears names and adds a fake "Thank you." over silence. List the speaker's known mis-hearings in the style sheet and add the fixes to `plan_take.py`'s spelling corrections.
2. Force-align with wav2vec2 (`take_align.py`). Use these word times, not Whisper's, which drift up to about 0.3 s. If two words overlap, the word falls back to Whisper's interval, clipped to its neighbours. Fix any misplaced words using the energy profile.
3. Write `pipeline/directions/<take>.json` (lower third and end-card text for the event).
4. Plan each version: `python3 plan_take.py <words.json> <version.json>` writes `<version>.plan.json` (kept sentences, tightened pauses, every cut in measured silence, resulting length).
   - **A full:** remove only dead air. Silence below -45 dB for at least 0.3 s, gaps over 0.5 s only. Leave 0.40 s after a sentence, 0.33 s after a comma, 0.28 s mid-phrase, 0.25 s before the first word, 0.6 s after the last, then a 1.6 s end-card hold. Remove fillers only if they are not in the approved script.
   - **B trim:** pauses to about 0.2 to 0.3 s plus obvious trims.
   - **C under 60 s:** at or just under 60.0 s (target 57 to 60 s). Remove whole redundant sentences first, then tighten pauses to about 0.2 s (0.1 s minimum inside a phrase). Never remove half a sentence when a whole one can go.
   - Always keep: the hook, the credibility line, the problem statement, the event line (`<EventDate>`, `<EventPlace>`) and the CTA. The client's exact must-keep lines go in the style sheet.
   - Usually cut first: examples, reflective asides, extra tails, rhetorical follow-ups.
5. Cutting rules: cut only in silence (below -45 dB), at least about 0.05 s clear of word edges, never clip a word. Straight cuts (picture and sound together); split J/L cuts only if asked. Never cut a filler fused to the next word (for example "Now,"); flag it instead. Never leave a sentence ungrammatical at a join (for example a stranded "to"). Watch for leftover syllable fragments.
6. Under B-roll, dead air goes down to about 0.13 s of true silence in every version, A included. Shorten the B-roll clip over the join so its source stays continuous. Never cut inside a B-roll or word-fix span; move those blocks whole.
7. Build from the take's own transcript and alignment. Do not fetch or compare a website script unless the reviewer asks (a reviewer instruction). Start building in ReelSmith straight away.
8. Word fixes (only when the reviewer asks, or to fix a flagged problem):
   - Use the speaker's real voice, spliced from another take. Swap only the needed word or syllables and keep timing so lips stay in sync.
   - Build a labelled comparison (for example `<Take>_<word>_COMPARE.mp4` with ORIGINAL + C1 to C5, or a `PitchFix/` folder). the reviewer picks.
   - Method (`vofix.py`): cut A1 audio at frame boundaries in silence, mute that piece, lay the fix on A2, match level (earlier fixes needed about +1.3 to +1.7 dB), 8 to 20 ms crossfades on consonant boundaries. Picture is untouched. Example: `vofix.py --bridge 127.0.0.1:9876 --save-as <new.fcproj> --start-frame N --end-frame M --vo <wav>`.
   - Rejected on a previous project (don't offer by default): ElevenLabs voice clone (wrong voice, lips don't match) and Praat/PSOLA pitch shifting (sounded nasal).
   - Log every fix done in DECISIONS.md (word, target take, donor take).
   - After any swap: re-check with Whisper and wav2vec2, then fix caption text and punctuation (statements end with a period, not "?").
9. Takes with no credibility line: swap in the `<RefTake>` authority section (record its source in and out times in the style sheet). Scripts: `Authority_Landscape/work/authority_slot_live.py` (landscape) and `authority_slot_live_vert.py` (vertical). Only when the reviewer asks.
10. Weak or different CTA: graft the reference CTA take's ending in silence with a 2-frame crossfade and matched loudness (`endgraft_live.py`). Only when the reviewer asks; a take keeps its own CTA if the reviewer says so.

## 6. Build in ReelSmith (live)

1. Command: `python3 build_take.py <plan.json> --direction directions/<take>.json --out <dir>/<name> --bridge 127.0.0.1:9876` (add `--dry` to test). It does: cuts, 2-frame crossfades, montage over the credibility section, lower third, end card, captions, save, SRT, export.
2. Build the Full version first, then B and C.
3. Export: H.264 High, CRF 18, AAC, at the take's own size and fps.

## 7. Edit rules (shots and B-roll)

1. No visible clip of any kind (A-roll or B-roll) under 0.5 s (15 frames at 30 fps) between two visual cuts. `shotcheck.py` checks; `build_take.py` and `montage_shift.py` run `shotcheck.fix_broll` and report `short_shots_after`. Fix a sliver after the montage by extending the last B-roll clip to the A-roll cut.
2. "Jarring jump" means only a sub-0.5 s shot. Ordinary same-framing A-roll jump cuts are fine and are not flagged.
3. Never flash A-roll before B-roll. B-roll starts exactly on the A-roll cut, or after the new shot has been on screen a comfortable beat (about 1 s or more; 36 frames / 1.2 s worked before). Same at the end. Move the whole montage block with its lower third, keeping clip durations and source in-points (`montage_shift.py`).
4. Intro rule: the montage starts after the speaker's self-introduction. The exact trigger words are a style-sheet setting (section 15). If an A-roll cut falls 1 to 14 frames before the trigger word, start the montage exactly on that cut. Code: `shotcheck.montage_start_word` (update its trigger words for the client).
5. Montage: the client's approved B-roll picks (listed in `picks/<RefTake>_picks.json` and the style sheet). Equal frame-snapped slots (about 2 s), straight cuts, V2 picture only, landscape clips filled to 9:16 with centre-tracking keyframes. Never use a clip marked NEVER or unlicensed stock.
6. No credibility section and no authority swap: add no B-roll, and list candidate spots in DECISIONS.md.
7. Lower third: slides in from the left on the self-introduction. 2.0 s if over the montage's first clip, otherwise 3.0 s. Text, fonts, panel and accent colour are style-sheet settings.

## 8. Captions

1. Use the speaker's actual spoken words, with names spelled as in the style sheet.
2. Group by natural phrases and pauses: 1 to 4 words, about 20 characters or fewer. Break at sentence ends, commas and pauses of 0.15 s or more; never span a pause of 0.25 s or more.
3. Keep set phrases together (the client's list is in the style sheet; always include the speaker's name, the event date and the place). Never end a block on a dangling function word (at, the, to, a, of, and, your).
4. Timing from forced-aligned onsets: show at the onset (at most about 0.05 s early), clear about 0.1 s after the last word, minimum 0.3 s. No captions over the end card.
5. Scripts, in order: `align_captions.py` (wav2vec2 word times), `plan_captions.py` (grouping, writes `work/_cap_plan.json`), `apply_captions.py` (replaces C1 and C2 captions live, then save, SRT, MP4). After a manual timeline edit use `recaption_live.py <plan.json>`. Note: the committed versions of the first three have a hard-coded project path; point `D` at the target project first. **Unverified:** whether a generalised wrapper exists.
6. Caption look is a style-sheet setting. Burn into the mp4 and also write a `.srt`.
7. Check cue sync with `sync_check.py <render.mp4>` (reads the matching `.srt`): each cue should start within about -0.04 to +0.09 s of its first word.

## 9. End card and CTA

1. The end card starts at the spoken CTA trigger (for example "Tap the link") and holds to the last frame with the whole spoken CTA under it. Never trim it to save time. Fade in only.
2. Layout, text and font sizes are style-sheet settings (vertical and landscape). On landscape, keep the panel low enough that the speaker's face stays visible. Landscape script: `endcard_landscape_live.py`.
3. Fix scripts from earlier work: `endcard_date_fix.py` / `endcard_date_v4.py` (date), `endcard_cta_big.py` (CTA size), `endcard_cta_split.py`, `endcard_cta_nudge.py`. Each is hard-coded to a project path and to the old text; adapt a copy.
4. Event text comes from `directions/<take>.json`.
5. Record the reference spoken CTA and any open wording questions in the style sheet. Don't change unresolved wording without the reviewer.

## 10. 4:5 versions (Feed)

1. Only for vertical takes. Landscape-only takes get no 4:5 (list them in `takes.md`).
2. Never cut or slide end-card frames out of a finished video. Rebuild natively.
3. Generate a config (read-only inspect of the source): `python3 make_cfg_4x5.py <Take> <source.fcproj> <cfg.json>`. It prints the geometry and any pre-existing visible shots under 15 frames. It expects a 1080x1920 sequence.
4. Build: `python3 reframe_4x5_live.py <cfg.json> [--no-export]`. It copies the source to `<take_dir>/4x5/<Take>_U60_4x5.fcproj` (plus a `_pre4x5` backup copy), opens it in the live app, sets the sequence to 1080x1350 with `scaleMotion` off (the frame keeps the top 1350 rows, so the speaker keeps headroom), reframes B-roll, resizes and re-places captions (size scaled so pixel size is unchanged, margin 0.20), moves the lower third and re-lays-out the end card. Timing is never touched. Saves and exports srt and mp4. The original project is never opened for writing.
5. Verify: `python3 verify_4x5.py <render.mp4> <srt> <orig.mp4> <fcproj>` (checks size, frame count and duration against the original, whether audio is identical, and writes a contact sheet). `verify_all_4x5.py` runs all.
6. Output in `Acceptable/<Take>/4x5/`: fcproj, mp4, srt, build_log.json, verify.json, contact_sheet, keyframes.jpg. Delete temporary contact sheets only if you made them just for checking.

## 11. Render, QC and review

1. Render from ReelSmith (live export or `reelsmith-cli export`). Then run `verify_render.py <plan.json> <render.mp4> <build_log.json> <sheet.jpg>`: Whisper re-transcription against expected words, loudness against the `<RefTake>` reference, SRT checks (punctuation, none over the end card), contact sheet.
2. Checks: duration and frame count match the plan (a pause cut should be exactly N frames shorter); `shotcheck.py` shows no shot under 15 frames; no click at any join; loudness about -24 LUFS and true peak at or below -2 dBTP; C at or under 60.0 s.
3. Review page: one Netlify site per client at `<ReviewSite>`, built by a script in `<ReviewRoot>` (adapted from the previous `build_improved.py`; take lengths list in `R`). One row per take, the Under 60 cut as the focal card; Good to go / Not good enough buttons; sticky "Submit review" bar. Re-encode web copies for the site.
4. Deploy with the command in the machine profile (or publish the review page locally). After a remote deploy, check each page returns 200. Renaming a site breaks old links, so pick the name once.
5. Approval form: Netlify form `<ApprovalForm>`, which emails the reviewer. The hosting token lives only in the env var named by the machine profile; never print it.
6. the reviewer's good/not-good marks are the approval. Don't call a version accepted without them.

## 12. Delivery

1. Copy each approved version to `<ClientRoot>Acceptable/<Take>/`: mp4, srt, fcproj and `DECISIONS.md` (what changed, the reviewer's notes, backups, new lengths, checks, flags). File names: `<Take>_A_full_<len>s`, `<Take>_B_trim_<len>s`, `<Take>_C_60_<len>s`.
2. Copy to iCloud `<SyncRoot><Take>/` (mp4, srt and DECISIONS.md at least; whether the fcproj also goes there is **unverified**) and verify sha256. Move replaced iCloud files to `_icloud_removed_before_<change>/`.
3. Don't drop copies in the the user's downloads folder unless the reviewer asks. Watch disk space.
4. DECISIONS.md also lists flags: weak words, intonation, unresolved items.

## 13. Thumbnails and Meta handoff

1. Thumbnails: a clean, flattering A-roll frame of the speaker's face with eyes open. Never an end-card frame or its fade-in. Pull frames from the finished render with ffmpeg on the Mac or box, check each visually, delete rejects. (Exact extraction commands used before are **unverified**.)
2. Meta (account and IDs in `<MetaNotes>`; token only in env `META_ACCESS_TOKEN`): creatives use asset_feed_spec placement customization. 4:5 for Feed, 9:16 for Stories and Reels. Landscape-only ads use one landscape video everywhere.
3. Post captions follow the client's caption tone in the style sheet. Live copy is `<MetaCaptions>`.
4. Keep everything PAUSED. Never set anything ACTIVE; don't touch billing.
5. Upload helpers and videos go in `/workspace/meta_upload/<Client>/` (subfolders such as `4x5`, `4x5_mac`).
6. Campaign restructuring outside the edit is out of scope.

## 14. Starting a new client

- [ ] Fill in the Project variables (section 0).
- [ ] Create `<ClientRoot>` with: `Original-Videos/`, per-ad source folders, `even_better_audio/`, `Projects/`, `Acceptable/`, `PitchFix/`, `pipeline/` (copy scripts from the last project; `directions/`, `picks/` empty).
- [ ] Mark source folders read-only in your head and in `PRINCIPLES.md`.
- [ ] Write `pipeline/takes.md`: take names, orientation (vertical or landscape-only), which take is `<RefTake>`.
- [ ] Write the style sheet `pipeline/style.md` (section 15) and `style.json` to match.
- [ ] Write `pipeline/directions/<take>.json` for every take (lower third and end-card text, event date and place).
- [ ] Get the reviewer's approved B-roll picks into `picks/` and `broll_library.json`; mark any NEVER clips.
- [ ] Find and fix hard-coded paths and text in reused scripts (caption scripts, end-card fix scripts, `shotcheck.montage_start_word`, `plan_take.py` spelling fixes).
- [ ] Set up `<ReviewRoot>` with the review-page builder and deploy script; create the Netlify site `<ReviewSite>` and form `<ApprovalForm>`; confirm the review notification covers the new form.
- [ ] Create the iCloud folder `<SyncRoot>`.
- [ ] Create `<MetaNotes>` (account, page and ad IDs; no tokens) and `<MetaCaptions>`.
- [ ] Check Mac disk space before the first render.

## 15. Per-client style sheet

Copy this into `<ClientRoot>pipeline/style.md` and fill it in. Example values are what the previous client used; treat them as defaults, not rules.

**Speaker and spelling**
- Name spelling in captions: example `Jane Doe` (watch that Whisper doesn't respell it).
- Known Whisper mis-hearings to fix: example "Jon" heard as "John".

**Script must-keeps**
- Credibility line: example "I'm `<Speaker>` ... the credibility line from the style sheet".
- Event line: example "March 3, Austin, TX".
- Reference spoken CTA: example "Tap the link below and grab 15 minutes with me and I'll be straight about whether this is the right room for you."
- Reference CTA take for grafts, and authority section source times: example `<RefTake>` authority section at about 23.7 to 33.2 s (9.4 s).
- Open wording questions: example "Book your spot" vs "15-minute call" (unresolved; don't change).

**Intro rule**
- Example: if the speaker opens with "Hi", keep the face on screen through "Hi, I'm `<Speaker>`" and start B-roll right after `<SpeakerSurname>`. With no "Hi", start the montage on "I'm `<Speaker>`".

**B-roll montage**
- Approved clips and order: example five clips ordered youngest to oldest speaker, about 2 s each, from `picks/<RefTake>_picks.json`.
- NEVER clips: list them.

**Lower third**
- Example: "`<Speaker>`" Inter Bold 72 / title line (e.g. "Founder, `<Company>`") Medium 40, on a 55% black rounded panel with a gold `#C9A24A` bar, slides in from the left. Landscape panel 560 px wide.

**Captions**
- Example: Inter Bold, white, 4 px black outline, bottom anchor. Vertical 50 px, 30% margin. Landscape 57.5 px at 12% margin.
- Set phrases to keep together: example "spring workshop", "Austin", `<Speaker>`, "15 minutes".

**End card**
- Vertical example: black 70% rounded panel 1000x680. Title (e.g. "Spring Workshop") Bold 76. Gold rule. Date line "`<EventDate>` · `<EventPlace>`" SemiBold, white, 64 px. CTA on two lines (e.g. "Tap the link below" / "to grab 15 minutes with `<Speaker first name>`"), Medium 54 px, box height 150.
- Landscape example: panel 1520x450 at y 576 to 1026; headline on one line, Bold 72.
- CTA trigger words that start the card: example "Tap the link".

**Meta post captions**
- Example tone: written in first person as the speaker, never the speaker's name in third person, sell the sizzle (hook plus one short event line), no event mechanics in blurbs. Source of tone: the client's own page.

---

## Pre-delivery QC checklist

- [ ] Built from a project copy, backup exists, sources untouched.
- [ ] Audio: isolated (or enhanced only if it sounds natural), -24 LUFS (within 1 LU), true peak at or below -2 dBTP, 2-frame crossfades, no clicks.
- [ ] Whisper re-transcription: no clipped words, fragments or broken grammar at joins.
- [ ] No visible shot under 15 frames; no A-roll flash before B-roll.
- [ ] Intro rule from the style sheet followed.
- [ ] Captions: phrase groups, no dangling function words, set phrases together, forced-aligned, no captions on the end card, correct punctuation, names spelled per the style sheet; sync_check clean.
- [ ] End card from the CTA trigger to the last frame; text and sizes per the style sheet; no trim.
- [ ] C at or under 60.0 s; frame count matches the plan.
- [ ] 4:5 built natively (headroom, captions, end card), verify_4x5 passes; none made for landscape-only takes.
- [ ] DECISIONS.md written; finals in Acceptable and iCloud with checksums; nothing in the user's top-level downloads folder.
- [ ] Thumbnails are A-roll face frames, eyes open, no end card.
- [ ] Review page updated; the reviewer's good/not-good received.
- [ ] Everything in Meta still PAUSED. No secrets printed anywhere.
