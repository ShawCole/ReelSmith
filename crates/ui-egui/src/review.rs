//! Review links (`apps/reelsmith-portal`): the same editor opened from a portal link. The host
//! (the web build) fills [`ReviewState`] from the portal and sends what it collects in
//! `requests`; this module locks the shell to the link's role and draws the Comments and Edit
//! History panels.

use std::collections::HashMap;

use egui::{Align2, Rect, RichText, Sense, pos2, vec2};
use reelsmith_engine::review::Role;
use reelsmith_time::Tick;
use serde_json::json;

use crate::ReelsmithApp;
use crate::dock::PanelKind;
use crate::theme::Tokens;

/// The workspace a reviewer or watcher gets: Program and the review panels above, Tools and
/// the Timeline below. It is not in Window ▸ Workspaces.
pub const REVIEW_WORKSPACE: &str = "Client Review";
/// The editor's workspace on a portal link: the default layout with the review panels beside
/// Properties.
pub const PORTAL_EDITOR_WORKSPACE: &str = "Portal Editor";

/// Panels a reviewer or watcher may have open.
pub const REVIEW_PANELS: &[PanelKind] =
    &[PanelKind::Program, PanelKind::Timeline, PanelKind::Tools, PanelKind::AudioMeters, PanelKind::Timecode, PanelKind::Comments, PanelKind::EditHistory];

/// One step in the version's log, as the portal sent it.
#[derive(Clone, Debug, Default)]
pub struct ActionRow {
    pub seq: u64,
    /// When, already in the viewer's words ("14:02").
    pub at: String,
    pub who: String,
    /// `client`, `agent`, `admin`, `watcher`.
    pub kind: String,
    pub label: String,
    pub command: String,
    pub project_changed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct CommentRow {
    pub id: String,
    pub author: String,
    pub start: i64,
    pub end: i64,
    pub body: String,
    pub resolved: bool,
    pub fixed_in: Option<String>,
    /// (author, body)
    pub replies: Vec<(String, String)>,
}

/// What the page asks the portal to do; the host drains these each frame.
#[derive(Clone, Debug, PartialEq)]
pub enum ReviewRequest {
    Comment { start: i64, end: i64, body: String },
    Reply { id: String, body: String },
    Resolve { id: String, resolved: bool },
    Mark { good: bool, note: String },
    Status(String),
}

/// A portal link's state in the editor.
#[derive(Debug)]
pub struct ReviewState {
    pub role: Role,
    /// The person or agent this link belongs to.
    pub actor: String,
    pub project_name: String,
    pub version_label: String,
    /// `draft`, `agent_working`, `ready`, `changes_requested`, `approved`.
    pub status: String,
    pub actions: Vec<ActionRow>,
    pub comments: Vec<CommentRow>,
    pub requests: Vec<ReviewRequest>,
    /// A line for the header (sync errors, "updated by …").
    pub notice: Option<String>,
    draft: String,
    note: String,
    replies: HashMap<String, String>,
}

impl ReviewState {
    pub fn new(role: Role, actor: &str) -> Self {
        Self {
            role,
            actor: actor.to_string(),
            project_name: String::new(),
            version_label: String::new(),
            status: String::new(),
            actions: Vec::new(),
            comments: Vec::new(),
            requests: Vec::new(),
            notice: None,
            draft: String::new(),
            note: String::new(),
            replies: HashMap::new(),
        }
    }

    /// Whether the link keeps the stripped shell (no menus, library, other workspaces).
    pub fn locked(&self) -> bool {
        self.role != Role::Editor
    }
}

impl ReelsmithApp {
    /// Open this editor as a portal link: lock the engine to the role, start collecting
    /// executed commands, and lay out the role's workspace. Call after the project is open.
    pub fn enter_review(&mut self, state: ReviewState) {
        self.session.enter_review(state.role);
        let workspace = if state.locked() { REVIEW_WORKSPACE } else { PORTAL_EDITOR_WORKSPACE };
        self.ui.dock = crate::dock::workspace(workspace);
        self.ui.mode = crate::state::Mode::Edit;
        self.review = Some(state);
    }

    /// Change role in place (a reviewer becomes a watcher while the editor works, and back).
    pub fn set_review_role(&mut self, role: Role) {
        let Some(review) = &mut self.review else { return };
        if review.role == role {
            return;
        }
        review.role = role;
        self.session.policy = Some(reelsmith_engine::review::policy(role));
    }

    /// Whether `p` may be shown on this link.
    pub fn review_allows_panel(&self, p: PanelKind) -> bool {
        match &self.review {
            Some(r) if r.locked() => REVIEW_PANELS.contains(&p),
            _ => true,
        }
    }

    /// Whether a UI-only command (`window.*`, modes, quit…) may run on this link. Engine commands
    /// are decided by the session's policy.
    pub fn review_allows_ui_command(&self, id: &str) -> bool {
        let Some(r) = &self.review else { return true };
        if !r.locked() {
            return !id.starts_with("window.workspace") && !matches!(id, "app.quit" | "file.closeProject" | "file.closeAllProjects");
        }
        if let Some(panel) = id.strip_prefix("window.panel.") {
            return PanelKind::from_name(panel).is_some_and(|p| REVIEW_PANELS.contains(&p));
        }
        !(id.starts_with("window.") || id.starts_with("mode.") || id.starts_with("file.") || id.starts_with("prefs.") || id == "app.quit")
    }
}

fn timecode(app: &ReelsmithApp, t: i64) -> String {
    let rate = app.session.active_sequence().map(|s| s.settings.frame_rate).unwrap_or_default();
    reelsmith_time::format_timecode_frames(rate.frame_at(Tick(t)), rate, false)
}

fn seek(app: &mut ReelsmithApp, t: i64) {
    if let Err(e) = app.session.execute("playhead.set", json!({"time": t})) {
        app.ui.status = e.to_string();
    }
}

/// The header that replaces the menu bar on a locked link: what is being reviewed, by whom,
/// its status, and the reviewer's mark.
pub fn header(app: &mut ReelsmithApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    ui.painter().rect_filled(rect, 0.0, t.header_bg);
    let Some(review) = &app.review else { return };
    let title = if review.version_label.is_empty() { review.project_name.clone() } else { format!("{} · {}", review.project_name, review.version_label) };
    let who = match review.role {
        Role::Reviewer => format!("Reviewing as {}", review.actor),
        Role::Watcher if review.status == "agent_working" => "Your editor is working. You can watch the changes live.".to_string(),
        Role::Watcher => "Watching".to_string(),
        Role::Editor => format!("Editing as {}", review.actor),
    };
    let status = status_words(&review.status).to_string();
    let notice = review.notice.clone();
    let role = review.role;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(12.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.label(RichText::new(title).strong().size(14.0).color(t.text));
    child.add_space(16.0);
    child.label(RichText::new(who).size(12.0).color(t.text_dim));
    if !status.is_empty() {
        child.add_space(12.0);
        child.label(RichText::new(status).size(12.0).color(t.accent));
    }
    if let Some(n) = notice {
        child.add_space(12.0);
        child.label(RichText::new(n).size(12.0).color(t.text_faint));
    }
    if role == Role::Reviewer {
        child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let approve = ui.add(egui::Button::new(RichText::new("Approve").strong()));
            app.auto.add_response("review.approve", &approve, "Approve");
            let changes = ui.button("Request changes");
            app.auto.add_response("review.requestChanges", &changes, "Request changes");
            let note = &mut app.review.as_mut().map(|r| std::mem::take(&mut r.note)).unwrap_or_default();
            ui.add(egui::TextEdit::singleline(note).hint_text("Note for your editor").desired_width(240.0));
            let good = approve.clicked().then_some(true).or(changes.clicked().then_some(false));
            if let Some(r) = app.review.as_mut() {
                if let Some(good) = good {
                    r.requests.push(ReviewRequest::Mark { good, note: std::mem::take(note) });
                }
                r.note = std::mem::take(note);
            }
        });
    }
}

fn status_words(status: &str) -> &'static str {
    match status {
        "draft" => "Draft",
        "agent_working" => "Editor at work",
        "ready" => "Ready for review",
        "changes_requested" => "Changes requested",
        "approved" => "Approved",
        _ => "",
    }
}

/// Comments: every thread on this version, oldest first by time in the cut. A time chip moves
/// the playhead there; the box at the bottom comments at the playhead (or on the In/Out range).
pub fn comments_panel(app: &mut ReelsmithApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let Some(review) = &app.review else {
        crate::dock::placeholder(ui, rect, &t, "Comments appear on review links");
        return;
    };
    let mut comments = review.comments.clone();
    comments.sort_by_key(|c| (c.start, c.id.clone()));
    let can_write = review.role != Role::Watcher;
    let mut seek_to = None;
    let mut out: Vec<ReviewRequest> = Vec::new();
    let mut replies = std::mem::take(&mut app.review.as_mut().map(|r| &mut r.replies).cloned().unwrap_or_default());
    let mut draft = app.review.as_mut().map(|r| std::mem::take(&mut r.draft)).unwrap_or_default();
    let composer_h = if can_write { 64.0 } else { 0.0 };
    let list = Rect::from_min_max(rect.min, pos2(rect.max.x, rect.max.y - composer_h));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list.shrink(6.0)));
    let labels: Vec<(String, String)> = comments.iter().map(|c| (timecode(app, c.start), if c.end > c.start { timecode(app, c.end) } else { String::new() })).collect();
    egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("review.comments").show(&mut child, |ui| {
        if comments.is_empty() {
            ui.label(RichText::new("No comments yet.").color(t.text_faint));
        }
        for (c, (from, to)) in comments.iter().zip(&labels) {
            ui.horizontal(|ui| {
                let chip = if to.is_empty() { from.clone() } else { format!("{from}–{to}") };
                if ui.add(egui::Button::new(RichText::new(chip).monospace().size(11.0)).small()).clicked() {
                    seek_to = Some(c.start);
                }
                ui.label(RichText::new(&c.author).strong().size(12.0).color(t.text));
                if c.resolved {
                    let badge = c.fixed_in.as_ref().map_or("Resolved".to_string(), |v| format!("Fixed in {v}"));
                    ui.label(RichText::new(badge).size(11.0).color(t.accent));
                }
            });
            ui.label(RichText::new(&c.body).size(12.0).color(if c.resolved { t.text_dim } else { t.text }));
            for (author, body) in &c.replies {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(RichText::new(author).strong().size(11.0).color(t.text_dim));
                    ui.label(RichText::new(body).size(11.0).color(t.text_dim));
                });
            }
            if can_write {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    let text = replies.entry(c.id.clone()).or_default();
                    let edit = ui.add(egui::TextEdit::singleline(text).hint_text("Reply").desired_width((ui.available_width() - 120.0).max(80.0)));
                    let send = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.small_button("Reply").clicked() || send) && !text.trim().is_empty() {
                        out.push(ReviewRequest::Reply { id: c.id.clone(), body: std::mem::take(text) });
                    }
                    let label = if c.resolved { "Reopen" } else { "Resolve" };
                    if ui.small_button(label).clicked() {
                        out.push(ReviewRequest::Resolve { id: c.id.clone(), resolved: !c.resolved });
                    }
                });
            }
            ui.separator();
        }
    });
    if can_write {
        let composer = Rect::from_min_max(pos2(rect.min.x, rect.max.y - composer_h), rect.max);
        ui.painter().hline(composer.x_range(), composer.min.y, egui::Stroke::new(1.0, t.separator));
        let (start, end) = comment_range(app);
        let at = if end > start { format!("{}–{}", timecode(app, start), timecode(app, end)) } else { timecode(app, start) };
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(composer.shrink(6.0)));
        c.label(RichText::new(format!("Comment at {at}")).size(11.0).color(t.text_dim));
        c.horizontal(|ui| {
            let edit = ui.add(egui::TextEdit::singleline(&mut draft).hint_text("What should change here?").desired_width((ui.available_width() - 70.0).max(80.0)));
            app.auto.add_response("review.comment.text", &edit, "Comment");
            let send = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let add = ui.button("Add");
            app.auto.add_response("review.comment.add", &add, "Add");
            if (add.clicked() || send) && !draft.trim().is_empty() {
                out.push(ReviewRequest::Comment { start, end, body: std::mem::take(&mut draft) });
            }
        });
    }
    if let Some(r) = app.review.as_mut() {
        r.draft = draft;
        r.replies = std::mem::take(&mut replies);
        r.requests.extend(out);
    }
    if let Some(t) = seek_to {
        seek(app, t);
    }
}

/// The comment's time: the In/Out range when both are set, else the playhead.
fn comment_range(app: &ReelsmithApp) -> (i64, i64) {
    let playhead = app.session.playhead().0;
    let marks = app.session.active_sequence().map(|s| (s.mark_in, s.mark_out));
    match marks {
        Some((Some(i), Some(o))) if o.0 > i.0 => (i.0, o.0),
        _ => (playhead, playhead),
    }
}

/// Edit History: everything that happened to this version, newest first, by whom and when.
/// Selection and playhead steps are left out.
pub fn history_panel(app: &mut ReelsmithApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let Some(review) = &app.review else {
        crate::dock::placeholder(ui, rect, &t, "Edit history appears on review links");
        return;
    };
    let rows: Vec<ActionRow> = review.actions.iter().filter(|a| a.project_changed || a.command.starts_with("portal.")).rev().cloned().collect();
    let editor = review.role == Role::Editor;
    let status = review.status.clone();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(6.0)));
    if editor {
        child.horizontal(|ui| {
            let working = status == "agent_working";
            let (label, next) = if working { ("Ready for review", "ready") } else { ("I'm editing", "agent_working") };
            let b = ui.button(label);
            app.auto.add_response("review.status", &b, label);
            if b.clicked()
                && let Some(r) = app.review.as_mut()
            {
                r.requests.push(ReviewRequest::Status(next.to_string()));
            }
        });
        child.separator();
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("review.history").show(&mut child, |ui| {
        if rows.is_empty() {
            ui.label(RichText::new("No changes yet.").color(t.text_faint));
        }
        for a in &rows {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
            let colour = if a.kind == "client" { t.accent } else { t.text };
            ui.painter().text(pos2(r.min.x + 2.0, r.min.y + 9.0), Align2::LEFT_CENTER, &a.label, Tokens::ui(12.0), t.text);
            ui.painter().text(pos2(r.min.x + 2.0, r.min.y + 25.0), Align2::LEFT_CENTER, format!("{} · {}", a.who, a.at), Tokens::ui(11.0), colour);
            ui.painter().text(pos2(r.max.x - 2.0, r.min.y + 9.0), Align2::RIGHT_CENTER, format!("#{}", a.seq), Tokens::ui(10.0), t.text_faint);
        }
    });
}
