//! Product name drawn as text. Upstream trademark logos are not included.

use crate::theme::Tokens;

/// Draw the product name `height` points tall, left edge at `left_center` (vertically centred).
pub fn paint_wordmark(ui: &egui::Ui, left_center: egui::Pos2, height: f32, dark: bool) -> Option<egui::Rect> {
    let color = if dark { egui::Color32::WHITE } else { egui::Color32::from_rgb(20, 20, 24) };
    let galley = ui.painter().layout_no_wrap("ReelSmith".to_string(), Tokens::semibold(height.max(1.0)), color);
    let r = egui::Rect::from_min_size(egui::pos2(left_center.x, left_center.y - galley.size().y / 2.0), galley.size());
    ui.painter().galley(r.min, galley, color);
    Some(r)
}
