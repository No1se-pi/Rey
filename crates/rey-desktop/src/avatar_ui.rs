use crate::theme::*;
use eframe::egui::{
    self, Color32, ColorImage, Pos2, Rect, RichText, Stroke, TextureHandle, TextureOptions, Vec2,
};
use rey_core::avatar::{AvatarController, Clip, Manifest, State};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

enum Texture {
    Loading,
    Loaded(TextureHandle),
    Missing,
}

type Decoded = (PathBuf, anyhow::Result<ColorImage>);

pub struct AvatarArt {
    manifest: Manifest,
    textures: HashMap<PathBuf, Texture>,
    queue: Sender<PathBuf>,
    completed: Receiver<Decoded>,
    pub error: Option<String>,
}

impl AvatarArt {
    pub fn load(root: &Path, context: &egui::Context) -> Self {
        let mut details = None;
        let mut selected = (root.join("assets/avatar"), Manifest::default());
        for directory in [root.join("assets/avatar"), root.join("app/assets/avatar")] {
            let file = directory.join("manifest.json");
            if !file.is_file() {
                continue;
            }
            match std::fs::read(&file)
                .map_err(anyhow::Error::from)
                .and_then(|bytes| Ok(serde_json::from_slice::<Manifest>(&bytes)?))
            {
                Ok(manifest) if !manifest.states.is_empty() => {
                    selected = (directory, manifest);
                    break;
                }
                Ok(_) => (),
                Err(error) => details = Some(format!("{} · {error}", file.display())),
            }
        }
        let (directory, manifest) = selected;
        let expected = manifest
            .canvas
            .as_ref()
            .map(|canvas| (canvas.width, canvas.height));
        let (queue, requests) = mpsc::channel::<PathBuf>();
        let (responses, completed) = mpsc::channel();
        let ctx = context.clone();
        let _ = std::thread::Builder::new()
            .name("rey-avatar-decode".to_owned())
            .spawn(move || {
                for path in requests {
                    let decoded = decode(&directory, &path, expected);
                    if responses.send((path, decoded)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
        let mut art = Self {
            manifest,
            textures: HashMap::new(),
            queue,
            completed,
            error: details,
        };
        let initial = art.paths_to_preload();
        for path in initial {
            art.request(path);
        }
        art
    }

    fn paths_to_preload(&self) -> Vec<PathBuf> {
        let mut files: Vec<_> = [
            State::Idle,
            State::Listening,
            State::Thinking,
            State::Speaking,
            State::Happy,
            State::Sad,
            State::Confused,
            State::Surprised,
            State::Annoyed,
            State::Sleeping,
            State::Error,
        ]
        .iter()
        .filter_map(|state| {
            self.manifest
                .states
                .get(state)?
                .frame(Duration::ZERO, 0.0)
                .cloned()
        })
        .collect();
        if let Some(blink) = &self.manifest.animations.blink {
            files.extend(blink.frames.iter().take(32).map(|frame| frame.file.clone()));
        }
        if let Some(speech) = &self.manifest.animations.speech {
            files.extend(
                [
                    &speech.silence,
                    &speech.frames.closed,
                    &speech.frames.small,
                    &speech.frames.soft,
                    &speech.frames.round,
                ]
                .into_iter()
                .cloned(),
            );
        }
        files
    }

    fn request(&mut self, path: PathBuf) {
        if self.textures.contains_key(&path) || self.textures.len() >= 64 {
            return;
        }
        if self.queue.send(path.clone()).is_ok() {
            self.textures.insert(path, Texture::Loading);
        }
    }

    pub fn envelope_interval(&self) -> Duration {
        Duration::from_millis(
            self.manifest
                .animations
                .speech
                .as_ref()
                .map_or(80, |speech| speech.suggested_update_ms.clamp(30, 200)),
        )
    }

    pub fn show(&mut self, ui: &mut egui::Ui, controller: &AvatarController, height: f32) {
        while let Ok((path, image)) = self.completed.try_recv() {
            let texture = match image {
                Ok(image) => Texture::Loaded(ui.ctx().load_texture(
                    path.to_string_lossy(),
                    image,
                    TextureOptions::LINEAR,
                )),
                Err(error) => {
                    self.error = Some(format!("{} · {error}", path.display()));
                    Texture::Missing
                }
            };
            self.textures.insert(path, texture);
        }
        let (state, time) = controller.snapshot();
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 14.0, Color32::from_rgb(31, 24, 46));
        let center = Pos2::new(rect.center().x, rect.top() + height * 0.46);
        let max_radius = (width * 0.65).min(height * 0.48);
        let glow = match state {
            State::Error | State::Annoyed => [160, 84, 111],
            State::Listening => [104, 162, 147],
            State::Thinking => [111, 101, 186],
            _ => [142, 100, 190],
        };
        for index in (1..=30).rev() {
            let radius = max_radius * index as f32 / 30.0;
            painter.circle_filled(
                center,
                radius,
                Color32::from_rgba_unmultiplied(glow[0], glow[1], glow[2], 3),
            );
        }
        painter.circle_stroke(
            center,
            max_radius * 0.83,
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(199, 166, 237, 30)),
        );
        let (frame, next_frame) =
            self.manifest
                .dynamic_frame(state, time, controller.amplitude, controller.blink_seed);
        let mut paths: Vec<_> = frame.cloned().into_iter().collect();
        for candidate in [state, self.manifest.fallback_state, State::Idle] {
            if let Some(path) = self
                .manifest
                .states
                .get(&candidate)
                .and_then(|clip| clip.frame(time, controller.amplitude))
            {
                paths.push(path.clone());
            }
        }
        let mut found = false;
        for path in paths {
            self.request(path.clone());
            if let Some(Texture::Loaded(texture)) = self.textures.get(&path) {
                let size = self.manifest.canvas.as_ref().map_or_else(
                    || texture.size_vec2(),
                    |canvas| Vec2::new(canvas.width as f32, canvas.height as f32),
                );
                let scale = (width / size.x).min((height - 24.0) / size.y);
                let drawn = size * scale;
                let top_left = Pos2::new(rect.center().x - drawn.x / 2.0, rect.bottom() - drawn.y);
                painter.image(
                    texture.id(),
                    Rect::from_min_size(top_left, drawn),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                found = true;
                break;
            }
        }
        if !found {
            let pulse = if matches!(state, State::Thinking | State::Listening | State::Speaking) {
                (time.as_secs_f32() * 3.0).sin() * 5.0
            } else {
                0.0
            };
            painter.circle_filled(
                center,
                max_radius * 0.37 + pulse,
                Color32::from_rgb(67, 50, 92),
            );
            painter.circle_stroke(
                center,
                max_radius * 0.37 + pulse,
                Stroke::new(1.0_f32, ACCENT),
            );
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                "◇",
                egui::FontId::proportional(44.0),
                ACCENT,
            );
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.colored_label(if state == State::Error { ERROR } else { MINT }, "●");
            ui.label(RichText::new(state.label()).color(TEXT).strong());
        });
        let transition = match state {
            State::Idle => Some(Duration::from_secs(180).saturating_sub(time)),
            State::Happy
            | State::Sad
            | State::Confused
            | State::Surprised
            | State::Annoyed
            | State::Error => Some(Duration::from_secs(5).saturating_sub(time)),
            _ => None,
        };
        let animation = match self.manifest.states.get(&state) {
            Some(Clip::Frames { fps, .. }) => {
                Some(Duration::from_secs_f32(1.0 / fps.clamp(1.0, 60.0)))
            }
            _ if !found
                && matches!(state, State::Thinking | State::Listening | State::Speaking) =>
            {
                Some(Duration::from_millis(45))
            }
            _ => None,
        };
        if let Some(delay) = [transition, next_frame, animation]
            .into_iter()
            .flatten()
            .min()
        {
            ui.ctx()
                .request_repaint_after(delay.max(Duration::from_millis(8)));
        }
    }
}

fn decode(
    directory: &Path,
    path: &Path,
    expected: Option<(u32, u32)>,
) -> anyhow::Result<ColorImage> {
    anyhow::ensure!(
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("png")),
        "Ожидается PNG"
    );
    let directory = directory.canonicalize()?;
    let file = directory.join(path).canonicalize()?;
    anyhow::ensure!(
        file.starts_with(directory),
        "Кадр должен находиться в каталоге аватара"
    );
    let mut reader = image::ImageReader::with_format(
        std::io::BufReader::new(std::fs::File::open(file)?),
        image::ImageFormat::Png,
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(80 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?.into_rgba8();
    anyhow::ensure!(
        expected.is_none_or(|dimensions| dimensions == image.dimensions()),
        "Размер кадра отличается от холста манифеста"
    );
    let size = [image.width() as usize, image.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, image.as_raw()))
}

pub fn window_icon(root: &Path) -> Option<egui::IconData> {
    for relative in [
        "assets/avatar/mini_128.png",
        "app/assets/avatar/mini_128.png",
    ] {
        if let Ok(png) = image::open(root.join(relative)) {
            let png = png.into_rgba8();
            return Some(egui::IconData {
                width: png.width(),
                height: png.height(),
                rgba: png.into_raw(),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supplied_pngs_match_the_canvas_and_reject_escaping_paths() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/avatar");
        if !root.join("idle.png").is_file() {
            return; // Distributed source can start with the placeholder.
        }
        let manifest: Manifest =
            serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
        let canvas = manifest.canvas.unwrap();
        for clip in manifest.states.values() {
            let path = clip.frame(Duration::ZERO, 0.0).unwrap();
            let frame = decode(&root, path, Some((canvas.width, canvas.height))).unwrap();
            assert_eq!(frame.size, [canvas.width as usize, canvas.height as usize]);
        }
        assert!(decode(&root, Path::new("idle.png"), Some((1, 1))).is_err());
        assert!(
            decode(
                &root,
                &root.join("../avatar/../references/expression_source_sheet.png"),
                None
            )
            .is_err()
        );
    }
}
