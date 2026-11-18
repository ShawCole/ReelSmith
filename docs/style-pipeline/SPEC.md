# Style pipeline runbook: learn, suggest, apply (any machine, any client)

This pipeline learns an editing style from a user's past finished ads, reviews new drafts against that style, and turns raw takes into an improved edit in the same style. It works on any machine, with any editor backend, for any client.

It builds on `AD_RUNBOOK.md` (the client-agnostic runbook). Every craft and safety rule there still applies. This file makes those rules machine-agnostic and profile-driven.

**How to read the status markers**
- **[existing]**: this exists and has been used (it came from the ReelSmith project scripts).
- **[design proposal]**: new design. The script, file or schema does not exist yet. Its CLI and format are specs to build against.
- **[unverified]**: carried over from the source and never confirmed.

No secret values appear in this file. Secrets are named only by the env var that holds them.

---

## 0. Overview

```
            past finals + notes + approvals
                         │
                 (1) EXTRACT  ──►  style_profile.json  (house  ⊕  client)
                                         │
     draft / raw take ──► (2) SUGGEST ──►  suggestions.json + diff report
                                         │
     raw take ─────────► (3) APPLY  ──►  plan → edit → render → QC
                                         │
                         human review (good / not good)  ◄── approval gate
                                         │
                         (4) FEEDBACK ──►  profile v+1
```

Three config files drive everything:
- `machine.yaml`: where things live on this machine and which tools to use (section 1).
- `style_profile.json`: how this client's ads should look and sound (section 4). It is layered over a house profile.
- `project.yaml`: the per-project variables from the earlier runbook (client, speaker, event, takes) (section 2).

Nothing in the pipeline hard-codes a path, a machine, a client or an editor.

---

## 1. Machine profile (`machine.yaml`) [design proposal]

Each machine gets one `machine.yaml`. Scripts find it from `--machine <path>`, else `$STYLEPIPE_MACHINE`, else `./machine.yaml`.

```yaml
# machine.yaml  (schema v1)
name: my-laptop
os: macos            # macos | linux | windows
shell:
  wrap: null         # optional prefix for remote command runners, e.g. "/bin/bash -c"
paths:
  workspace_root: ~/StylePipe            # all clients live under here
  client_root: "{workspace_root}/{client}_Ads"
  temp_dir: "{workspace_root}/_tmp"      # frame grabs etc.; cleaned after checks
  min_free_gb: 20                        # refuse to render below this
tools:
  ffmpeg: ffmpeg                         # or an absolute path
  ffprobe: ffprobe
  python: python3
  whisper_model: large-v3-turbo
  whisper_device: auto                   # cpu | cuda | mps | auto
  aligner: whisperx                      # whisperx | wav2vec2
editor:
  backend: ffmpeg                        # ffmpeg | reelsmith-live | reelsmith-headless
  reelsmith:
    cli: reelsmith-cli
    bridge: 127.0.0.1:9876
    token_env: REELSMITH_CONTROL_TOKEN
    token_file: ~/.config/reelsmith/control_token   # path only; never read into logs
    pace: 0.25
voice_isolation:
  provider: none                         # none | elevenlabs | deepfilternet | resemble-enhance
  key_env: ELEVENLABS_API_KEY
storage:
  deliver_to: "{client_root}/Acceptable"
  sync:
    type: none                           # none | icloud | dropbox | gdrive | rsync | s3
    target: null
    verify: sha256
review_site:
  type: local                            # local | netlify | static-host | none
  site_name: "{client}-ads-in-review"
  deploy_cmd: null                       # e.g. a deploy script; runs as-is, logs no secrets
  token_env: NETLIFY_AUTH_TOKEN
  approval_form: "{client}-approval"
publish:
  meta:
    enabled: false
    token_env: META_ACCESS_TOKEN
    notes_file: "{client_root}/meta_notes.md"
    force_status: PAUSED                 # cannot be changed by config
```

`{workspace_root}`, `{client}` and `{client_root}` are filled in at run time.

### Example profile: a laptop

```yaml
name: edit-laptop
os: macos
shell:
  wrap: null
paths:
  workspace_root: ~/StylePipe
  client_root: "~/StylePipe/{client}_Ads"
  min_free_gb: 20
tools:
  ffmpeg: ffmpeg          # always run with -nostdin
  whisper_model: large-v3-turbo
  aligner: wav2vec2
editor:
  backend: reelsmith-live
  reelsmith: { bridge: 127.0.0.1:9876, token_env: REELSMITH_CONTROL_TOKEN,
               token_file: ~/.config/reelsmith/control_token, pace: 0.25 }
voice_isolation: { provider: elevenlabs, key_env: ELEVENLABS_API_KEY }
storage:
  sync: { type: none, target: null, verify: sha256 }
review_site:
  type: local
  deploy_cmd: null
publish:
  meta: { enabled: false, token_env: META_ACCESS_TOKEN, force_status: PAUSED }
```

Notes for this profile: point `tools.ffmpeg` at an absolute path when it is not on `PATH`. A second machine can do audio, transcripts and review pages while the edit runs where ReelSmith is open. How the app was first launched with `--control 9876` is **[unverified]**.

---

## 2. Project config (`project.yaml`) [design proposal]

These are the project variables from `AD_RUNBOOK.md` §0, now saved as a file at `{client_root}/project.yaml`:

```yaml
client: AcmeCo
speaker: { name: "Jane Doe", surname: "Doe", first: "Jane" }
project: "Spring Workshop"
event: { date: "March 3", place: "Austin, TX" }
house_profile: profiles/house/style_profile.json
client_profile: "{client_root}/profile/style_profile.json"
ref_take: Ad1-Long
takes:
  - { name: Ad1-Long, orientation: vertical }
  - { name: Ad2-Short-A, orientation: landscape-only }
versions: [A_full, B_trim, C_60]     # default convention
```

---

## 3. Requirements and install

| Tool | Needed for | Required? |
|---|---|---|
| ffmpeg + ffprobe (with libass, libx264, ebur128 filter) | everything: decode, loudness, scene cuts, render, captions burn-in | yes |
| Python 3.10+ | all scripts | yes |
| Python packages: `numpy`, `pyyaml`, `jsonschema`, `soundfile`, `pysubs2`, `scenedetect`, `opencv-python` | analysis, SRT/ASS, shot detection, frames | yes |
| Whisper (`openai-whisper` or `faster-whisper`), model large-v3-turbo | transcription | yes |
| Forced aligner: `whisperx` (bundles wav2vec2) or `torchaudio` wav2vec2 | word timings | yes |
| `pytesseract` + Tesseract | reading burned-in caption/end-card text from past finals | optional |
| Voice isolation: ElevenLabs Voice Isolator (API), DeepFilterNet or resemble-enhance (local) | cleaning dialogue | optional |
| ReelSmith + `reelsmith-cli` | the ReelSmith backends | optional |

Install notes:
- **macOS:** `brew install ffmpeg tesseract python@3.11`. On Apple Silicon, use `whisper_device: mps` or `cpu`.
- **Linux (Debian/Ubuntu):** `sudo apt install ffmpeg tesseract-ocr python3-venv`. Use CUDA if there's a GPU.
- **Windows:** `winget install Gyan.FFmpeg`, Python from python.org, Tesseract from the UB Mannheim build. Use PowerShell. Put paths with spaces in quotes in `machine.yaml`.
- **All platforms:** `python3 -m venv .venv && .venv/bin/pip install -r requirements.txt` (requirements file **[design proposal]**). Check the install with `stylepipe doctor` **[design proposal]**, which prints tool versions, free disk space and which secret env vars are set (it prints "set" or "missing", never the value).

Always run ffmpeg with `-nostdin` in scripts.

**Secrets:** these come only from env vars named in `machine.yaml` (`ELEVENLABS_API_KEY`, `REELSMITH_CONTROL_TOKEN`, `NETLIFY_AUTH_TOKEN`, `META_ACCESS_TOKEN`). Never print, log, copy or commit them. Scripts must redact any env value whose name ends in `_TOKEN` or `_KEY` from their logs.

---

## 4. Editor backend abstraction [design proposal]

All stages talk to one interface. The backend is chosen by `editor.backend`.

```python
class EditorBackend:
    def open_copy(self, source, dest) -> Project          # always a copy; never opens a source for writing
    def inspect(self) -> dict                             # path, dirty flag, tracks, fps, size
    def apply_plan(self, plan: dict) -> None              # cuts, crossfades, B-roll, lower third, end card
    def set_captions(self, cues: list) -> None
    def reframe(self, aspect: str, cfg: dict) -> None     # e.g. "4:5"
    def save(self, backup_suffix: str) -> Path            # writes <name>_pre<Change> backup first
    def render(self, out: Path, preset: dict) -> Path
    def export_srt(self, out: Path) -> Path
```

| Backend | How it works | When to use |
|---|---|---|
| `reelsmith-live` **[existing tools]** | MCP client `rsmcp.py` → `reelsmith-cli mcp --bridge <bridge>`; `fc.tool('project_inspect')`, `fc.run('<command id>', ...)`. Builds paced in the open window. | The user wants to watch edits happen |
| `reelsmith-headless` **[existing tools]** | `reelsmith-cli --project <copy.fcproj>` with `inspect`, `exec`, `run <script.jsonl>`, `export --preset`, `render`, `--save/--save-as` | ReelSmith is installed but nobody needs to watch |
| `ffmpeg` **[design proposal]** | The plan becomes an EDL, then an ffmpeg `filter_complex` script: `trim`/`atrim` segments, `acrossfade` (constant power, `c1=qsin:c2=qsin`) at cuts, `overlay` for B-roll and lower-third/end-card PNGs (rendered with Pillow from the profile's layout), `subtitles`/`ass` filter for burned captions, and `crop`/`scale` for 4:5. The "project file" is the `plan.json` + `edl.json` + generated assets, so it can be rebuilt any time. | Any machine; CI; no GUI editor |

Rules that apply to every backend:
- Check `inspect()` before editing. If the open project isn't the expected one, or has unsaved changes, stop. If it has unsaved user changes, save them as they are with a `_useredits`-style backup (`_useredits`) first.
- Live scripts act on whatever project is open, so open the target first.
- The existing ReelSmith scripts (`build_take.py`, `vofix.py`, `montage_shift.py`, `reframe_4x5_live.py`, caption and end-card scripts) become the internals of the ReelSmith backend. Several have hard-coded project paths and old text, so parameterise them first.
- Export defaults (set in the profile): H.264 High, CRF 18, AAC, at the source's own size and fps.

---

## 5. Stage 1: Extract (train a style profile) [design proposal]

### 5.1 Inputs (the "training set")

Put the user's previous finished content under `{client_root}/training/` (or point `--input` at it). Nothing here is ever modified.

```
training/
  <Item>/
    final.mp4            # required
    final.srt            # optional; improves caption features
    project.fcproj|.edl  # optional; gives exact cuts, fonts, colours
    raw/  *.mp4          # optional; the source takes, for raw→final deltas
    DECISIONS.md         # optional; learned rules
    approval.json        # optional: {"version":"C_60","mark":"good"|"not_good","note":"...","by":"...","at":"..."}
```

Approval marks can also come from the review-site form export (CSV/JSON). Map them into `approval.json`.

### 5.2 CLI spec

```
extract_style.py --machine machine.yaml --input <training_dir> \
    --client <Client> [--house profiles/house/style_profile.json] \
    --out {client_root}/profile/style_profile.json [--min-examples 3] [--report extract_report.md]
```

Exit 0 on success. It writes a new profile version, never overwriting the old one (`style_profile.v<N>.json`, with `style_profile.json` pointing to the latest).

### 5.3 Features and how to measure them

For each feature, compute per-item values, then the profile stores the median, P10–P90 and a tolerance band.

| Feature | How to measure |
|---|---|
| Integrated loudness (LUFS), LRA | `ffmpeg -nostdin -i f -map 0:a -af ebur128=peak=true -f null -` and parse the summary `I:` and `LRA:` |
| True peak (dBTP) | same command, `Peak:` with `peak=true` |
| Compression/limiting present | crest factor (peak-to-RMS) per 400 ms window; a flat crest suggests a limiter |
| Speech pace (WPM) | Whisper words ÷ speech-active minutes (from aligner word spans) |
| Pause lengths | gaps between aligned words, bucketed: after sentence end, after comma, mid-phrase, before first word, after last word |
| Dead-air floor | silence threshold and minimum gap removed, inferred by comparing raw→final when raw exists; otherwise the shortest gaps kept |
| Audio crossfade length | at each detected cut, look for a short gain dip or overlap in the waveform (±3 frames). Read exactly from fcproj/EDL if present |
| Cut rate | shot boundaries (`scenedetect` ContentDetector, or the fcproj/EDL) per minute |
| Shot length distribution | durations between cuts, in frames: min, P10, median; the profile records `min_shot_frames` (source rule: 15 frames at 30 fps = 0.5 s) |
| A-roll / B-roll ratio and placement | classify shots by face detection (OpenCV DNN or Haar) plus match to raw takes; record % B-roll, where the montage starts relative to the intro words, slot length, montage clip count |
| Montage timing rule | offset between the self-intro trigger word end and the first B-roll frame; offset between A-roll cut and B-roll start (no "A-roll flash" means 0 frames or ≥ ~30 frames) |
| Caption phrase length | from SRT: words per cue, chars per cue, cue duration, break reasons (sentence/comma/pause), dangling function words at the end of a cue (should be 0) |
| Caption sync | cue start minus aligned first-word onset (source target: −0.04 to +0.09 s) |
| Caption look | from fcproj if present (font, size, outline, colour, anchor, margin). Otherwise from frames: crop the caption area, OCR it, measure text height in px, sample fill/outline colours, vertical position as % of frame. Mark confidence low when inferred |
| Lower third | detect a static overlay in the first ~10 s (frame differencing); position, size, panel opacity, accent colour by pixel sampling; OCR the text; in/out times |
| End card | start time vs the spoken CTA trigger word (aligned transcript); hold to last frame (y/n); fade in/out; panel size and position; text sizes via OCR box height; no captions over it (check SRT) |
| Hook structure | first sentence from the transcript: length in seconds and words, type (question, bold claim, "Hi, I'm…" intro), time until name and credibility line |
| Script structure | order of sections, tagged by keyword/LLM classification: hook, credibility, problem, event line, CTA. Record which are always kept in cut-downs |
| Length targets | final durations per version label (A/B/C), e.g. C at 57–60 s |
| Aspect ratios and sizes | `ffprobe` width/height/fps per final; which versions exist per orientation (e.g. 4:5 only for vertical) |
| Export settings | `ffprobe` codec, profile, bitrate; CRF is inferred only if stated in notes |
| Thumbnail rule | from delivered thumbnails if present: A-roll face, eyes open (eye-aspect-ratio check), never end card |

### 5.4 Learned rules from notes and approvals

- **DECISIONS.md:** parse it into rule candidates. Lines with "never", "always", "don't", "the reviewer prefers or rejected", "keep", plus lists of fixes. Each becomes `{rule, scope, source, quote}`. Examples from the source project: never cut a filler fused to the next word; never leave a sentence ungrammatical at a join; voice cloning and PSOLA pitch shifting were rejected.
- **Approved vs rejected deltas:** for every item with both a good and a not-good version, diff their measured features (pause lengths, length, shot lengths, caption sizes, end-card timing). Features that consistently move the same way from rejected to approved become rules, with direction and size (for example "approved versions are 0.1 s tighter after sentences").
- **Weighting:** approved finals count fully, unmarked finals at 0.5, not-good versions only as negative examples.
- **Not learned:** the hard safety rules in §9. Those are fixed and not part of the profile.

### 5.5 `style_profile.json` schema sketch

```json
{
  "schema": "stylepipe.style_profile/1",
  "profile_id": "acmeco",
  "version": 3,
  "extends": "house@7",
  "created_at": "2026-11-18T14:10:00-05:00",
  "training_set": [{"item": "Ad1-Long", "mark": "good", "sha256": "..."}],
  "features": {
    "audio.loudness_lufs": {
      "value": -24.0, "tolerance": 1.0, "observed": [-24.1, -23.8, -24.3],
      "confidence": 0.9, "source": "measured", "examples": ["Ad1-Long/C_60"]
    },
    "audio.true_peak_max_dbtp": {"value": -2.0, "confidence": 0.9, "source": "measured"},
    "audio.crossfade_frames": {"value": 2, "shape": "constant_power", "confidence": 0.6, "source": "fcproj"},
    "pace.wpm": {"value": 158, "p10": 145, "p90": 170, "confidence": 0.8},
    "pauses.after_sentence_s": {"by_version": {"A_full": 0.40, "B_trim": 0.25, "C_60": 0.20}, "confidence": 0.7},
    "cuts.min_shot_frames": {"value": 15, "at_fps": 30, "confidence": 1.0, "source": "rule"},
    "broll.montage": {"start_rule": "after_self_intro", "trigger_words": ["Hi, I'm {speaker}"], "slot_s": 2.0, "clips": 5, "confidence": 0.7},
    "captions.grouping": {"max_words": 4, "max_chars": 20, "break_pause_s": 0.15, "never_span_pause_s": 0.25,
                          "set_phrases": ["{speaker}", "{event.date}"], "confidence": 0.85},
    "captions.look": {"font": "Inter Bold", "size_px": {"vertical": 50, "landscape": 57.5}, "outline_px": 4,
                      "color": "#FFFFFF", "anchor": "bottom", "margin_pct": {"vertical": 30, "landscape": 12},
                      "confidence": 0.5, "source": "ocr+pixels"},
    "endcard": {"start": "cta_trigger", "trigger_words": ["Tap the link"], "hold_to_end": true, "fade": "in_only",
                "no_captions": true, "layout_ref": "assets/endcard_vertical.json", "confidence": 0.8},
    "length.targets_s": {"C_60": {"min": 57, "max": 60.0}},
    "aspects": {"vertical": ["9:16", "4:5"], "landscape-only": ["16:9"]},
    "script.must_keep": ["hook", "credibility", "problem", "event_line", "cta"],
    "script.cut_first": ["examples", "asides", "tails", "rhetorical_followups"]
  },
  "rules": [
    {"id": "R12", "text": "Never cut a filler fused to the next word; flag it.", "source": "DECISIONS.md",
     "examples": ["Ad3-Short-B"], "confidence": 0.9}
  ],
  "approval_deltas": [
    {"feature": "pauses.after_sentence_s", "direction": "shorter", "delta": -0.08, "n_pairs": 4, "confidence": 0.6}
  ],
  "spelling": {"names": ["Jane Doe"], "asr_fixes": {"Jain": "Jane"}}
}
```

**Confidence:** 0 to 1. It is based on example count, agreement (spread compared with tolerance) and source quality: `rule` (stated by the user) > `fcproj`/`edl` (exact) > `measured` > `ocr+pixels` (inferred). Features under 0.4 are reported but not enforced in Apply. Every feature lists `examples` so a suggestion can point to the actual past ad.

### 5.6 Layering

`effective = house ⊕ client ⊕ project overrides`. Later layers win field by field. The house profile holds the user's defaults across all clients (seeded from the craft rules in §9 and the source project's values). The client profile holds what was learned from that client's finals plus the per-client style sheet (`AD_RUNBOOK.md` §15). `project.yaml` can override things like event text. `stylepipe show-profile --effective` **[design proposal]** prints the merged result with each field's origin.

---

## 6. Stage 2: Suggest (review a draft or raw take) [design proposal]

```
suggest.py --machine machine.yaml --profile <effective or client profile> \
    --input <draft.mp4 | raw.mp4> [--srt draft.srt] [--project draft.fcproj|edl.json] \
    [--version C_60] --out suggestions/<name>/ [--top 20]
```

What it does:
1. Measures the input with the same feature extractors as Extract (shared module, so the numbers are comparable).
2. Compares each feature with the profile's value and tolerance, and checks every rule (§9 checks plus learned rules).
3. Ranks findings by `severity × confidence × fix ease`. Severity order: safety/hard rule > QC fail (loudness, short shots, caption over end card, over length) > style drift > polish.

Outputs:
- `suggestions.json`: `[{id, rank, severity, feature, observed, expected, evidence: {timecodes, frame_grab, transcript_snippet}, profile_examples, proposed_fix: {action, params}, auto_fixable}]`
- `diff_report.md` (and an HTML version for the review site): a table of feature, draft, profile, delta, pass/warn/fail, with timecoded notes ("00:12.40 shot of 9 frames < 15", "caption ends on 'the' at 00:31.2", "C is 61.3 s > 60.0").
- Frame grabs go in `{temp_dir}` and are deleted after the report is built, unless kept on purpose as report assets.

For raw takes it also proposes a plan (which sentences to drop for each version, pause targets) without rendering anything.

Suggest never modifies the input.

---

## 7. Stage 3: Apply (raw → improved edit) [design proposal]

```
apply_style.py --machine machine.yaml --project project.yaml --take <Take> \
    --versions A_full,B_trim,C_60 [--profile <path>] [--backend ffmpeg|reelsmith-live|reelsmith-headless] \
    [--dry] [--stop-after plan|edit|render] [--aspect 4:5]
```

Steps (each one writes into `{client_root}/Projects/<Take>_final/work/` and logs to `build_log.json`):

1. **Preflight:** run `doctor`, check free disk ≥ `min_free_gb`, check sources are read-only (or treat them as such), take a copy.
2. **Audio:** extract a 48 kHz mono WAV. Run voice isolation if `voice_isolation.provider` is set (optionally enhance; keep the isolated track if enhancement sounds over-processed). Sync-check against camera audio. Normalise only if more than the profile tolerance off (clip gain only; no limiter/compressor unless the profile says otherwise).
3. **Transcribe + align:** Whisper → forced alignment (whisperx/wav2vec2). Apply the profile's `spelling.asr_fixes`. Drop hallucinated "Thank you." over silence. Use aligned times, not Whisper's.
4. **Plan** (an existing `plan_take.py`, generalised to read pause targets, silence threshold, must-keep and cut-first from the profile): per version, keep or drop whole sentences and tighten pauses. Every cut goes in measured silence, clear of word edges. Hit the length targets. Output `<version>.plan.json`.
   - **Gate A (optional):** Suggest runs on the plan, and the user can approve or adjust it before editing.
5. **Edit** through the backend: open a copy, apply the plan, crossfades, montage per the profile rule, lower third, captions (grouped and timed per profile), end card. Save with a `_pre<Change>` backup.
6. **Render** with the profile's export preset, then export the SRT.
7. **QC against the profile** (§9 checklist as automated checks, plus Whisper re-transcription of the render for clipped words, fragments and grammar at joins). Any fail stops here with a report. No silent "fixes" to the rendered mp4: fixes go back into the project and get re-rendered.
8. **4:5** (only for vertical takes when the profile lists 4:5): rebuild natively from a copy of the project, never by cropping a finished mp4. Then verify size, frame count, duration and identical audio against the 9:16 version.
9. **Review:** build or update the review page (local HTML, or deploy via `review_site.deploy_cmd`), one row per take with good / not-good buttons.
   - **Gate B (required):** nothing is "accepted" until the reviewer marks it good.
10. **Deliver** (only after Gate B): copy to `storage.deliver_to/<Take>/` with `DECISIONS.md` (what changed, the reviewer's words, backups, lengths, checks, flags). Sync to `storage.sync` and verify sha256. Move replaced files into `superseded_before_<change>/`.
11. **Publish handoff** (optional, only when `publish.meta.enabled`): upload creatives and create ads with status **PAUSED**. Always PAUSED; the script refuses ACTIVE whatever the config says, and never touches billing. Campaign restructuring is out of scope.

Word fixes (splicing a word from another take of the same speaker), authority-section swaps and CTA grafts are **never automatic**. Apply only proposes them as suggestions, and they run on request (ReelSmith backend: existing `vofix.py`, `authority_slot_live*.py`, `endgraft_live.py`; ffmpeg backend: **[design proposal]** equivalents).

---

## 8. Stage 4: Feedback loop [design proposal]

```
feedback.py --profile {client_root}/profile/style_profile.json \
    --marks <review_export.json|approval.json...> --builds Projects/*/work/build_log.json \
    [--promote-house] --out style_profile.v<N+1>.json
```

- Each reviewed version, with its mark and note, is added to the training set (the measured features come from its `build_log.json`/QC).
- Good: reinforce the features it used (raises confidence, narrows tolerance towards its values).
- Not good: record it as a negative example. If the reviewer's note names a cause ("captions too big", "pause too long"), turn it into a candidate rule for the user to confirm.
- Recompute `approval_deltas`. Write a new profile version with a changelog (`profile/CHANGELOG.md`: feature, old → new, why, which marks). Never overwrite old versions, so any profile version can be rolled back.
- Rule changes from free-text notes stay `pending` until the user confirms them. Numeric drift is applied automatically but capped per update (e.g. ≤ 25% of tolerance) so one mark can't swing the style.
- `--promote-house` suggests moving features that are stable across ≥ 2 clients into the house profile (the user confirms).

---

## 9. Craft and safety rules as profile-driven checks

**Fixed safety rules** (not learnable; enforced in code in every stage and backend):
1. Never modify source takes or source folders. Work on copies only.
2. Back up before every edit (`<name>_pre<Change>`).
3. Never delete. Move replaced files into `superseded_before_<change>/` (and `_sync_removed_before_<change>/` on the sync target).
4. Never re-render, trim or patch a finished mp4 directly. Change the project and re-render.
5. Write nothing to the user's top-level folders (e.g. Downloads root) unless asked. Delete temporary frame grabs after checks.
6. Save, render and refresh the SRT after every edit.
7. Secrets come only from env vars. Never print, log or copy them.
8. The reviewer's good/not-good marks are the approval. Never auto-publish. Meta is always PAUSED; never touch billing.
9. Check free disk space before renders and sync copies; don't leave extra copies.

**Profile-driven checks** (the value comes from the effective profile; the source project's value is shown as the default):

| Check | Profile key | Default |
|---|---|---|
| Integrated loudness within tolerance | `audio.loudness_lufs` | −24 LUFS ± 1 LU |
| True peak | `audio.true_peak_max_dbtp` | ≤ −2 dBTP |
| No limiter/compressor | `audio.dynamics` | none |
| Crossfade on every audio cut, no clicks | `audio.crossfade_frames` | 2 frames, constant power, centred |
| Cuts only in silence, clear of word edges | `cuts.silence_db`, `cuts.word_margin_s` | −45 dB, 0.05 s |
| No clipped words / fragments / broken grammar at joins | (Whisper re-check) | must pass |
| Minimum visible shot | `cuts.min_shot_frames` | 15 frames at 30 fps (0.5 s) |
| No A-roll flash before/after B-roll | `broll.min_aroll_beat_frames` | 0 frames or ≥ ~30 (36 used before) |
| Montage starts per intro rule | `broll.montage.start_rule` | after self-intro trigger words |
| Under-B-roll dead air | `broll.dead_air_s` | 0.13 s |
| Pause targets per version | `pauses.*` | A: 0.40/0.33/0.28/0.25 lead/0.6 tail + 1.6 s end hold; B: ~0.2–0.3; C: ~0.2 (0.1 min) |
| Length target | `length.targets_s` | C 57–60.0 s |
| Must-keep sections present | `script.must_keep` | hook, credibility, problem, event line, CTA |
| Caption grouping | `captions.grouping` | 1–4 words, ≤ 20 chars, break at ≥ 0.15 s, never span ≥ 0.25 s |
| No dangling function word at cue end | `captions.no_end_words` | at, the, to, a, of, and, your |
| Caption timing | `captions.timing` | ≤ 0.05 s early, clear 0.1 s after, min 0.3 s; sync −0.04 to +0.09 s |
| No captions on end card | `endcard.no_captions` | true |
| Name spelling | `spelling.names` | per client |
| Caption look | `captions.look` | per client (see style sheet) |
| End card from CTA trigger to last frame, never trimmed, fade in only | `endcard.*` | per client |
| 4:5 rebuilt natively, vertical only, verify passes | `aspects` | 9:16 + 4:5 for vertical |
| Export preset | `export` | H.264 High, CRF 18, AAC, source size/fps |
| Thumbnails: A-roll face, eyes open, no end card | `thumbnail.*` | as stated |

### Pre-delivery QC checklist (generated from the profile by `apply_style.py`; every line is pass/fail with evidence)

- [ ] Built from a copy; backup exists; sources untouched (hash check against the stored source hashes).
- [ ] Audio within profile loudness/peak; crossfades on every cut; no clicks.
- [ ] Whisper re-transcription clean at every join.
- [ ] No shot under `min_shot_frames`; no A-roll flash before B-roll.
- [ ] Intro/montage rule followed.
- [ ] Captions pass grouping, timing, sync, spelling, end-word and end-card checks.
- [ ] End card per profile, untrimmed.
- [ ] Each version within its length target; frame count matches the plan.
- [ ] 4:5 versions native and verified; none for landscape-only takes.
- [ ] DECISIONS.md written; delivered and synced with checksums; nothing in top-level user folders.
- [ ] Thumbnails pass the thumbnail rule.
- [ ] Review page updated; reviewer's good/not-good received (Gate B).
- [ ] Anything published is PAUSED. No secrets in any log or report.

---

## 10. Folder layout (relative to `paths.client_root`)

```
<Client>_Ads/
  project.yaml
  sources/ (or Original-Videos/, per-ad folders)   read-only
  training/<Item>/...                              read-only inputs for Extract
  profile/style_profile.v<N>.json, style_profile.json, CHANGELOG.md, assets/ (end-card/lower-third layouts, B-roll picks)
  audio/<take>_isolated.wav                        (replaces even_better_audio/)
  Projects/<Take>_final/work/                      plans, logs, backups, build_log.json
  suggestions/<name>/                              suggestions.json, diff_report.md
  Acceptable/<Take>/                               finals, DECISIONS.md, 4x5/, superseded_before_*/
  review_site/                                     generated review page
```

The house profile lives at `{workspace_root}/profiles/house/`.

---

## 11. Getting started on a new machine or client

1. Install the requirements (§3) and run `stylepipe doctor`.
2. Write `machine.yaml` (copy the template; for the edit machine use the example profile).
3. Set secret env vars only for the providers you use.
4. Create `{client_root}` and `project.yaml`. Put past finals, SRTs, projects, DECISIONS.md and approval marks under `training/`.
5. Run `extract_style.py`. Read `extract_report.md` and fix or confirm any low-confidence features (fonts, end-card text) in the client profile by hand.
6. Run `suggest.py` on one past final as a sanity check. It should come back mostly green.
7. Run `apply_style.py --dry` on one raw take, review the plan, then run for real.
8. Review page → good/not-good → `feedback.py`.

---

## 12. Build status

| Piece | Status |
|---|---|
| ReelSmith live/headless control, `rsmcp.py`, `build_take.py`, `plan_take.py`, `take_align.py`, caption scripts, `shotcheck.py`, `montage_shift.py`, `vofix.py`, `make_cfg_4x5.py`, `reframe_4x5_live.py`, `verify_4x5.py`, `verify_render.py`, `sync_check.py`, end-card scripts | **[existing]**, but several have hard-coded paths/text and assume the edit machine; they need parameterising |
| `machine.yaml`, `project.yaml`, `stylepipe doctor`, `show-profile` | **[design proposal]** |
| `EditorBackend` interface, ffmpeg backend | **[design proposal]** |
| `extract_style.py`, `suggest.py`, `apply_style.py`, `feedback.py`, `style_profile.json` schema | **[design proposal]** |
| OCR/pixel recovery of caption and end-card look from flat mp4s | **[design proposal]**; expect low confidence |
| Generalised caption wrapper | **[unverified]** whether one exists |
| How ReelSmith was first launched with `--control` | **[unverified]** |
| Whether project files are synced along with finals | **[unverified]** |
| Thumbnail extraction commands used before | **[unverified]** |
