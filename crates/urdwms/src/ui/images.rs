//! Photos on screen.
//!
//! A phone or camera still is 8–12 megapixels; decoding one takes a few
//! hundred milliseconds on the tablet's CPU and a full-size texture would eat
//! tens of megabytes of memory. So decoding happens off the UI thread and the
//! texture is scaled to what a screen can show.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use egui::{Color32, ColorImage, CornerRadius, TextureHandle, TextureOptions, Ui, Vec2};

const MAX_SIDE: u32 = 1600;

enum Slot {
    Loading(Receiver<Result<ColorImage, String>>),
    Ready(TextureHandle),
    Failed(String),
}

#[derive(Default)]
pub struct ImageCache {
    slots: HashMap<PathBuf, Slot>,
}

pub fn decode(bytes: &[u8]) -> Result<ColorImage, String> {
    use image::ImageDecoder;
    let err = |e: image::ImageError| format!("影像無法解碼：{e}");
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("影像無法解碼：{e}"))?;
    let mut decoder = reader.into_decoder().map_err(err)?;
    // Tablet and phone cameras store portrait shots sideways plus an EXIF
    // flag; honour it, or every label is read lying on its side.
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(err)?;
    img.apply_orientation(orientation);
    let img = if img.width().max(img.height()) > MAX_SIDE { img.thumbnail(MAX_SIDE, MAX_SIDE) } else { img };
    let rgba = img.to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

impl ImageCache {
    fn slot(&mut self, ctx: &egui::Context, path: &Path) -> &mut Slot {
        if !self.slots.contains_key(path) {
            // Keep memory bounded on a long shift: textures are cheap to
            // rebuild from disk, the tablet's 8 GB is not elastic.
            if self.slots.len() >= 40 {
                self.slots.clear();
            }
            let (tx, rx) = channel();
            let owned = path.to_path_buf();
            let ctx2 = ctx.clone();
            std::thread::spawn(move || {
                let result = std::fs::read(&owned).map_err(|e| format!("讀不到影像：{e}")).and_then(|b| decode(&b));
                let _ = tx.send(result);
                ctx2.request_repaint();
            });
            self.slots.insert(path.to_path_buf(), Slot::Loading(rx));
        }
        let slot = self.slots.get_mut(path).unwrap();
        if let Slot::Loading(rx) = slot
            && let Ok(result) = rx.try_recv()
        {
            *slot = match result {
                Ok(img) => Slot::Ready(ctx.load_texture(path.to_string_lossy(), img, TextureOptions::LINEAR)),
                Err(e) => Slot::Failed(e),
            };
        }
        slot
    }

    /// Show a photo fitted inside `max`, on a dark ground so the box stands out.
    pub fn show(&mut self, ui: &mut Ui, path: &Path, max: Vec2) {
        let ctx = ui.ctx().clone();
        match self.slot(&ctx, path) {
            Slot::Ready(tex) => {
                let size = tex.size_vec2();
                let scale = (max.x / size.x).min(max.y / size.y).min(1.0);
                let shown = size * scale;
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(max.x.min(ui.available_width()), shown.y), egui::Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(8), Color32::from_rgb(0x11, 0x11, 0x11));
                let image_rect = egui::Rect::from_center_size(rect.center(), shown);
                ui.painter().image(
                    tex.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            Slot::Loading(_) => {
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(max.x.min(ui.available_width()), max.y.min(240.0)),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(rect, CornerRadius::same(8), Color32::from_rgb(0x22, 0x22, 0x22));
                ui.put(rect, egui::Spinner::new().size(28.0));
            }
            Slot::Failed(e) => {
                let e = e.clone();
                ui.colored_label(super::theme::BAD, e);
            }
        }
    }

    /// Hand over an already-decoded image (the worker decodes alongside
    /// recognition, so the verdict screen shows the photo at once).
    pub fn insert(&mut self, ctx: &egui::Context, path: &Path, image: ColorImage) {
        let tex = ctx.load_texture(path.to_string_lossy(), image, TextureOptions::LINEAR);
        self.slots.insert(path.to_path_buf(), Slot::Ready(tex));
    }

}
