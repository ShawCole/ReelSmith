// ReelSmith Review home: the admin overview (#/admin/{token}) and a client's projects and
// upload form (#/c/{token}). This is not the review UI: reviewing happens in the real editor
// at /review/#{token}. Tokens stay in the fragment and travel as a Bearer header.
"use strict";

const TICKS_PER_SECOND = 254016000000n;
const PART = 8 * 1024 * 1024;
const app = document.querySelector("#app");
let token = "";

function el(tag, attrs, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === false || v == null) continue;
    node.setAttribute(k, v === true ? "" : v);
  }
  for (const child of children) node.append(child);
  return node;
}

function field(label, control) {
  return el("label", { class: "field" }, el("span", null, label), control);
}

async function api(path, options = {}) {
  const headers = { Authorization: "Bearer " + token };
  let body;
  if (options.json !== undefined) {
    headers["Content-Type"] = "application/json";
    body = JSON.stringify(options.json);
  }
  const response = await fetch(path, { method: options.method || "GET", headers, body });
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.message || (data.errors || []).map((e) => e.message).join("; ") || "Request failed");
  return data;
}

function linkRow(label, href) {
  const url = location.origin + href;
  return el("div", { class: "links" }, el("span", null, label + ": "), el("a", { href: url }, url));
}

async function route() {
  const hash = location.hash.replace(/^#\/?/, "");
  const [kind, value] = hash.split("/");
  token = value || "";
  app.replaceChildren();
  try {
    if (kind === "admin" && token) await adminView();
    else if (kind === "c" && token) await clientView();
    else app.append(el("p", { class: "note" }, "Open the link you were sent."));
  } catch (error) {
    app.append(el("p", { class: "error" }, error.message));
  }
}

async function adminView() {
  const data = await api("/api/admin/overview");
  app.append(el("h2", null, "Clients"));
  for (const client of data.clients) {
    app.append(el("div", { class: "card" }, el("strong", null, client.name), linkRow("Upload link", "/#/c/" + client.token)));
  }
  const name = el("input", { type: "text", placeholder: "Client name" });
  const add = el("button", { class: "primary" }, "Add client");
  add.addEventListener("click", async () => {
    try { await api("/api/admin/clients", { method: "POST", json: { name: name.value } }); route(); } catch (e) { alert(e.message); }
  });
  app.append(el("div", { class: "row" }, name, add));
  app.append(el("h2", null, "Projects"));
  for (const p of data.projects) {
    const card = el("div", { class: "card" }, el("strong", null, p.name + " · " + p.client_name + " · " + p.final_type));
    card.append(linkRow("Editor", "/edit/#" + p.editor_token), linkRow("Review", "/review/#" + p.reviewer_token), linkRow("Watch", "/watch/#" + p.watcher_token));
    card.append(el("div", { class: "note" }, p.versions.map((v) => v.label + " (" + v.status.replace(/_/g, " ") + ")").join(" · ")));
    app.append(card);
  }
}

async function clientView() {
  const data = await api("/api/home");
  app.append(el("h2", null, data.client_name + ": your projects"));
  if (!data.projects.length) app.append(el("p", { class: "note" }, "No projects yet."));
  for (const p of data.projects) {
    const card = el("div", { class: "card" }, el("strong", null, p.name));
    card.append(linkRow("Review", "/review/#" + p.reviewer_token));
    card.append(el("div", { class: "note" }, p.versions.map((v) => v.label + " (" + v.status.replace(/_/g, " ") + ")").join(" · ")));
    app.append(card);
  }
  app.append(el("h2", null, "New project"));
  app.append(uploadForm(data));
}

function dropZone(label, multiple) {
  const input = el("input", { type: "file", multiple, accept: "video/*,audio/*,image/*", class: "hidden" });
  const list = el("ul", { class: "files" });
  const zone = el("div", { class: "drop" }, label + " (drop files here or click)");
  let files = [];
  const show = () => list.replaceChildren(...files.map((f) => el("li", null, f.name + " · " + Math.round(f.size / 1048576) + " MB")));
  zone.addEventListener("click", () => input.click());
  input.addEventListener("change", () => { files = [...input.files]; show(); });
  zone.addEventListener("dragover", (e) => { e.preventDefault(); zone.classList.add("over"); });
  zone.addEventListener("dragleave", () => zone.classList.remove("over"));
  zone.addEventListener("drop", (e) => { e.preventDefault(); zone.classList.remove("over"); files = [...e.dataTransfer.files]; show(); });
  return { node: el("div", null, zone, input, list), files: () => files };
}

function uploadForm(data) {
  const form = el("form");
  const name = el("input", { type: "text", required: true, value: "New cut" });
  const type = el("select", null, el("option", { value: "reel" }, "Reel"), el("option", { value: "ad" }, "Ad"), el("option", { value: "long_form" }, "Long-form"));
  const endCard = el("input", { type: "checkbox" });
  const syncEndCard = () => { endCard.checked = type.value === "ad"; };
  type.addEventListener("change", syncEndCard);
  syncEndCard();
  const targetKind = el("select", null, el("option", { value: "unspecified" }, "Profile default"), el("option", { value: "exact" }, "Exact"), el("option", { value: "range" }, "Range"));
  const exact = el("input", { type: "number", min: "1", placeholder: "seconds" });
  const min = el("input", { type: "number", min: "1", placeholder: "min seconds" });
  const max = el("input", { type: "number", min: "1", placeholder: "max seconds" });
  const exactRow = field("Exact length (seconds)", exact);
  const rangeRow = el("div", null, field("Minimum (seconds)", min), field("Maximum (seconds)", max));
  const syncTarget = () => { exactRow.classList.toggle("hidden", targetKind.value !== "exact"); rangeRow.classList.toggle("hidden", targetKind.value !== "range"); };
  targetKind.addEventListener("change", syncTarget);
  syncTarget();
  const count = el("input", { type: "number", min: "1", max: "12", value: "1" });
  const diffs = el("div");
  const syncDiffs = () => {
    const n = Math.max(1, Math.min(12, Number(count.value) || 1));
    diffs.replaceChildren();
    for (let i = 0; i < n; i++) {
      const input = el("input", { type: "text", placeholder: n === 1 ? "Optional" : "What is different (hook, pacing, B-roll)" });
      diffs.append(field(n === 1 ? "Version label" : "Version " + (i + 1), input));
    }
  };
  count.addEventListener("input", syncDiffs);
  syncDiffs();
  const brollOn = el("input", { type: "checkbox" });
  const brollPanel = el("div", { class: "hidden card" });
  const checks = [];
  for (const item of data.broll) {
    const box = el("input", { type: "checkbox", value: item.id });
    checks.push(box);
    brollPanel.append(el("label", { class: "row" }, box, el("span", null, item.name + " · " + item.width + "×" + item.height)));
  }
  if (!data.broll.length) brollPanel.append(el("p", { class: "note" }, "No B-roll in your library yet. Add some below, or leave it to the editor."));
  const brollDrop = dropZone("B-roll files (kept separate from the raw footage)", true);
  brollPanel.append(brollDrop.node);
  brollOn.addEventListener("change", () => brollPanel.classList.toggle("hidden", !brollOn.checked));
  const rawDrop = dropZone("Raw A-roll footage", true);
  const status = el("p", { class: "note" });
  const submit = el("button", { class: "primary", type: "submit" }, "Upload and create project");
  form.append(
    field("Project name", name),
    field("Final type", type),
    el("label", { class: "row" }, endCard, el("span", null, "End card")),
    field("Target final length", targetKind), exactRow, rangeRow,
    field("How many versions?", count), diffs,
    el("label", { class: "row" }, brollOn, el("span", null, "Use B-roll?")), brollPanel,
    el("div", { class: "field" }, el("span", null, "Raw footage"), rawDrop.node),
    submit, status,
  );
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    submit.disabled = true;
    try {
      const raws = await describe(rawDrop.files());
      if (!raws.length) throw new Error("Add the raw footage first.");
      for (const f of raws) { status.textContent = "Uploading " + f.file.name + "…"; await upload(f, "aroll", status); }
      const picked = checks.filter((b) => b.checked).map((b) => b.value);
      if (brollOn.checked) {
        const brolls = await describe(brollDrop.files());
        for (const f of brolls) { status.textContent = "Uploading " + f.file.name + "…"; await upload(f, "broll", status); }
        const names = brolls.map((f) => f.file.name.replace(/\.[^.]+$/, ""));
        for (const item of (await api("/api/home")).broll) if (names.includes(item.name) && !picked.includes(item.id)) picked.push(item.id);
      }
      const brief = {
        project_name: name.value,
        final_type: type.value,
        end_card: endCard.checked,
        version_count: Number(count.value) || 1,
        differences: [...diffs.querySelectorAll("input")].map((i) => i.value.trim()),
        use_broll: brollOn.checked,
        broll_library_ids: brollOn.checked ? picked : [],
        raw_files: raws.map((f) => ({ name: f.file.name, width: f.width, height: f.height, duration: f.duration, role: "aroll" })),
        target: targetKind.value === "exact" ? { kind: "exact", ticks: ticks(exact.value) }
          : targetKind.value === "range" ? { kind: "range", min: ticks(min.value), max: ticks(max.value) } : { kind: "unspecified" },
      };
      status.textContent = "Creating the project…";
      await api("/api/projects", { method: "POST", json: brief });
      status.textContent = "Project created. Your editor will send it back for review.";
      route();
    } catch (error) {
      status.textContent = error.message;
      status.className = "error";
    } finally {
      submit.disabled = false;
    }
  });
  return form;
}

function ticks(seconds) {
  const ms = BigInt(Math.round(Number(seconds || 0) * 1000));
  return (ms * TICKS_PER_SECOND / 1000n).toString();
}

async function describe(files) {
  const out = [];
  for (const file of files) {
    const p = await probe(file);
    if (!p) throw new Error(file.name + ": the browser could not read its size and length.");
    out.push({ file, width: p.width, height: p.height, duration: ticks(p.seconds) });
  }
  return out;
}

function probe(file) {
  return new Promise((resolve) => {
    const url = URL.createObjectURL(file);
    const done = (v) => { URL.revokeObjectURL(url); resolve(v); };
    if (file.type.startsWith("image/")) {
      const img = new Image();
      img.onload = () => done({ width: img.naturalWidth, height: img.naturalHeight, seconds: 5 });
      img.onerror = () => done(null);
      img.src = url;
      return;
    }
    const video = document.createElement("video");
    video.preload = "metadata";
    video.onloadedmetadata = () => done(video.videoWidth ? { width: video.videoWidth, height: video.videoHeight, seconds: video.duration || 0 } : null);
    video.onerror = () => done(null);
    video.src = url;
  });
}

// Resumable multipart upload: the upload id is kept in localStorage, parts already stored are skipped.
async function upload(f, role, status) {
  const file = f.file;
  const key = "portal-upload:" + role + ":" + file.name + ":" + file.size + ":" + file.lastModified;
  let id = localStorage.getItem(key);
  let have = [];
  if (id) {
    try {
      const s = await api("/api/uploads/" + id);
      if (s.complete) { localStorage.removeItem(key); return; }
      have = s.parts || [];
    } catch { id = null; }
  }
  if (!id) {
    const started = await api("/api/uploads", { method: "POST", json: { filename: file.name, role, size: file.size } });
    if (started.mode === "r2") throw new Error(started.message);
    id = started.id;
    localStorage.setItem(key, id);
  }
  const parts = Math.max(1, Math.ceil(file.size / PART));
  for (let n = 1; n <= parts; n++) {
    if (have.includes(n)) continue;
    const response = await fetch("/api/uploads/" + id + "/" + n, {
      method: "PUT",
      headers: { Authorization: "Bearer " + token },
      body: file.slice((n - 1) * PART, Math.min(file.size, n * PART)),
    });
    if (!response.ok) throw new Error((await response.json().catch(() => ({}))).message || "Upload part failed");
    status.textContent = "Uploading " + file.name + " (" + Math.round((n / parts) * 100) + "%)";
  }
  await api("/api/uploads/" + id + "/complete", {
    method: "POST",
    json: { name: file.name.replace(/\.[^.]+$/, ""), width: f.width, height: f.height, duration: f.duration },
  });
  localStorage.removeItem(key);
}

window.addEventListener("hashchange", route);
route();
