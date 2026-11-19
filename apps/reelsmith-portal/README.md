# reelsmith-portal

The review portal. A client uploads raw footage; the editor cuts it; the client reviews and edits the cut in the real ReelSmith editor (the wasm build of `apps/reelsmith-web`), not a look-alike. The design, the roles and the HTTP routes are in [`docs/portal/DESIGN.md`](../../docs/portal/DESIGN.md).

## How it works

- Each version of a cut is an engine project: `base.fcproj` (as first handed to review), `head.fcproj` (now), and `actions.jsonl`, an append-only log of every step with who took it and when.
- The page runs the editor and posts each top-level engine command it executed (`id`, `params`) to `POST /api/v/{version}/actions`. The server checks the link's role against `reelsmith_engine::review::allows`, replays the batch in order on its own headless engine session, appends it to the log and writes `head.fcproj`. A refused or failing batch leaves nothing behind (403 / 409). Pages never upload a project file.
- Comments, replies, resolves, marks and status changes are portal steps in the same log (`portal.*`), never replayed on the engine.
- Approve / Request changes writes `feedback/approval.json` (style-pipeline SPEC §5.1) and `feedback/feedback.json` (`reelsmith.portal.feedback/2`: approval, comments, and the client's steps in effect with the touched clips' durations before and after).

## Run it locally

Build the editor once (`cargo xtask web`, it writes `target/web/dist`), then from the repository root:

```bash
cargo run -p reelsmith-portal
```

The process listens on `127.0.0.1:8787` and prints the admin, client, editor, review and watch links. Those links are the credentials for the data directory it just created (`.portal-data/`, gitignored). Tokens sit in the URL fragment (`/review/#<token>`) and travel as `Authorization: Bearer`, so they never reach a server log.

The first run seeds one client, Harbor & Co, and a project, Spring launch reel, whose version starts from the engine's demo project.

## Configuration

Environment variables only. Nothing secret is compiled in.

| Variable | Default | Meaning |
|---|---|---|
| `PORTAL_BIND` | `127.0.0.1:8787` | `host:port` |
| `PORTAL_DATA_DIR` | `.portal-data` | projects, action logs, local objects |
| `PORTAL_EDITOR_DIST` | `<repo>/target/web/dist` | the wasm editor served under `/edit/`, `/review/`, `/watch/` |
| `PORTAL_ADMIN_TOKEN` | generated on first run | admin token |
| `PORTAL_STORAGE` | `local` | `local` or `r2` |
| `PORTAL_MAX_UPLOAD_BYTES` | 8 GiB | cap on one declared upload |
| `PORTAL_QUIET` | unset | `1` to skip printing links |
| `R2_ACCOUNT_ID`, `R2_BUCKET`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` | | required when storage is `r2` |

`local` stores multipart uploads on disk and needs no credentials. `r2` signs S3-compatible URLs and does not finish the multipart upload against the bucket in this build; the server says so.

## Tests

```bash
cargo test -p reelsmith-portal
```

They cover link roles at boot (and a link for one project refused on another), the role gate on posted commands, stale and failing batches rolling back, the log refusing to be shortened, base + log replaying to head, cross-origin isolation headers and `application/wasm`, the SPEC §5.1 approval file, the read-only lock while the editor works, the upload form rules, the local upload adapter and SigV4 signing.

## Not in this build

Media proxies and the upload-to-first-cut step (a new project's versions start empty), comment markers on the editor's ruler, live push to watchers (they poll `/head`), and the live R2 complete-upload calls.
