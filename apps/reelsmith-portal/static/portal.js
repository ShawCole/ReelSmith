const TICKS_PER_MS = 254016000n;
const app = document.querySelector("#app");
let token = "";
let kind = "";
let page = null;
let playhead = 0;
let selectedId = null;
let playing = false;
let tab = "comments";
let draft = null;
let statusTimer = 0;

function el(tag, attrs, text) {
  const node = document.createElement(tag);
  if (attrs) {
    for (const [key, value] of Object.entries(attrs)) {
      if (value == null) continue;
      if (key === "class") node.className = value;
      else node.setAttribute(key, String(value));
    }
  }
  if (text != null) node.textContent = text;
  return node;
}

function clear(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
}

function route() {
  const parts = location.hash.replace(/^#\/?/, "").split("/").filter(Boolean);
  return { kind: parts[0] || "", token: decodeURIComponent(parts[1] || "") };
}

async function api(path, opts) {
  const options = opts || {};
  const headers = { Authorization: "Bearer " + token };
  let body;
  if (options.json != null) {
    headers["Content-Type"] = "application/json";
    body = JSON.stringify(options.json);
  } else if (options.raw != null) {
    body = options.raw;
  }
  const response = await fetch(path, { method: options.method || "GET", headers, body });
  const text = await response.text();
  let data = {};
  if (text) {
    try { data = JSON.parse(text); } catch { data = { message: text }; }
  }
  if (!response.ok) {
    const extra = Array.isArray(data.errors) ? data.errors.map((item) => item.message).join(" ") : "";
    throw new Error(data.message || extra || ("Request failed (" + response.status + ")"));
  }
  return data;
}

function ticksFromSeconds(seconds) {
  const ms = BigInt(Math.round(Number(seconds) * 1000));
  return (ms * TICKS_PER_MS).toString();
}

function tc(frame, fps) {
  const safe = Math.max(0, frame | 0);
  const rate = fps || 30;
  const ff = safe % rate;
  const total = Math.floor(safe / rate);
  const ss = total % 60;
  const mm = Math.floor(total / 60);
  const pad = (value) => String(value).padStart(2, "0");
  return pad(mm) + ":" + pad(ss) + ":" + pad(ff);
}

function boot() {
  const next = route();
  kind = next.kind;
  token = next.token;
  if (!kind || !token) {
    landing();
    return;
  }
  if (kind === "admin") loadAdmin();
  else if (kind === "c") loadHome();
  else if (kind === "r" || kind === "a") loadWorkspace();
  else landing();
}

function landing() {
  clear(app);
  const box = el("div", { class: "shell plain" });
  box.append(el("h1", { class: "mark" }, "ReelSmith Review"));
  box.append(el("p", null, "Open the reviewer, editor, or client link printed when the portal starts. The link is the key."));
  app.append(box);
}

function showError(error) {
  const line = document.querySelector("#status");
  if (line) line.textContent = error.message || String(error);
  else landing();
}

async function loadAdmin() {
  try {
    const data = await api("/api/admin/overview");
    renderAdmin(data);
  } catch (error) { showError(error); }
}

function renderAdmin(data) {
  clear(app);
  const box = el("div", { class: "shell plain" });
  box.append(el("h1", { class: "mark" }, "Clients"));
  box.append(el("p", { class: "note" }, "Each client has their own upload link. Editor links are the full timeline. Reviewer links are the stripped timeline."));
  const form = el("form");
  const name = el("input", { placeholder: "Client name", required: "true" });
  const button = el("button", { class: "primary", type: "submit" }, "Add client");
  form.append(name, button);
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    try {
      await api("/api/admin/clients", { method: "POST", json: { name: name.value } });
      loadAdmin();
    } catch (error) { showError(error); }
  });
  box.append(form);
  const table = el("table");
  table.append(el("tr"), );
  const head = el("tr");
  ["Client", "Upload", "Projects"].forEach((label) => head.append(el("th", null, label)));
  table.append(head);
  for (const client of data.clients) {
    const row = el("tr");
    row.append(el("td", null, client.name));
    const upload = el("td");
    const link = el("a", { href: "#/c/" + client.token }, "Open upload form");
    upload.append(link);
    row.append(upload);
    const projects = data.projects.filter((project) => project.client_name === client.name);
    const cell = el("td");
    if (!projects.length) cell.textContent = "No projects yet";
    for (const project of projects) {
      const block = el("div");
      block.append(el("strong", null, project.name));
      block.append(el("div", { class: "note" }, project.final_type + " · " + project.versions.join(", ")));
      block.append(el("a", { href: "#/r/" + project.reviewer_token }, "Reviewer"));
      block.append(document.createTextNode(" · "));
      block.append(el("a", { href: "#/a/" + project.agent_token }, "Editor"));
      cell.append(block);
    }
    row.append(cell);
    table.append(row);
  }
  box.append(table);
  const recent = el("section");
  recent.append(el("h2", null, "Recent history"));
  for (const event of data.recent) {
    const card = el("div", { class: "card" });
    card.append(el("div", { class: "meta" }, event.actor + " · " + event.action));
    card.append(el("div", null, event.summary));
    if (event.reason) card.append(el("div", { class: "note" }, event.reason));
    recent.append(card);
  }
  box.append(recent);
  const status = el("p", { id: "status", class: "error" });
  box.append(status);
  app.append(box);
}

async function loadHome() {
  try {
    const data = await api("/api/home");
    renderHome(data);
  } catch (error) { showError(error); }
}

function renderHome(data) {
  clear(app);
  const box = el("div", { class: "shell plain" });
  box.append(el("h1", { class: "mark" }, data.client_name));
  box.append(el("p", { class: "note" }, "Raw footage and B-roll stay in separate uploads. Output matches the raw aspect unless a profile or Change format says otherwise."));
  if (data.projects.length) {
    const list = el("div");
    for (const project of data.projects) {
      const card = el("div", { class: "card" });
      card.append(el("strong", null, project.name));
      card.append(el("div", { class: "note" }, project.final_type + " · " + project.versions.join(", ")));
      card.append(el("a", { href: "#/r/" + project.reviewer_token }, "Open reviewer timeline"));
      list.append(card);
    }
    box.append(list);
  }
  box.append(uploadForm(data));
  box.append(el("p", { id: "status", class: "error" }));
  app.append(box);
}

function uploadForm(data) {
  const form = el("form");
  const name = field("Project name", el("input", { required: "true", value: "New cut" }));
  const type = el("select");
  [["reel", "Reel"], ["ad", "Ad"], ["long_form", "Long-form"]].forEach(([value, label]) => {
    const option = el("option", { value }, label);
    type.append(option);
  });
  const targetKind = el("select");
  [["unspecified", "Profile default"], ["exact", "Exact seconds"], ["range", "Range"]].forEach(([value, label]) => targetKind.append(el("option", { value }, label)));
  const exact = el("input", { type: "number", min: "1", step: "1", placeholder: "seconds" });
  const min = el("input", { type: "number", min: "1", step: "1", placeholder: "min seconds" });
  const max = el("input", { type: "number", min: "1", step: "1", placeholder: "max seconds" });
  const count = el("input", { type: "number", min: "1", max: "12", value: "1" });
  const diffs = el("div");
  const brollOn = el("input", { type: "checkbox" });
  const gallery = el("div", { class: "hidden" });
  const checks = [];
  for (const item of data.broll) {
    const row = el("label", { class: "row" });
    const box = el("input", { type: "checkbox", value: item.id });
    checks.push(box);
    row.append(box, el("span", null, item.name + " · " + item.width + "×" + item.height));
    gallery.append(row);
  }
  if (!data.broll.length) gallery.append(el("p", { class: "note" }, "No B-roll in the library yet. Upload some below, or leave the gallery empty and the editor will pick."));
  const brollFiles = el("input", { type: "file", multiple: "true" });
  const rawFiles = el("input", { type: "file", multiple: "true", required: "true" });
  const change = el("input", { type: "checkbox" });
  const width = el("input", { type: "number", min: "1", placeholder: "width" });
  const height = el("input", { type: "number", min: "1", placeholder: "height" });
  const manual = el("div", { class: "note" }, "If a file has no video metadata, type its width, height and seconds. That is still the raw's aspect, not a format change.");
  const fallbackW = el("input", { type: "number", min: "1", placeholder: "footage width" });
  const fallbackH = el("input", { type: "number", min: "1", placeholder: "footage height" });
  const fallbackS = el("input", { type: "number", min: "1", step: "0.1", placeholder: "seconds" });
  function refreshDiffs() {
    clear(diffs);
    const n = Math.max(1, Math.min(12, Number(count.value) || 1));
    for (let i = 0; i < n; i++) {
      const input = el("input", { placeholder: n === 1 ? "Optional difference" : "Difference (hook, pacing, B-roll)" });
      diffs.append(field(n === 1 ? "Version label" : "Version " + (i + 1), input));
    }
  }
  count.addEventListener("input", refreshDiffs);
  refreshDiffs();
  brollOn.addEventListener("change", () => gallery.classList.toggle("hidden", !brollOn.checked));
  form.append(
    name,
    field("Final type", type),
    field("Target length", targetKind),
    field("Exact", exact),
    el("div", { class: "grid" }, ),
  );
  const range = el("div", { class: "grid" });
  range.append(field("Minimum", min), field("Maximum", max));
  form.append(range, field("How many versions", count), diffs);
  const brollRow = el("label", { class: "row" });
  brollRow.append(brollOn, el("span", null, "Use B-roll?"));
  form.append(brollRow, gallery, field("B-roll files (separate from the raw footage)", brollFiles));
  form.append(field("Raw footage", rawFiles), manual, field("Width", fallbackW), field("Height", fallbackH), field("Duration", fallbackS));
  const changeRow = el("label", { class: "row" });
  changeRow.append(change, el("span", null, "Change format"));
  form.append(changeRow, field("Output width", width), field("Output height", height));
  const submit = el("button", { class: "primary", type: "submit" }, "Upload and create project");
  form.append(submit);
  form.addEventListener("submit", (event) => submitProject(event, {
    data, name: name.querySelector("input"), type, targetKind, exact, min, max, count, diffs, brollOn, checks, brollFiles, rawFiles, change, width, height, fallbackW, fallbackH, fallbackS, submit,
  }));
  return form;
}

function field(label, control) {
  const wrap = el("label", { class: "field" });
  wrap.append(el("span", null, label), control);
  return wrap;
}

async function submitProject(event, ui) {
  event.preventDefault();
  ui.submit.disabled = true;
  const status = document.querySelector("#status");
  try {
    status.textContent = "Uploading raw footage…";
    const raws = await describeFiles(ui.rawFiles.files, ui.fallbackW, ui.fallbackH, ui.fallbackS);
    for (const file of raws) {
      await uploadChunks(file.file, "aroll", file);
      status.textContent = "Stored " + file.file.name;
    }
    const newBroll = [];
    if (ui.brollOn.checked && ui.brollFiles.files.length) {
      status.textContent = "Uploading B-roll…";
      const described = await describeFiles(ui.brollFiles.files, ui.fallbackW, ui.fallbackH, ui.fallbackS);
      for (const file of described) {
        await uploadChunks(file.file, "broll", file);
        newBroll.push(file.file.name);
      }
    }
    const home = await api("/api/home");
    const picked = ui.checks.filter((box) => box.checked).map((box) => box.value);
    for (const item of home.broll) {
      if (newBroll.includes(item.name) && !picked.includes(item.id)) picked.push(item.id);
    }
    const differences = [...ui.diffs.querySelectorAll("input")].map((input) => input.value.trim());
    const brief = {
      project_name: ui.name.value,
      final_type: ui.type.value,
      version_count: Number(ui.count.value) || 1,
      differences,
      use_broll: ui.brollOn.checked,
      broll_library_ids: ui.brollOn.checked ? picked : [],
      broll_uploads: [],
      raw_files: raws.map((file) => ({
        name: file.file.name,
        width: file.width,
        height: file.height,
        duration: file.duration,
        role: "aroll",
      })),
      change_format: ui.change.checked,
      explicit_width: ui.change.checked ? Number(ui.width.value) : null,
      explicit_height: ui.change.checked ? Number(ui.height.value) : null,
    };
    if (ui.targetKind.value === "exact") brief.target = { kind: "exact", ticks: ticksFromSeconds(ui.exact.value) };
    else if (ui.targetKind.value === "range") brief.target = { kind: "range", min: ticksFromSeconds(ui.min.value), max: ticksFromSeconds(ui.max.value) };
    else brief.target = { kind: "unspecified" };
    status.textContent = "Creating the project…";
    const project = await api("/api/projects", { method: "POST", json: brief });
    status.textContent = "";
    const done = el("div", { class: "card" });
    done.append(el("strong", null, "Project ready"));
    done.append(el("div"));
    done.append(el("a", { href: "#/r/" + project.reviewer_token }, "Open the reviewer timeline"));
    ui.submit.parentElement.append(done);
  } catch (error) {
    status.textContent = error.message;
  } finally {
    ui.submit.disabled = false;
  }
}

async function describeFiles(fileList, fallbackW, fallbackH, fallbackS) {
  const out = [];
  for (const file of fileList) {
    const probed = await probe(file);
    const width = probed && probed.width ? probed.width : Number(fallbackW.value);
    const height = probed && probed.height ? probed.height : Number(fallbackH.value);
    const duration = probed && probed.ms ? (BigInt(probed.ms) * TICKS_PER_MS).toString() : ticksFromSeconds(fallbackS.value || "0");
    if (!width || !height || duration === "0") throw new Error(file.name + " needs a width, height and duration");
    out.push({ file, width, height, duration });
  }
  return out;
}

function probe(file) {
  return new Promise((resolve) => {
    if (!file.type || !file.type.startsWith("video/")) { resolve(null); return; }
    const url = URL.createObjectURL(file);
    const video = document.createElement("video");
    video.preload = "metadata";
    video.onloadedmetadata = () => {
      const result = { width: video.videoWidth, height: video.videoHeight, ms: Math.max(1, Math.round((video.duration || 0) * 1000)) };
      URL.revokeObjectURL(url);
      resolve(result);
    };
    video.onerror = () => { URL.revokeObjectURL(url); resolve(null); };
    video.src = url;
  });
}

async function uploadChunks(file, role, meta) {
  const storeKey = "portal-upload:" + role + ":" + file.name + ":" + file.size;
  let id = localStorage.getItem(storeKey);
  let have = [];
  if (id) {
    try {
      const status = await api("/api/uploads/" + id);
      if (status.complete) {
        localStorage.removeItem(storeKey);
        return id;
      }
      have = status.parts || [];
    } catch { id = null; }
  }
  if (!id) {
    const started = await api("/api/uploads", { method: "POST", json: { filename: file.name, role, size: file.size } });
    if (started.mode === "r2") throw new Error(started.message);
    id = started.id;
    localStorage.setItem(storeKey, id);
  }
  const chunk = 1024 * 1024;
  const parts = Math.max(1, Math.ceil(file.size / chunk));
  for (let index = 1; index <= parts; index++) {
    if (have.includes(index)) continue;
    const slice = file.slice((index - 1) * chunk, Math.min(file.size, index * chunk));
    const response = await fetch("/api/uploads/" + id + "/" + index, {
      method: "PUT",
      headers: { Authorization: "Bearer " + token },
      body: slice,
    });
    if (!response.ok) {
      const text = await response.text();
      throw new Error(text || "Upload part failed");
    }
  }
  await api("/api/uploads/" + id + "/complete", {
    method: "POST",
    json: { name: file.name.replace(/\.[^.]+$/, ""), width: meta.width, height: meta.height, duration: meta.duration },
  });
  localStorage.removeItem(storeKey);
  return id;
}

async function loadWorkspace(version) {
  try {
    const path = "/api/workspace" + (version ? "?version=" + encodeURIComponent(version) : "");
    page = await api(path);
    if (selectedId == null) {
      const region = page.version.comments.find((comment) => comment.region && !comment.resolved);
      playhead = region ? region.start_frame : 0;
    }
    renderWorkspace();
    schedulePoll();
  } catch (error) { showError(error); }
}

function schedulePoll() {
  window.clearInterval(statusTimer);
  statusTimer = window.setInterval(async () => {
    if (!page || document.hidden) return;
    try {
      const head = await api("/api/workspace/" + page.version.id + "/head");
      if (head.event_count !== page.version.event_count || head.state_of !== page.version.state_of || head.status !== page.version.status) {
        const keep = playhead;
        await loadWorkspace(page.version.id);
        playhead = keep;
        paintPlayhead();
      }
    } catch { /* the next poll retries */ }
  }, 2000);
}

function renderWorkspace() {
  clear(app);
  const version = page.version;
  const agent = page.role === "agent" || page.role === "admin";
  const top = el("header", { class: "top" });
  top.append(el("div", { class: "mark" }, "ReelSmith "), el("div", null, page.project_name));
  top.querySelector(".mark").append(el("span", null, "Review"));
  top.append(el("div", { class: "spacer" }));
  top.append(el("div", { class: "pill" }, agent ? "Editor" : "Reviewer"));
  top.append(el("div", { class: "sub" }, version.width + "×" + version.height + " · " + page.brief.aspect_label + " · " + page.brief.aspect_source));
  app.append(top);
  const shell = el("div", { class: agent ? "shell" : "shell reviewer" });
  if (agent) shell.append(renderRail(version));
  shell.append(renderStage(version, agent));
  shell.append(renderDrawer(version, agent));
  app.append(shell);
  paintPlayhead();
}

function renderRail(version) {
  const rail = el("aside", { class: "rail" });
  rail.append(el("h2", null, "Library"));
  rail.append(el("p", { class: "note" }, "Raw files stay in storage. This list is the editor's view."));
  if (!version.library.length) rail.append(el("p", { class: "note" }, "No library items on this cut."));
  for (const item of version.library) {
    const card = el("div", { class: "card" });
    card.append(el("strong", null, item.name));
    card.append(el("div", { class: "meta" }, item.role + (item.selected ? " · selected" : "") + " · " + item.width + "×" + item.height));
    if (item.role === "broll" && !page.read_only) {
      const place = el("button", { type: "button" }, "Place on V2");
      place.addEventListener("click", () => send({ op: "place_broll", item_id: item.id }));
      card.append(place);
    }
    rail.append(card);
  }
  rail.append(el("h2", null, "Checks"));
  rail.append(el("p", { class: "note" }, version.meters_note));
  if (!version.qc.length) rail.append(el("p", { class: "note" }, "No flags."));
  for (const flag of version.qc) rail.append(el("div", { class: "qc" }, flag.message));
  rail.append(el("h2", null, "Decisions"));
  const notes = el("textarea");
  notes.value = version.decisions || "";
  const save = el("button", { type: "button" }, "Save decisions");
  save.addEventListener("click", () => send({ op: "decisions", text: notes.value }));
  rail.append(notes, save);
  const working = el("button", { type: "button" }, "I'm editing");
  working.addEventListener("click", () => send({ op: "status", status: "agent_working" }));
  const ready = el("button", { type: "button", class: "primary" }, "Ready for review");
  ready.addEventListener("click", () => send({ op: "status", status: "ready" }));
  rail.append(el("div", { class: "row" }, ), working, ready);
  rail.append(el("p", { class: "note" }, "Live updates use a short poll. A websocket, and the full engine timeline, are not in this build."));
  return rail;
}

function renderStage(version, agent) {
  const stage = el("main", { class: "stage" });
  if (page.read_only) stage.append(el("div", { class: "banner" }, page.read_only_reason || "Your editor is working."));
  const versions = el("div", { class: "versions" });
  for (const item of page.versions) {
    const button = el("button", { type: "button", class: item.id === version.id ? "primary" : "" }, item.difference);
    button.addEventListener("click", () => loadWorkspace(item.id));
    versions.append(button);
    if (item.mark) versions.append(el("span", { class: "badge" }, item.mark));
  }
  stage.append(versions);
  const actions = el("div", { class: "actions" });
  const note = el("input", { placeholder: "Note for the approval" });
  const request = el("button", { type: "button", class: "danger" }, "Request changes");
  const approve = el("button", { type: "button", class: "good" }, "Approve");
  request.addEventListener("click", () => send({ op: "request_changes", note: note.value }));
  approve.addEventListener("click", () => send({ op: "approve", note: note.value }));
  if (!agent) actions.append(note, el("span", { class: "spacer" }), request, approve);
  else actions.append(el("span", { class: "note" }, "Approval stays with the client. Use Ready for review when the cut can be marked."));
  stage.append(actions);
  const ratio = version.width / version.height;
  const monitor = el("div", { class: "monitor" });
  monitor.style.aspectRatio = String(ratio);
  monitor.style.width = ratio < 1 ? "280px" : "min(100%, 640px)";
  const picture = el("div", { class: "picture" });
  picture.append(el("strong", { id: "clip-title" }, ""));
  const caption = el("div", { class: "caption-overlay", id: "caption-overlay" });
  monitor.append(picture, caption);
  monitor.addEventListener("pointerdown", (event) => startRegion(event, monitor));
  stage.append(el("div", { class: "monitor-wrap" }, ), );
  stage.lastChild.append(monitor);
  const transport = el("div", { class: "transport" });
  const play = el("button", { type: "button", id: "play" }, "Play");
  play.addEventListener("click", togglePlay);
  const split = el("button", { type: "button" }, "Split");
  split.addEventListener("click", () => { if (selectedId) send({ op: "razor", clip_id: selectedId, frame: playhead }); });
  const lift = el("button", { type: "button" }, "Lift");
  lift.addEventListener("click", () => { if (selectedId) send({ op: "lift", clip_id: selectedId }); });
  const snap = el("button", { type: "button" }, version.snap_words ? "Snap words: on" : "Snap words: off");
  snap.addEventListener("click", () => send({ op: "snap", enabled: !version.snap_words }));
  transport.append(play, el("span", { class: "time", id: "tc" }, tc(playhead, version.fps_num)), split, lift, snap);
  const markIn = el("button", { type: "button" }, "Mark in");
  const markOut = el("button", { type: "button" }, "Mark out");
  const range = el("button", { type: "button" }, "Comment on range");
  markIn.addEventListener("click", () => { monitor.dataset.in = String(playhead); });
  markOut.addEventListener("click", () => { monitor.dataset.out = String(playhead); });
  range.addEventListener("click", () => commentRange(monitor));
  transport.append(markIn, markOut, range);
  stage.append(transport);
  stage.append(renderTimeline(version, agent));
  stage.append(renderInspector(version));
  stage.append(el("p", { id: "status", class: "error" }));
  return stage;
}

function renderTimeline(version, agent) {
  const wrap = el("div", { id: "timeline" });
  wrap.append(track("V1", version, "video"));
  wrap.append(track("V2", version, "video"));
  if (agent && version.words.length) wrap.append(wordTrack(version));
  wrap.append(track("A1", version, "audio"));
  wrap.append(track("A2", version, "audio"));
  wrap.append(captionTrack(version));
  return wrap;
}

function durationFrames(version) {
  return Math.max(version.duration_frames, 1, ...version.clips.map((clip) => clip.start_frame + clip.duration_frames), ...version.captions.map((caption) => caption.start_frame + caption.duration_frames));
}

function track(name, version, mode) {
  const lane = el("div", { class: "lane", "data-track": name });
  const total = durationFrames(version);
  for (const clip of version.clips.filter((clip) => clip.track === name)) {
    const node = el("div", { class: "clip " + clip.kind + (clip.id === selectedId ? " selected" : "") });
    node.style.left = (100 * clip.start_frame / total) + "%";
    node.style.width = (100 * clip.duration_frames / total) + "%";
    node.append(el("span", null, clip.name));
    const left = el("div", { class: "handle left" });
    const right = el("div", { class: "handle right" });
    left.addEventListener("pointerdown", (event) => dragClip(event, clip, "in", total));
    right.addEventListener("pointerdown", (event) => dragClip(event, clip, "out", total));
    node.addEventListener("pointerdown", (event) => {
      if (event.target === left || event.target === right) return;
      selectedId = clip.id;
      dragClip(event, clip, "move", total);
    });
    node.append(left, right);
    lane.append(node);
  }
  lane.append(el("div", { class: "playhead", "data-head": "1" }));
  lane.addEventListener("pointerdown", (event) => {
    if (event.target !== lane) return;
    const rect = lane.getBoundingClientRect();
    playhead = Math.round(((event.clientX - rect.left) / rect.width) * total);
    paintPlayhead();
  });
  return lane;
}

function wordTrack(version) {
  const lane = el("div", { class: "lane words" });
  const total = durationFrames(version);
  for (const word of version.words) {
    const node = el("div", { class: "word" }, word.text);
    node.style.left = (100 * word.start_frame / total) + "%";
    node.style.width = Math.max(1, 100 * (word.end_frame - word.start_frame) / total) + "%";
    lane.append(node);
  }
  lane.append(el("div", { class: "playhead", "data-head": "1" }));
  return lane;
}

function captionTrack(version) {
  const lane = el("div", { class: "lane" });
  const total = durationFrames(version);
  for (const caption of version.captions) {
    const node = el("div", { class: "cap" }, caption.text);
    node.style.left = (100 * caption.start_frame / total) + "%";
    node.style.width = (100 * caption.duration_frames / total) + "%";
    node.addEventListener("pointerdown", () => { selectedId = caption.id; renderWorkspace(); });
    lane.append(node);
  }
  lane.append(el("div", { class: "playhead", "data-head": "1" }));
  return lane;
}

function dragClip(event, clip, mode, total) {
  if (page.read_only) return;
  event.preventDefault();
  event.stopPropagation();
  const lane = event.currentTarget.closest(".lane");
  const origin = event.clientX;
  const width = lane.getBoundingClientRect().width;
  function move() {}
  function up(end) {
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointermove", move);
    const delta = Math.round(((end.clientX - origin) / width) * total);
    if (delta === 0 && mode !== "move") { selectedId = clip.id; paintPlayhead(); return; }
    selectedId = clip.id;
    if (mode === "move") send({ op: "move", clip_id: clip.id, frame: Math.max(0, clip.start_frame + delta), track: clip.track });
    else if (mode === "in" || mode === "out") send({ op: "trim", clip_id: clip.id, edge: mode, frames: delta });
  }
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
}

function renderInspector(version) {
  const box = el("div", { class: "inspector", id: "inspector" });
  const clip = version.clips.find((item) => item.id === selectedId);
  const caption = version.captions.find((item) => item.id === selectedId) || captionAt(version, playhead);
  if (clip) {
    box.append(el("strong", null, clip.name));
    const length = el("input", { type: "number", min: "1", value: String(clip.duration_frames) });
    const applyLen = el("button", { type: "button" }, "Set length");
    applyLen.addEventListener("click", () => send({ op: "length", clip_id: clip.id, frames: Number(length.value) }));
    const gain = el("input", { type: "range", min: "0", max: "200", value: String(Math.round(clip.gain_permille / 10)) });
    const mute = el("input", { type: "checkbox" });
    mute.checked = clip.muted;
    const applyGain = el("button", { type: "button" }, "Set volume");
    applyGain.addEventListener("click", () => send({ op: "gain", clip_id: clip.id, permille: Number(gain.value) * 10, muted: mute.checked }));
    box.append(field("Length (frames)", length), applyLen, field("Volume %", gain));
    const muteRow = el("label", { class: "row" });
    muteRow.append(mute, el("span", null, "Mute"));
    box.append(muteRow, applyGain);
  }
  if (caption) {
    const text = el("textarea");
    text.value = caption.text;
    const save = el("button", { type: "button" }, "Save caption");
    save.addEventListener("click", () => send({ op: "caption", caption_id: caption.id, text: text.value }));
    box.append(field("Caption", text), save);
  }
  const w = el("input", { type: "number", min: "1", value: String(version.width) });
  const h = el("input", { type: "number", min: "1", value: String(version.height) });
  const format = el("button", { type: "button" }, "Change format");
  format.addEventListener("click", () => send({ op: "change_format", width: Number(w.value), height: Number(h.value) }));
  box.append(field("Format width", w), field("Format height", h), format);
  return box;
}

function renderDrawer(version) {
  const drawer = el("aside", { class: "drawer" });
  const tabs = el("div", { class: "tabs" });
  for (const name of ["comments", "history", "compare"]) {
    const button = el("button", { type: "button", class: tab === name ? "on" : "" }, name[0].toUpperCase() + name.slice(1));
    button.addEventListener("click", () => { tab = name; renderWorkspace(); });
    tabs.append(button);
  }
  drawer.append(tabs);
  const list = el("div", { class: "list" });
  if (tab === "comments") renderComments(list, version);
  else if (tab === "history") renderHistory(list, version);
  else renderCompare(list);
  drawer.append(list);
  return drawer;
}

function renderComments(list, version) {
  if (!version.comments.length) list.append(el("p", { class: "note" }, "Drag a box on the picture, or mark a time range."));
  for (const comment of version.comments) {
    const card = el("div", { class: "card" + (comment.resolved ? " resolved" : "") });
    card.addEventListener("click", () => { playhead = comment.start_frame; paintPlayhead(); });
    const when = comment.start_frame === comment.end_frame ? tc(comment.start_frame, version.fps_num) : tc(comment.start_frame, version.fps_num) + "–" + tc(comment.end_frame, version.fps_num);
    card.append(el("div", { class: "meta" }, comment.author + " · " + when + (comment.region ? " · frame region" : " · time range")));
    card.append(el("p", null, comment.body));
    if (comment.fixed_in) card.append(el("span", { class: "badge" }, "Fixed in " + comment.fixed_in));
    for (const reply of comment.replies) {
      const line = el("div", { class: "note" });
      line.textContent = reply.author + ": " + reply.body;
      card.append(line);
    }
    const row = el("div", { class: "row" });
    const resolve = el("button", { type: "button" }, comment.resolved ? "Reopen" : "Resolve");
    resolve.addEventListener("click", (event) => {
      event.stopPropagation();
      send({ op: "resolve", comment_id: comment.id, resolved: !comment.resolved, fixed_in: comment.resolved ? null : version.difference });
    });
    const replyText = el("input", { placeholder: "Reply" });
    const reply = el("button", { type: "button" }, "Reply");
    reply.addEventListener("click", (event) => {
      event.stopPropagation();
      if (replyText.value.trim()) send({ op: "reply", comment_id: comment.id, body: replyText.value });
    });
    row.append(resolve, replyText, reply);
    card.append(row);
    list.append(card);
  }
}

function renderHistory(list, version) {
  const row = el("div", { class: "row" });
  const undo = el("button", { type: "button" }, "Undo");
  const redo = el("button", { type: "button" }, "Redo");
  undo.disabled = !version.can_undo || page.read_only;
  redo.disabled = !version.can_redo || page.read_only;
  undo.addEventListener("click", () => send({ op: "undo" }, reasonIfAgent()));
  redo.addEventListener("click", () => send({ op: "redo" }));
  row.append(undo, redo, el("span", { class: "note" }, version.event_count + " recorded steps"));
  list.append(row);
  const events = version.events.slice().reverse();
  for (const event of events) {
    const card = el("div", { class: "card" + (event.in_effect ? " active" : "") });
    card.append(el("div", { class: "meta" }, event.actor + " · " + event.at));
    card.append(el("div", null, event.summary));
    card.append(el("div", { class: "note" }, event.action));
    if (event.reason) card.append(el("div", null, event.reason));
    const restore = el("button", { type: "button" }, "Restore this step");
    restore.addEventListener("click", () => send({ op: "restore", event_id: event.id }, reasonIfAgent()));
    card.append(restore);
    list.append(card);
  }
}

function renderCompare(list) {
  const select = el("select");
  for (const item of page.versions) {
    if (item.id !== page.version.id) select.append(el("option", { value: item.id }, item.difference));
  }
  const button = el("button", { type: "button" }, "Compare");
  const out = el("div");
  button.addEventListener("click", async () => {
    if (!select.value) return;
    try {
      const diff = await api("/api/workspace/compare/" + page.version.id + "/" + select.value);
      clear(out);
      const add = (title, rows) => {
        if (!rows.length) return;
        out.append(el("h3", null, title));
        for (const row of rows) out.append(el("div", { class: "note" }, row));
      };
      add("Only on this cut", diff.only_left);
      add("Only on the other cut", diff.only_right);
      add("Length", diff.duration_deltas);
      add("Captions", diff.caption_changes);
      if (!out.childNodes.length) out.append(el("p", { class: "note" }, "These cuts match on clips and captions."));
    } catch (error) { showError(error); }
  });
  list.append(select, button, out);
}

function reasonIfAgent() {
  if (page.role !== "agent") return null;
  const reason = window.prompt("If this replaces a client edit, add the reason. Leave blank when it does not.");
  return reason || null;
}

function captionAt(version, frame) {
  return version.captions.find((caption) => frame >= caption.start_frame && frame < caption.start_frame + caption.duration_frames);
}

function clipAt(version, frame) {
  return version.clips.find((clip) => clip.kind === "video" && frame >= clip.start_frame && frame < clip.start_frame + clip.duration_frames);
}

function paintPlayhead() {
  if (!page) return;
  const version = page.version;
  const total = durationFrames(version);
  const pct = (100 * playhead / total) + "%";
  document.querySelectorAll("[data-head]").forEach((node) => { node.style.left = pct; });
  const tcNode = document.querySelector("#tc");
  if (tcNode) tcNode.textContent = tc(playhead, version.fps_num);
  const clip = clipAt(version, playhead);
  const title = document.querySelector("#clip-title");
  const picture = document.querySelector(".picture");
  if (title) title.textContent = clip ? clip.name : "";
  if (picture) {
    const hue = clip ? (clip.name.length * 37) % 50 : 30;
    picture.style.background = "linear-gradient(160deg, hsl(" + (20 + hue) + " 28% 28%), #14110e 70%)";
  }
  const caption = captionAt(version, playhead);
  const overlay = document.querySelector("#caption-overlay");
  if (overlay) {
    overlay.textContent = caption ? caption.text : "";
    overlay.style.top = caption ? (caption.anchor_y / 10) + "%" : "78%";
  }
  document.querySelectorAll(".region").forEach((node) => node.remove());
  const monitor = document.querySelector(".monitor");
  if (!monitor) return;
  for (const comment of version.comments) {
    if (!comment.region) continue;
    const inside = playhead >= comment.start_frame && playhead <= comment.end_frame;
    if (!inside) continue;
    const box = el("div", { class: "region" });
    box.style.left = (comment.region.x / 10) + "%";
    box.style.top = (comment.region.y / 10) + "%";
    box.style.width = (comment.region.w / 10) + "%";
    box.style.height = (comment.region.h / 10) + "%";
    monitor.append(box);
  }
  if (draft) {
    const box = el("div", { class: "region draft" });
    box.style.left = (draft.x / 10) + "%";
    box.style.top = (draft.y / 10) + "%";
    box.style.width = (draft.w / 10) + "%";
    box.style.height = (draft.h / 10) + "%";
    monitor.append(box);
  }
}

function startRegion(event, monitor) {
  if (page.read_only || event.button !== 0) return;
  const rect = monitor.getBoundingClientRect();
  const point = (ev) => ({
    x: Math.max(0, Math.min(999, Math.round(((ev.clientX - rect.left) / rect.width) * 1000))),
    y: Math.max(0, Math.min(999, Math.round(((ev.clientY - rect.top) / rect.height) * 1000))),
  });
  const start = point(event);
  function move(ev) {
    const end = point(ev);
    draft = normalize(start, end);
    paintPlayhead();
  }
  function up(ev) {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    let region = normalize(start, point(ev));
    if (region.w < 40 || region.h < 40) {
      region = { x: Math.max(0, start.x - 120), y: Math.max(0, start.y - 70), w: 240, h: 140 };
      if (region.x + region.w > 1000) region.x = 1000 - region.w;
      if (region.y + region.h > 1000) region.y = 1000 - region.h;
    }
    draft = region;
    paintPlayhead();
    const pop = el("form", { class: "popover" });
    const input = el("textarea", { placeholder: "Comment on this region" });
    const save = el("button", { class: "primary", type: "submit" }, "Add comment");
    const cancel = el("button", { type: "button" }, "Cancel");
    pop.append(input, save, cancel);
    pop.style.left = "8px";
    pop.style.top = "8px";
    monitor.append(pop);
    input.focus();
    cancel.addEventListener("click", () => { draft = null; pop.remove(); paintPlayhead(); });
    pop.addEventListener("submit", (submitEvent) => {
      submitEvent.preventDefault();
      const body = input.value.trim();
      draft = null;
      pop.remove();
      if (!body) { paintPlayhead(); return; }
      send({ op: "comment", body, start_frame: playhead, end_frame: playhead, region });
    });
  }
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
}

function normalize(a, b) {
  const x = Math.min(a.x, b.x);
  const y = Math.min(a.y, b.y);
  return { x, y, w: Math.max(1, Math.abs(a.x - b.x)), h: Math.max(1, Math.abs(a.y - b.y)) };
}

function commentRange(monitor) {
  const start = Number(monitor.dataset.in);
  const end = Number(monitor.dataset.out);
  if (!Number.isFinite(start) || !Number.isFinite(end)) {
    showError(new Error("Mark an in and an out first."));
    return;
  }
  const body = window.prompt("Comment on this time range");
  if (!body) return;
  const lo = Math.min(start, end);
  const hi = Math.max(start, end);
  send({ op: "comment", body, start_frame: lo, end_frame: hi, region: null });
}

function togglePlay() {
  playing = !playing;
  const button = document.querySelector("#play");
  if (button) button.textContent = playing ? "Pause" : "Play";
  if (!playing) return;
  let last = performance.now();
  const step = (now) => {
    if (!playing || !page) return;
    const fps = page.version.fps_num / page.version.fps_den;
    playhead += Math.round(((now - last) / 1000) * fps);
    last = now;
    const end = durationFrames(page.version);
    if (playhead >= end) { playhead = end; playing = false; if (button) button.textContent = "Play"; }
    paintPlayhead();
    if (playing) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

async function send(edit, reason) {
  if (page && page.read_only) { showError(new Error(page.read_only_reason)); return; }
  try {
    const keep = playhead;
    page = await api("/api/workspace/" + page.version.id + "/edits", { method: "POST", json: { reason: reason || null, edit } });
    playhead = keep;
    renderWorkspace();
  } catch (error) { showError(error); }
}

window.addEventListener("hashchange", boot);
window.addEventListener("keydown", (event) => {
  if (!page) return;
  const typing = event.target && (event.target.tagName === "INPUT" || event.target.tagName === "TEXTAREA");
  if (typing) return;
  if (event.code === "Space") { event.preventDefault(); togglePlay(); }
  if (event.code === "ArrowRight") { playhead += 1; paintPlayhead(); }
  if (event.code === "ArrowLeft") { playhead = Math.max(0, playhead - 1); paintPlayhead(); }
});
boot();
