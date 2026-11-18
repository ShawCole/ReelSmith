# ReelSmith Review Portal

Clients upload raw footage. The editor (a person or the agent) cuts it. The client reviews that cut on a timeline, edits it directly, comments on a frame or a time range, and marks it good or not good. Every one of those steps is kept.

This is the design for that portal, and the contract the first build in `apps/reelsmith-portal` implements. The style pipeline in [../style-pipeline/SPEC.md](../style-pipeline/SPEC.md) consumes the approval mark and the client edits. Nothing here publishes a video.

## 1. Why it is not the wasm editor

`apps/reelsmith-web` is the desktop egui interface compiled to WebAssembly: the same shell as the native app, with bins, meters, the command registry and the engine. It is a whole application, not a panel that can be dropped into a page and stripped for a client.

Embedding it would hand every reviewer the agent tools (raw library, alignment, QC, export) and would still not provide per-client links, the upload form, frame-region comments or an approval mark. Teaching `ui-egui` a reviewer role is a later change to the editor itself.

The portal therefore has its own review timeline. The operations match the editor's command ids where those commands exist (`timeline.trim`, `timeline.razor`, `timeline.move`, `timeline.lift`, `clip.gain`, `caption.edit`). A later build can replay the same events through `Session::execute` and, for the editor role only, host `reelsmith-web`. Reviewers keep this timeline.

The browser page is a client of the Rust service. The rules — the log, the upload form, aspect, storage — live in Rust and are what the tests cover. The page is original; it does not take layout, colour or branding from the earlier review prototype.

## 2. One project, two modes

A project belongs to one client and holds one or more versions. A version is a cut with its own timeline, comments, approval and event log. Versions are named by how they differ ("tighter hook", "slower product beat"), not by A, B or C.

| | Editor (`#/a/…`) | Reviewer (`#/r/…`) |
|---|---|---|
| Timeline | yes | yes |
| Trim, split, lift, move, length | yes | yes |
| Volume, mute, captions | yes | yes |
| Change format | yes | yes |
| Comments, history, undo, restore | yes | yes |
| Approve / request changes | no — that mark is the client's | yes |
| Raw library, word lane, QC flags, decisions | yes | no |
| "I'm editing" / "Ready for review" | yes | no |

While the version status is `agent_working`, the reviewer timeline is read-only and shows "your editor is working". The page polls `GET /api/workspace/{version}/head` every two seconds and redraws when the log or the status changes. A websocket is the later replacement for that poll; the log itself does not change.

Snap to word boundaries is on by default. The toggle is an event (`portal.snap`). Words are stored on the timeline. Nothing in this build transcribes them; the editor mode can show words that were written into the project. Reviewers do not receive the word lane.

## 3. Time

Edit math uses `reelsmith_time::Tick` (`TICKS_PER_SECOND` = 254 016 000 000). The portal stores that integer as `MediaTick` and serializes it as a decimal string, so a JSON number cannot round it.

The browser talks in whole frames at the sequence frame rate (30/1 unless a project says otherwise). Frame `n` is `n * frame_ticks`, and `frame_ticks` must divide the time base exactly. One second at 30 fps is 30 frames and `TICKS_PER_SECOND` ticks.

Volume is not time. Gain is an integer in thousandths (1000 = unity, 4000 = the cap). A frame region is four integers, each 0..=1000, for x, y, width and height.

## 4. Append-only edit log

Each version has a base snapshot and a log of `EditEvent`s. An event records:

- `id`, increasing, never reused
- `at`, an RFC 3339 timestamp from `chrono`
- `actor`: client (id and name), agent, or admin
- `action`, `targets`, `summary`
- `reason`, when the agent is required to give one
- `based_on`, the cursor this step was applied on
- `before` and `after`, full snapshots

The file `history/{project}/{version}.jsonl` is only appended to. The service refuses to rewrite it shorter. `state_of` in a cursor file selects which event's `after` snapshot is current. `None` means the base. Undo, redo and restore append a record and move the cursor. They do not delete the step they reverse.

The chain of steps that are actually in effect walks `based_on`, except a restore jumps to the event it restored (`relates_to`). An undone branch stays in the file and in the history sidebar, marked as not in effect. Restore of any earlier step is itself a step, so it can be undone.

### Agent edits never silently replace a client edit

Before an agent event is committed, the service compares the snapshots and lists every clip, caption or sequence property that changed. For each of those, it walks the current chain and finds the latest picture edit that touched that target. If that actor was the client and the new event has no reason, the edit is rejected and the timeline is unchanged. The reason is stored on the event ("Restored the last word of the CTA. The cut was landing inside the word.").

Comments, replies, status and decisions notes are not picture edits. An agent can resolve a comment without that reason. The agent cannot approve. Approval is the client's good / not-good mark.

## 5. Timeline

A snapshot holds the timeline, the comments, the status, the optional approval, the decisions note and the final type.

Clips have a track (`V1`, `V2`, `A1`, `A2`, `A3`), a kind (video, audio, music, b-roll, title, end card), a start, a duration, a source in-point and a source duration, gain, mute, and an optional linked partner. A trim, move, split or lift applies to the linked partner in the same event, so picture and dialogue stay together. A trim cannot pass the start of the timeline or the end of the source, and it cannot leave a clip with no frames. Split has to land strictly inside the clip.

Captions have text and an anchor measured in the same 0..=1000 vertical space as a comment region. Change format sets the sequence width and height, records `aspect_source = change_format`, and moves caption anchors when the orientation changes (portrait anchors sit slightly higher than landscape ones). That reposition is part of the same event.

## 6. Comments

A comment has an author, a start and end (equal for a single frame), an optional frame region, a body, a resolved flag, an optional "fixed in {version}" label, and replies.

- Dragging a box on the picture creates a frame-region comment at the playhead.
- Mark in, mark out, then "Comment on range" creates a time-range comment with no box.
- Resolve can set `fixed_in` to the version's difference label. The sidebar shows that as a badge.
- The same list is exported for the style pipeline.

## 7. Versions, approval, and the style pipeline

`Approve` writes `mark: good`. `Request changes` writes `mark: not_good`. Both store the note, the actor and the time, and both are events, so a later restore can bring the previous mark back. The current mark is whatever the current snapshot says.

On every edit the service writes two files under the data directory (not the git tree):

`feedback/{project}/{version}/approval.json` — the object [SPEC.md](../style-pipeline/SPEC.md) already calls `approval.json`:

```json
{"version":"tighter hook","mark":"good","note":"Approved","by":"Harbor & Co","at":"2026-11-18T21:00:00Z"}
```

`feedback/{project}/{version}/feedback.json` — schema `reelsmith.portal.feedback/1`. It repeats that approval and adds the comments (body, tick range, region, resolved, fixed_in) and the client picture-edits on the current chain (action, targets, summary, time, and the before/after frame lengths of those targets).

That is Gate B in the style pipeline. `feedback.py --marks` can point at `approval.json`. The richer file is the extra signal: client trims and caption rewrites are strong style evidence, with a before and an after. Good marks reinforce the profile. Not-good marks, with the note, are the negative examples. The portal does not rewrite `style_profile.json` itself. Profiles stay layered the way the spec says: house, then client, then the project. Final type can be changed later; that change is an event, and a project can hold versions of more than one type.

Nothing in the portal delivers a file, calls a network publish step, or creates an ad. The pipeline's rule that a Meta handoff is created paused is unchanged, and this build does not perform that handoff.

## 8. Upload form

One form creates a project.

| Field | Rule |
|---|---|
| Final type | Reel, Ad, or Long-form. Changeable later, per version. |
| Target length | Empty (use the profile default), an exact duration, or a range. A backwards range is rejected. |
| Versions | 1 to 12, default 1. One version may omit the label and is named "Primary reel / ad / cut". Two or more need difference labels. The labels `A`, `B`, `C`, `A_full`, `B_trim` and `C_60` are rejected. |
| Use B-roll | Off by default. If on, the client can tick items already in their library and/or upload new B-roll. An empty selection means the editor may pick. B-roll sent while the box is off is rejected. A file name that appears in both the raw list and the B-roll list is rejected. |
| Raw footage | At least one A-roll file. B-roll cannot be placed in the raw list. |

There is no rule that a cut must be under 60 seconds. A target of 60 seconds is the only thing that asks for 60 seconds. If the raw A-roll is shorter than the target, the plan carries a `raw_short` flag and the project is still created. The editor sees that flag. The server does not refuse the upload.

### Aspect

The output size is chosen in this order:

1. **Change format** — the person checked the box and gave a width and height. The decision is `change_format`. Doing it again on the timeline is a recorded step (section 5).
2. **Profile** — the brief carries a profile width and height. The decision is `profile`.
3. **Raw** — otherwise the aspect is the raw footage. Clips are grouped by reduced ratio (1920×1080 and 1280×720 are both 16:9). The group with the greater total duration wins. A tie keeps the group that appeared first. Clips that do not match are listed on a `mixed_aspect` flag.

The browser reports width, height and duration from the file when it can. If it cannot, the form asks for those numbers and treats them as the footage's own size, which is still the raw aspect, not a format change. A server-side probe of the container (through `reelsmith-isobmff`) is future work. Zero, negative, and sizes above 7680 are rejected.

### Defaults by final type

These are suggestions on the plan. A person can override them, and the override is an edit.

| | Reel | Ad | Long-form |
|---|---|---|---|
| Aspect if nothing else is set | the raw (often 9:16) | the raw | the raw (often 16:9) |
| End card | no | yes, with a CTA | no |
| Captions | on | on | optional |
| Pacing | tight | tight | loose |
| Also | | optional 4:5 feed, thumbnails | optional chapters and a lower third |
| Length cap | none | none | none |

The first cut lays each A-roll file on V1, with a linked dialogue clip on A1, a music bed on A2, and one caption per shot when the type defaults to captions on. Selected B-roll is listed in the editor library and is not cut in until someone places it (`portal.place_broll` puts it on V2). Automatic B-roll coverage belongs to the pipeline's Apply stage.

## 9. Storage

Completed source objects are immutable. The local adapter returns an error on delete and on a second create for the same key. Versions and backups refer to those keys. They never rewrite them.

Uploads are multipart. The client creates an upload (`filename`, `role` = `aroll` or `broll`, `size`), sends parts numbered from 1, and completes. Parts may arrive again (resume) and may be written out of order; completion requires every part from 1 with no gaps, and the byte count must equal the declared size. A part is at most 8 MiB in this build. The total is capped by `PORTAL_MAX_UPLOAD_BYTES` (default 8 GiB). The browser stores the upload id in `localStorage` and skips parts the status call already lists.

Keys are restricted to ASCII letters, digits, `/`, `.`, `_` and `-`, with no `..` and no leading slash. The service only accepts a key under `c/{that client's id}/`.

### Local adapter

`PORTAL_STORAGE=local` (the default) writes parts and finished objects under `{PORTAL_DATA_DIR}/storage`. No credentials. This is what `cargo run -p reelsmith-portal` uses, and what the tests use.

### R2

`PORTAL_STORAGE=r2` reads `R2_ACCOUNT_ID`, `R2_BUCKET`, `R2_ACCESS_KEY_ID` and `R2_SECRET_ACCESS_KEY`. The signer is AWS SigV4 for the S3 API, region `auto`, which is what Cloudflare R2 expects. `presign_put` returns a URL that does not contain the secret. Part size for an R2 plan is 5 MiB, R2's minimum except for the last part.

Creating the multipart upload, accepting ETags, and calling CompleteMultipartUpload need a live bucket. This build signs example part URLs and returns `deferred: true` with an explanation, and it does not pretend the object arrived. The browser shows that message. Wiring those HTTP calls is the next storage step; the local adapter stays the one that can be tested here.

Reviewers never receive raw bytes. `GET /api/objects?key=` is forbidden for a reviewer or client token. The timeline they see is the proxy: coloured shots, captions and the playhead. Generating real proxy media is future work. Until then the editor token is the only one that can read a stored object, and only a key under their client.

## 10. Access

Links are bearer tokens (32 random bytes, hex). They travel on `Authorization: Bearer`, and in the fragment of the page URL (`#/r/…`), which the browser does not send to the server. The server does not log the header.

| Link | Who | Can |
|---|---|---|
| `#/admin/{token}` | the operator | every client, every project, recent history, create a client |
| `#/c/{token}` | one client | their upload form, their projects, their B-roll library |
| `#/r/{token}` | a reviewer of one project | the stripped timeline |
| `#/a/{token}` | the editor of one project | the full timeline |

A client token cannot open another client's project. The admin token is `PORTAL_ADMIN_TOKEN` if that variable is set, otherwise a token generated on first run and stored in the data directory. The process prints the links once at startup so a local operator can open them. `PORTAL_QUIET=1` suppresses that. The data directory is the secret store. It is gitignored (`.portal-data/`).

## 11. HTTP

All JSON errors are `{"message": "..."}` with a 400, 403, 404, 409, 423 or 501. 423 is the read-only reviewer lock.

| Method | Path | Purpose |
|---|---|---|
| GET | `/` | the page |
| GET | `/api/health` | liveness |
| GET | `/api/admin/overview` | admin dashboard |
| POST | `/api/admin/clients` | `{name}` |
| GET | `/api/home` | this client's projects and B-roll |
| POST | `/api/briefs/check` | validate a brief, or 400 with field errors |
| POST | `/api/projects` | create from a valid brief |
| GET | `/api/workspace?version=` | the timeline page for this link |
| GET | `/api/workspace/{version}/head` | `{event_count, state_of, status}` for the poll |
| POST | `/api/workspace/{version}/edits` | `{reason, edit}` |
| GET | `/api/workspace/compare/{left}/{right}` | clip and caption differences |
| GET | `/api/feedback` | the pipeline export |
| POST | `/api/uploads` | start a multipart upload |
| PUT | `/api/uploads/{id}/{part}` | write one part |
| GET | `/api/uploads/{id}` | parts received so far |
| POST | `/api/uploads/{id}/complete` | finish; B-roll joins the library |
| GET | `/api/objects?key=` | editor or admin, raw bytes |

An edit body looks like `{"reason": null, "edit": {"op": "trim", "clip_id": "v1", "edge": "out", "frames": -12}}`. Ops include `trim`, `length`, `razor`, `move`, `lift`, `gain`, `caption`, `caption_move`, `snap`, `change_format`, `comment`, `resolve`, `reply`, `approve`, `request_changes`, `decisions`, `final_type`, `status`, `place_broll`, `undo`, `redo`, `restore`.

## 12. What this build does not do yet

- Websockets. Reviewers poll.
- Replaying events into `reelsmith-engine` or embedding `reelsmith-web` for the editor role.
- Transcription, word alignment from `reelsmith-speech`, or loudness from `reelsmith-audio-dsp`. QC flags are computed from the timeline and the brief. The editor rail says loudness is not measured.
- Proxy transcodes. Reviewers see the timeline, not picture from the raw file.
- Reading width and height out of the container on the server.
- The live R2 multipart calls (CreateMultipartUpload, upload by ETag, CompleteMultipartUpload). Signing is implemented and tested against the published SigV4 example.
- Accounts, email, or a password. The link is the credential.
- Any publish step.

## 13. Running it

See [../../apps/reelsmith-portal/README.md](../../apps/reelsmith-portal/README.md). Configuration is environment variables only. No secret is compiled in, and the data directory stays out of git.
