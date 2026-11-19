# ReelSmith Review Portal

Clients upload raw footage. The editor (a person or the editing agent) cuts it. The client opens
the cut **in the ReelSmith editor itself**, edits it directly on the timeline, comments on it, and
marks it good or not good. Every one of those steps is kept, with who took it and when, and fed to
the style pipeline ([../style-pipeline/SPEC.md](../style-pipeline/SPEC.md)).

The service is `apps/reelsmith-portal`; how to run it is in its
[README](../../apps/reelsmith-portal/README.md). Nothing here publishes a video.

## 1. One editor, one timeline

The review page is `apps/reelsmith-web`: the full editor compiled to WebAssembly, with the same
engine and the same egui UI as the desktop app. There is no second, look-alike timeline.

Each version of a cut is an engine project (`.fcproj`). The engine's command bus,
`Session::execute(id, params)`, is the one path every edit takes (menus, keys, mouse, the
`window.reelsmith` agent API). Two hooks on it make the portal possible
(`crates/engine/src/review.rs`):

- a **policy**, asked before every top-level command, that refuses what the link's role may not do;
- an **outbox** of every top-level command that succeeded: id, params, label, and whether it
  changed the project.

Only top-level commands are gated and logged. A command that runs others (`timeline.delete`
runs `edit.clear`) is allowed and logged once, as itself.

## 2. Links and roles

A link is a bearer token in the URL fragment (`/review/#<token>`), sent as
`Authorization: Bearer`, so it never reaches a server log.

| Link | Role | What the page is |
|---|---|---|
| `/edit/#…` | editor | The full editor, plus the Comments and Edit History panels. The editing agent drives it through `window.reelsmith.execute`. |
| `/review/#…` | reviewer | The same editor in the **Client Review** workspace: Program, Timeline, Tools, meters, Comments and Edit History. No menus, project library, Media Browser, effects, agent tools or other workspaces. Approve / Request changes in the header. |
| `/watch/#…` | watcher | The reviewer's layout, read-only, following the editor live. |
| `/#/c/…` | client | Their projects, their B-roll library, and the upload form. |
| `/#/admin/…` | admin | Every client and project. |

While the version's status is `agent_working`, a reviewer link runs as a watcher ("Your editor is
working") and the server answers its edits with 423.

**What a reviewer can do on the timeline:** select, trim (regular and ripple, trim mode), razor,
move, nudge, delete / ripple delete, lift / extract, add edit, close gaps, clip gain and volume,
enable/disable a clip, mute and solo targeted audio, caption text / timing / split / merge,
markers and In/Out, undo and redo. Playback and navigation are always allowed. Everything else
that changes the project is the editor's (`REVIEWER_EDITS` in `review.rs`).

**Never in a portal session** (any role): file open/save/import/export, render, proxies,
transcription, scene detection, media relink, presets and shortcuts. They read or write the disk
or analyse media, so they cannot replay from a log. A test fails when a new journaled command is
added without being classified.

A locked link also cannot open other panels (`show_panel`), switch workspace or mode, change UI
settings over the control channel, or read files through `window.reelsmith` (`readFile`,
`importFiles`, `importUrl` and `openProject` are not installed on portal links).

## 3. The server is the single writer

```
page (wasm editor)                         portal
  execute(id, params) ──outbox──► POST /api/v/{v}/actions {based_on_seq, actions}
                                     │ role check (review::allows)
                                     │ replay in order on its own headless Session
                                     │ append actions.jsonl, write head.fcproj
  ◄── 200 {seq} | 403 refused | 409 stale or failed | 423 editor at work
```

Per version, in the data directory (never in the repository):

| File | |
|---|---|
| `base.fcproj` | the cut as first handed to review; never rewritten |
| `head.fcproj` | the cut now, written by the server only |
| `actions.jsonl` | append-only; the store refuses to shorten it |
| `comments.json` | threads |
| `feedback/approval.json`, `feedback/feedback.json` | for the style pipeline (§6) |

An action line records `seq`, server time `at`, the **actor from the token** (the page cannot
claim to be someone else), the role, the command id and params, its label, whether it changed the
project, the `based_on_seq` it was made on, and for undo/redo the step it reverts.

Pages never upload a project file. A batch made on an old `seq`, or one that fails to replay, is
refused whole and leaves nothing behind; the page then re-opens the server's head. That is also
what happens when two people edit at once.

Replay is exact because new ids come from the project's own counter, and commands that act on the
selection or playhead have the selection and playhead steps in the log before them. A test replays
a reviewer's trim, gain, razor and undo on a fresh session and compares the projects.

## 4. Live watching

Every page polls `GET /api/v/{v}/head` (`{seq, status}`) once a second. When the log has moved on
and one of the new steps changed the cut, the page re-opens `head.fcproj` and keeps its playhead.
Comments, marks and status changes only refresh the panels. A push channel can replace the poll
without changing the log.

## 5. Comments and edit history

A comment has an author, a start and end time (equal for one frame; the In/Out range when both are
set), an optional frame region, a body, replies, and resolved / "fixed in". The time chip moves the
playhead there.

Edit History lists every step that changed the cut, plus comments, marks and status changes,
newest first: "Trim Edit · Harbor & Co (client) · Nov 18, 2026, 8:27 PM". Selection and playhead
steps stay in the log for replay but out of the panel.

## 6. Feedback into the style pipeline

Approve writes `mark: good`, Request changes `mark: not_good`, as `feedback/approval.json` in the
exact shape SPEC.md §5.1 reads:

```json
{"version":"Primary reel","mark":"good","note":"Approved","by":"Harbor & Co","at":"2026-11-19T01:33:43Z"}
```

`feedback/feedback.json` (schema `reelsmith.portal.feedback/2`) adds the comments and every
**client** step in effect, with the touched clips' durations before and after. Client trims and
caption rewrites are strong style evidence (SPEC §8, Stage 4). The portal never edits
`style_profile.json`, never publishes, and never creates an ad (Meta stays paused).

## 7. Upload form

| Field | Rule |
|---|---|
| Final type | Reel, Ad, or Long-form. Changeable later, per version. |
| End card | Optional (suggested on for Ad). |
| Target final length | Empty (profile default), an exact duration, or a range. No under-60-second rule. |
| How many versions? | A number, 1 to 12. Two or more need difference labels; `A`, `B`, `C`, `A_full`, `B_trim`, `C_60` are rejected. |
| Use B-roll? | Off by default. When ticked, the client's existing B-roll gallery appears (tick items) plus a separate B-roll upload area. |
| Raw footage | At least one A-roll file. |

**Aspect** is the raw footage's (the majority aspect by duration) unless the client/style profile
sets one or the client presses **Change format**. Most cuts will be 9:16.

**Uploads are resumable and multipart.** Parts may arrive again or out of order; completion needs
every part with no gaps and the declared size. The browser keeps the upload id and skips parts the
server already has. Completed sources are immutable: the store refuses to delete or replace them.

## 8. Storage

`PORTAL_STORAGE=local` (default) stores under the data directory and needs no credentials.
`PORTAL_STORAGE=r2` reads `R2_ACCOUNT_ID`, `R2_BUCKET`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`
from the environment; SigV4 signing for Cloudflare R2 is implemented and tested.

## 9. Not done yet

- Live R2 multipart calls (create, upload by ETag, complete). Local storage is the tested path.
- Review proxies for uploaded media, and streaming large raws by HTTP Range instead of a whole
  download (the seed project uses generated media, so it needs neither).
- A frame-region box drawn on the Program monitor for comments, and comment pins on the timeline
  ruler.
- The rule that an agent edit over a client's edit must carry a reason (the `reason` field is
  stored, not yet required).
- A push channel instead of the one-second poll.
