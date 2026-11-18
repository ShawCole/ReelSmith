# reelsmith-portal

The review portal: a client uploads raw footage, an editor cuts it, and the client reviews and edits the timeline. The design is [docs/portal/DESIGN.md](../../docs/portal/DESIGN.md).

## Run it locally

From the repository root, with the workspace's Rust toolchain:

```bash
cargo run -p reelsmith-portal
```

The process listens on `127.0.0.1:8787` and prints the admin, client, reviewer and editor links. Those links are the credentials for the data directory it just created (`.portal-data/`, gitignored). Open the reviewer link and the editor link.

The first run seeds one client, Harbor & Co, and a project, Spring workshop reel, with two versions and a recorded history.

## Configuration

Environment variables only. Nothing secret is compiled in.

| Variable | Default | Meaning |
|---|---|---|
| `PORTAL_BIND` | `127.0.0.1:8787` | `host:port` |
| `PORTAL_DATA_DIR` | `.portal-data` | projects, the edit log, and local objects |
| `PORTAL_ADMIN_TOKEN` | generated on first run | admin dashboard token |
| `PORTAL_STORAGE` | `local` | `local` or `r2` |
| `PORTAL_MAX_UPLOAD_BYTES` | 8 GiB | cap on one declared upload |
| `PORTAL_QUIET` | unset | `1` to skip printing links |
| `R2_ACCOUNT_ID` | | required when storage is `r2` |
| `R2_BUCKET` | | required when storage is `r2` |
| `R2_ACCESS_KEY_ID` | | required when storage is `r2` |
| `R2_SECRET_ACCESS_KEY` | | required when storage is `r2` |

`local` stores multipart uploads on disk and needs no credentials. `r2` signs S3-compatible URLs and does not finish the multipart upload against the bucket in this build; the server says so. Use `local` to exercise uploads.

## Tests

```bash
cargo test -p reelsmith-portal
```

The tests cover the edit log (undo, redo, restore, the agent's reason), the upload form (versions, B-roll, aspect, no implicit 60-second cap), the local upload adapter, and SigV4 signing.

## What works, and what does not

Working on a local run: both timeline modes, trim / split / move / length / volume / captions, frame-region and time-range comments, the full history with undo and restore, versions, approve and request changes, the upload form, and resumable uploads to the local disk.

Not in this build: websocket live sync (the page polls), embedding the wasm editor, real proxy video, server-side measurement of the file, transcription, loudness meters, and the live R2 complete-upload calls. The design doc lists these in the same place.
