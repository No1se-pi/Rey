#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod audio;
mod avatar_ui;
mod games_ui;
mod theme;

use anyhow::{Context, Result};
use eframe::egui::{self, Align, Color32, Layout, RichText};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use rey_core::{
    agent::{Agent, Reply, Turn},
    avatar::{AvatarController, State},
    config::{Config, ProviderKind, endpoint, is_local},
    games::{GameSnapshot, Kind, engine},
    memory::{Message, Note, Session, copy_database},
    voice::Voice,
};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};
use tokio::{runtime::Runtime, task::JoinHandle};
use uuid::Uuid;

use crate::{
    audio::{Capture, Pcm, Playback},
    avatar_ui::AvatarArt,
    games_ui::{GameAction, GameRoom, RoomView},
    theme::*,
};

fn main() -> Result<()> {
    let result = launch();
    #[cfg(target_os = "windows")]
    if let Err(error) = &result {
        rfd::MessageDialog::new()
            .set_title("Rey Desktop")
            .set_level(rfd::MessageLevel::Error)
            .set_description(format!("{error:#}"))
            .show();
    }
    result
}

fn launch() -> Result<()> {
    rey_core::init_logging();
    let mut args = std::env::args_os().skip(1);
    let root = match args.next() {
        Some(flag) if flag == "--root" => {
            PathBuf::from(args.next().context("Укажи путь после --root")?)
        }
        Some(_) => anyhow::bail!("Использование: rey-desktop [--root путь]"),
        None => std::env::current_dir()?,
    };
    anyhow::ensure!(args.next().is_none(), "Лишние аргументы запуска");
    let config = Config::load(root)?;
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1260.0, 790.0])
        .with_min_inner_size([650.0, 520.0]);
    if let Some(icon) = avatar_ui::window_icon(&config.root) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let agent = Agent::open(config)?;
    let runtime = Runtime::new()?;
    eframe::run_native(
        "Rey · Desktop",
        options,
        Box::new(move |context| {
            theme::apply(&context.egui_ctx);
            Ok(Box::new(Desktop::new(
                agent,
                runtime,
                context.egui_ctx.clone(),
            )))
        }),
    )
    .map_err(|error| anyhow::anyhow!("Не удалось запустить окно Rey: {error}"))?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Chat,
    Games,
    Memory,
    Skills,
    Settings,
    Status,
}

impl Page {
    const ALL: [Self; 6] = [
        Self::Chat,
        Self::Games,
        Self::Memory,
        Self::Skills,
        Self::Settings,
        Self::Status,
    ];
    fn title(self) -> &'static str {
        match self {
            Self::Chat => "Диалог",
            Self::Games => "Игры",
            Self::Memory => "Память",
            Self::Skills => "Навыки",
            Self::Settings => "Настройки",
            Self::Status => "Система",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Chat => "◈",
            Self::Games => "♘",
            Self::Memory => "▤",
            Self::Skills => "◇",
            Self::Settings => "⚙",
            Self::Status => "◉",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Channel {
    Chat,
    Game,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Reading,
    Generating,
    Synthesizing,
    Playing,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Self::Reading => "Распознаёт речь",
            Self::Generating => "Готовит ответ",
            Self::Synthesizing => "Генерирует голос",
            Self::Playing => "Воспроизводит ответ",
        }
    }
}

enum Input {
    Text(String),
    File(PathBuf),
    Recording(Vec<i16>),
}

enum Event {
    Phase(Uuid, Phase),
    Transcript(Uuid, String),
    Reply(Uuid, String, Reply),
    Voice(Uuid, Pcm, Option<PathBuf>),
    Finished(Uuid),
    Fault(Uuid, String),
    Models(u32, Result<Vec<String>>),
    Engine(Uuid, Uuid, Result<engine::Analysis>),
    SavedGame(Uuid, Result<()>),
    Backup(Result<PathBuf>),
}

struct Pending {
    id: Uuid,
    channel: Channel,
    phase: Phase,
    job: JoinHandle<()>,
    input: String,
    emotion: Option<String>,
    quiet: bool,
}

struct EngineJob {
    id: Uuid,
    task: JoinHandle<()>,
    play: bool,
}

struct NoteEditor {
    id: Option<i64>,
    title: String,
    markdown: String,
}

struct Desktop {
    agent: Agent,
    runtime: Runtime,
    ctx: egui::Context,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    page: Page,
    avatar: AvatarController,
    art: AvatarArt,
    markdown: CommonMarkCache,
    session: String,
    sessions: Vec<Session>,
    messages: Vec<Message>,
    draft: String,
    pending: Option<Pending>,
    recording: Option<Capture>,
    playback: Option<Playback>,
    envelope_at: Instant,
    voice: Voice,
    voice_enabled: bool,
    model: String,
    model_choices: Vec<String>,
    model_request: u32,
    provider_status: Option<String>,
    host_input: String,
    chess_input: String,
    shogi_input: String,
    memory_search: String,
    notes: Vec<Note>,
    note: Option<NoteEditor>,
    needs_note_refresh: bool,
    game: GameRoom,
    engine_job: Option<EngineJob>,
    toast: Option<String>,
    error: Option<String>,
    confirm_delete: Option<(String, String)>,
}

impl Desktop {
    fn new(agent: Agent, runtime: Runtime, ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let config = &agent.config;
        let sessions = agent.store.sessions().unwrap_or_default();
        let session = sessions
            .first()
            .map(|s| s.id.clone())
            .unwrap_or_else(new_id);
        let messages = agent.store.history(&session, 250).unwrap_or_default();
        let mut desktop = Self {
            art: AvatarArt::load(&config.root, &ctx),
            voice: Voice::new(config.clone()),
            voice_enabled: config.voice_enabled,
            model: config.model().to_owned(),
            host_input: config.provider_url().to_string(),
            chess_input: config.chess_engine.to_string_lossy().into(),
            shogi_input: config.shogi_engine.to_string_lossy().into(),
            agent,
            runtime,
            ctx,
            tx,
            rx,
            page: Page::Chat,
            avatar: AvatarController::default(),
            markdown: CommonMarkCache::default(),
            session,
            sessions,
            messages,
            draft: String::new(),
            pending: None,
            recording: None,
            playback: None,
            envelope_at: Instant::now(),
            model_choices: Vec::new(),
            model_request: 0,
            provider_status: None,
            memory_search: String::new(),
            notes: Vec::new(),
            note: None,
            needs_note_refresh: true,
            game: GameRoom::default(),
            engine_job: None,
            toast: None,
            error: None,
            confirm_delete: None,
        };
        desktop.refresh_models();
        desktop
    }

    fn error(&mut self, error: impl std::fmt::Display) {
        self.error = Some(error.to_string());
        self.avatar.set(State::Error);
    }

    fn refresh_models(&mut self) {
        self.model_request = self.model_request.wrapping_add(1);
        self.provider_status = None;
        let request = self.model_request;
        let result = self.agent.provider();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            let result = match result {
                Ok(provider) => provider.models().await,
                Err(error) => Err(error),
            };
            report(&tx, &ctx, Event::Models(request, result));
        });
    }

    fn select_chat(&mut self, id: String) {
        if self.pending.is_some() || self.recording.is_some() {
            return;
        }
        match self.agent.store.history(&id, 250) {
            Ok(messages) => {
                self.messages = messages;
                self.session = id;
                self.draft.clear();
            }
            Err(error) => self.error(error),
        }
    }

    fn start(&mut self, channel: Channel, input: Input, quiet: bool) {
        if self.pending.is_some() || self.recording.is_some() {
            return;
        }
        let initial = match &input {
            Input::Text(text) => text.trim().to_owned(),
            _ => String::new(),
        };
        if matches!(input, Input::Text(_)) && initial.is_empty() {
            return;
        }
        let game_context = self.game.game.as_ref().and_then(|game| {
            (channel == Channel::Game
                || !matches!(&input, Input::Text(text) if !GameSnapshot::concerns_game(text)))
            .then(|| game.snapshot())
        });
        let session = if channel == Channel::Game {
            self.game.chat_id.clone()
        } else {
            self.session.clone()
        };
        let phase = if matches!(input, Input::Text(_)) {
            Phase::Generating
        } else {
            Phase::Reading
        };
        let id = Uuid::new_v4();
        let request = VoiceRequest {
            id,
            input,
            session,
            game_context,
            game_chat: channel == Channel::Game,
            model: self.model.clone(),
            spoken: self.voice_enabled && !quiet,
            agent: self.agent.clone(),
            voice: self.voice.clone(),
            tx: self.tx.clone(),
            ctx: self.ctx.clone(),
        };
        let job = self.runtime.spawn(async move {
            if let Err(error) = request.run().await {
                report(
                    &request.tx,
                    &request.ctx,
                    Event::Fault(id, error.to_string()),
                );
            }
        });
        self.error = None;
        self.pending = Some(Pending {
            id,
            channel,
            phase,
            job,
            input: initial,
            emotion: None,
            quiet,
        });
        self.avatar.set(State::Thinking);
    }

    fn cancel(&mut self, recover_text: bool) {
        self.recording = None;
        self.playback = None;
        if let Some(pending) = self.pending.take() {
            pending.job.abort();
            if pending.emotion.is_none() {
                if recover_text {
                    self.recover_draft(&pending);
                }
                let session = if pending.channel == Channel::Game {
                    &self.game.chat_id
                } else {
                    &self.session
                };
                if let Ok(messages) = self.agent.store.history(session, 250) {
                    match pending.channel {
                        Channel::Chat => self.messages = messages,
                        Channel::Game => self.game.messages = messages,
                    }
                }
            }
        }
        self.avatar.set(State::Idle);
    }

    fn recover_draft(&mut self, pending: &Pending) {
        if pending.quiet || pending.input.is_empty() {
            return;
        }
        let draft = if pending.channel == Channel::Game {
            &mut self.game.draft
        } else {
            &mut self.draft
        };
        if draft.is_empty() {
            draft.clone_from(&pending.input);
        }
    }

    fn record_button(&mut self) {
        if let Some(recording) = self.recording.take() {
            match recording.stop() {
                Ok(samples) => self.start(Channel::Chat, Input::Recording(samples), false),
                Err(error) => self.error(error),
            }
        } else {
            match Capture::start(
                self.agent.config.max_audio_seconds,
                self.agent.config.max_audio_bytes,
            ) {
                Ok(recording) => {
                    self.recording = Some(recording);
                    self.avatar.set(State::Listening);
                }
                Err(error) => self.error(error),
            }
        }
    }

    fn poll(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Phase(id, phase) if self.active(id) => {
                    if let Some(pending) = &mut self.pending {
                        pending.phase = phase;
                    }
                    self.avatar.set(State::Thinking);
                }
                Event::Transcript(id, text) if self.active(id) => {
                    if let Some(pending) = &mut self.pending {
                        pending.input = text;
                    }
                }
                Event::Reply(id, text, reply) if self.active(id) => {
                    if let Some(pending) = &mut self.pending {
                        let messages = match pending.channel {
                            Channel::Chat => &mut self.messages,
                            Channel::Game => &mut self.game.messages,
                        };
                        messages.push(Message::new("user", text));
                        messages.push(Message::new("assistant", reply.text));
                        pending.emotion = Some(reply.emotion.clone());
                        self.avatar.reply(&reply.emotion);
                        self.needs_note_refresh = true;
                        if pending.channel == Channel::Chat
                            && let Ok(sessions) = self.agent.store.sessions()
                        {
                            self.sessions = sessions;
                        }
                    }
                }
                Event::Voice(id, pcm, archive) if self.active(id) => {
                    if let Some(path) = archive {
                        self.toast = Some(format!("Аудио сохранено · {}", path.display()));
                    }
                    match Playback::start(pcm) {
                        Ok(playback) => {
                            self.playback = Some(playback);
                            if let Some(pending) = &mut self.pending {
                                pending.phase = Phase::Playing;
                            }
                            self.avatar.set(State::Speaking);
                        }
                        Err(error) => {
                            self.pending = None;
                            self.error(error);
                        }
                    }
                }
                Event::Finished(id) if self.active(id) => {
                    self.pending = None;
                }
                Event::Fault(id, error) if self.active(id) => {
                    if let Some(pending) = self.pending.take()
                        && pending.emotion.is_none()
                    {
                        self.recover_draft(&pending);
                    }
                    self.error(error);
                }
                Event::Models(sequence, result) if self.model_request == sequence => match result {
                    Ok(models) => {
                        self.provider_status =
                            Some(format!("Подключена · моделей: {}", models.len()));
                        if self.model.trim().is_empty()
                            && let Some(model) = models.first()
                        {
                            self.model.clone_from(model);
                        }
                        self.model_choices = models;
                    }
                    Err(error) => {
                        self.model_choices.clear();
                        self.provider_status = Some(format!("Недоступен · {error}"));
                    }
                },
                Event::Engine(id, game_id, result) => self.finish_engine(id, game_id, result),
                Event::SavedGame(game_id, result) => {
                    if self
                        .game
                        .game
                        .as_ref()
                        .is_some_and(|game| game.id == game_id)
                    {
                        self.game.save_pending = false;
                        match result {
                            Ok(()) => {
                                self.game.saved = true;
                                self.game.refresh_history = true;
                                self.game.save_error = None;
                            }
                            Err(error) => self.game.save_error = Some(error.to_string()),
                        }
                    }
                }
                Event::Backup(result) => match result {
                    Ok(path) => self.toast = Some(format!("Резервная копия · {}", path.display())),
                    Err(error) => self.error(error),
                },
                _ => (),
            }
        }
        if let Some(playback) = &self.playback {
            if playback.done() {
                self.playback = None;
                if let Some(pending) = self.pending.take() {
                    self.avatar
                        .reply(pending.emotion.as_deref().unwrap_or_default());
                }
            } else {
                if self.envelope_at.elapsed() >= self.art.envelope_interval() {
                    self.avatar.amplitude = playback.amplitude();
                    self.envelope_at = Instant::now();
                }
                self.ctx.request_repaint_after(self.art.envelope_interval());
            }
        }
        if let Some(recording) = &self.recording {
            if recording.seconds() >= u64::from(recording.max_seconds) {
                self.record_button();
            } else {
                self.ctx.request_repaint_after(Duration::from_millis(250));
            }
        }
        self.tick_game();
    }

    fn active(&self, id: Uuid) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.id == id)
    }

    fn page(&mut self, page: Page) {
        self.page = page;
        self.toast = None;
    }

    fn nav(&mut self, ui: &mut egui::Ui, horizontal: bool) {
        for page in Page::ALL {
            let selected = self.page == page;
            let label = if horizontal {
                page.title().to_string()
            } else {
                format!("{}    {}", page.icon(), page.title())
            };
            let button = egui::Button::new(
                RichText::new(label)
                    .color(if selected { TEXT } else { MUTED })
                    .strong(),
            )
            .fill(if selected {
                RAISED
            } else {
                Color32::TRANSPARENT
            })
            .stroke(egui::Stroke::NONE)
            .corner_radius(9)
            .min_size(egui::vec2(if horizontal { 0.0 } else { 157.0 }, 35.0));
            if ui.add(button).clicked() {
                self.page(page);
            }
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("◈").size(27.0).color(ACCENT));
            ui.label(RichText::new("Rey").size(26.0).strong());
        });
        ui.add_space(17.0);
        self.nav(ui, false);
        if self.page == Page::Chat {
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Диалоги").color(MUTED).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            self.pending.is_none() && self.recording.is_none(),
                            egui::Button::new("+"),
                        )
                        .on_hover_text("Новый диалог")
                        .clicked()
                    {
                        self.select_chat(new_id());
                    }
                });
            });
            egui::ScrollArea::vertical()
                .id_salt("sessions")
                .max_height((ui.available_height() - 44.0).max(50.0))
                .show(ui, |ui| {
                    let mut selection = None;
                    for session in &self.sessions {
                        let title = summary(&session.title, 25);
                        if ui
                            .add_enabled(
                                self.pending.is_none() && self.recording.is_none(),
                                egui::Button::selectable(self.session == session.id, title),
                            )
                            .clicked()
                        {
                            selection = Some(session.id.clone());
                        }
                    }
                    if let Some(id) = selection {
                        self.select_chat(id);
                    }
                });
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.label(RichText::new("Ghost & Shell").color(MUTED).small());
        });
    }

    fn show_chat(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Диалог");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_enabled_ui(self.pending.is_none() && self.recording.is_none(), |ui| {
                    if ui.button("+ Новый").clicked() {
                        self.select_chat(new_id());
                    }
                    if self
                        .sessions
                        .iter()
                        .any(|session| session.id == self.session)
                        && ui.button("Удалить").clicked()
                    {
                        self.confirm_delete =
                            Some((self.session.clone(), "Удалить этот диалог?".into()));
                    }
                    egui::ComboBox::from_id_salt("model_chat")
                        .width(125.0)
                        .selected_text(summary(&self.model, 23))
                        .show_ui(ui, |ui| {
                            for model in &self.model_choices {
                                ui.selectable_value(&mut self.model, model.clone(), model);
                            }
                        });
                });
            });
        });
        if ui.available_width() < 640.0 && ui.ctx().screen_rect().width() < 1030.0 {
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.avatar.snapshot().0.label()).color(MINT));
                let mut selection = self.session.clone();
                egui::ComboBox::from_id_salt("small_history")
                    .selected_text("История диалогов")
                    .show_ui(ui, |ui| {
                        for session in &self.sessions {
                            ui.selectable_value(
                                &mut selection,
                                session.id.clone(),
                                summary(&session.title, 30),
                            );
                        }
                    });
                if selection != self.session {
                    self.select_chat(selection);
                }
            });
        }
        if let Some(game) = &self.game.game {
            let status = format!("{} · {}", game.kind.name(), game.turn_status());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(status).color(MINT));
                if ui.button("К доске").clicked() {
                    self.game.show_board();
                    self.page(Page::Games);
                }
            });
        }
        ui.add_space(9.0);
        let pending = self
            .pending
            .as_ref()
            .filter(|pending| pending.channel == Channel::Chat && pending.emotion.is_none())
            .map(|pending| pending.input.as_str())
            .filter(|text| !text.is_empty());
        let empty = self.messages.is_empty() && pending.is_none();
        let message_height = (ui.available_height() - 115.0).max(90.0);
        card(SIDE).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt(("chat", self.session.clone()))
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .max_height((message_height - 30.0).max(35.0))
                .show(ui, |ui| {
                    if empty {
                        ui.add_space((message_height * 0.29).max(6.0));
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("◇").size(40.0).color(ACCENT));
                            ui.heading("Я на связи.");
                            ui.label(RichText::new("О чём поговорим?").color(MUTED));
                        });
                    } else {
                        show_messages(ui, &self.messages, pending, &mut self.markdown);
                    }
                });
        });
        ui.add_space(4.0);
        if let Some(recording) = &self.recording {
            ui.colored_label(
                MINT,
                format!(
                    "● Запись · {:02}:{:02}",
                    recording.seconds() / 60,
                    recording.seconds() % 60
                ),
            );
        } else if let Some(pending) = &self.pending {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(pending.phase.label()).color(MUTED));
            });
        } else {
            ui.add_space(15.0);
        }
        ui.horizontal(|ui| {
            let ready = self.pending.is_none() && self.recording.is_none();
            let field = ui.add_enabled(
                ready,
                egui::TextEdit::singleline(&mut self.draft)
                    .hint_text("Написать Рей…")
                    .desired_width((ui.available_width() - 136.0).max(120.0))
                    .char_limit(4000)
                    .margin(egui::Margin::same(10)),
            );
            let enter = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if self.pending.is_some() {
                if ui.button("Стоп").clicked() {
                    self.cancel(true);
                }
            } else {
                let send = ready && !self.draft.trim().is_empty();
                let click = ui.add_enabled(send, primary("Отправить")).clicked();
                if send && (click || enter) {
                    let draft = std::mem::take(&mut self.draft);
                    self.start(Channel::Chat, Input::Text(draft), false);
                    field.request_focus();
                }
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.pending.is_none(),
                    egui::Button::new(if self.recording.is_some() {
                        "Закончить запись"
                    } else {
                        "Микрофон"
                    }),
                )
                .clicked()
            {
                self.record_button();
            }
            if ui
                .add_enabled(
                    self.pending.is_none() && self.recording.is_none(),
                    egui::Button::new("Аудиофайл"),
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter(
                        "Аудио",
                        &["wav", "mp3", "ogg", "opus", "m4a", "aac", "flac", "webm"],
                    )
                    .pick_file()
            {
                self.start(Channel::Chat, Input::File(path), false);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.checkbox(&mut self.voice_enabled, "Озвучивать");
            });
        });
    }

    fn show_memory(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Память");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(primary("+ Заметка")).clicked() {
                    self.note = Some(NoteEditor {
                        id: None,
                        title: String::new(),
                        markdown: String::new(),
                    });
                }
            });
        });
        let changed = ui
            .add(
                egui::TextEdit::singleline(&mut self.memory_search)
                    .hint_text("Поиск по заметкам")
                    .desired_width(f32::INFINITY),
            )
            .changed();
        if changed || self.needs_note_refresh {
            self.needs_note_refresh = false;
            match self.agent.store.notes(&self.memory_search, 100) {
                Ok(notes) => self.notes = notes,
                Err(error) => self.error(error),
            }
        }
        if let Some(note) = &mut self.note {
            let mut close = false;
            let mut id_to_delete = None;
            let mut fault = None;
            card(PANEL).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut note.title)
                        .hint_text("Название")
                        .desired_width(f32::INFINITY)
                        .char_limit(200),
                );
                ui.add(
                    egui::TextEdit::multiline(&mut note.markdown)
                        .hint_text("Заметка в Markdown · [[123]] открывает связанную заметку")
                        .desired_width(f32::INFINITY)
                        .desired_rows(7),
                );
                ui.horizontal(|ui| {
                    if ui.add(primary("Сохранить")).clicked() {
                        match self
                            .agent
                            .store
                            .save_note(note.id, &note.title, &note.markdown)
                        {
                            Ok(_) => close = true,
                            Err(error) => fault = Some(error.to_string()),
                        }
                    }
                    if ui.button("Закрыть").clicked() {
                        close = true;
                    }
                    if let Some(id) = note.id
                        && ui.button("Удалить заметку").clicked()
                    {
                        id_to_delete = Some(id);
                    }
                });
            });
            if let Some(error) = fault {
                self.error(error);
            }
            if let Some(id) = id_to_delete {
                self.confirm_delete = Some((format!("note:{id}"), "Удалить эту заметку?".into()));
            }
            if close {
                self.note = None;
                self.needs_note_refresh = true;
            }
        }
        if self.notes.is_empty() {
            ui.label(RichText::new("Заметок по этому запросу нет.").color(MUTED));
        }
        let mut next = None;
        let mut linked = None;
        for note in &self.notes {
            ui.push_id(note.id, |ui| {
                card(SIDE).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&note.title).strong());
                        ui.label(RichText::new(format!("#{}", note.id)).color(MUTED).small());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Править").clicked() {
                                next = Some(NoteEditor {
                                    id: Some(note.id),
                                    title: note.title.clone(),
                                    markdown: note.markdown.clone(),
                                });
                            }
                        });
                    });
                    let visible: String = note.markdown.chars().take(5000).collect();
                    CommonMarkViewer::new().show(ui, &mut self.markdown, &visible);
                    for (token, _) in visible
                        .split("[[")
                        .skip(1)
                        .filter_map(|part| part.split_once("]]"))
                    {
                        if let Ok(id) = token.parse::<i64>()
                            && ui.link(format!("Заметка #{id}")).clicked()
                        {
                            linked = Some(id);
                        }
                    }
                });
            });
        }
        if let Some(id) = linked {
            match self.agent.store.note(id) {
                Ok(note) => {
                    next = Some(NoteEditor {
                        id: Some(note.id),
                        title: note.title,
                        markdown: note.markdown,
                    })
                }
                Err(error) => self.error(error),
            }
        }
        if let Some(note) = next {
            self.note = Some(note);
        }
    }

    fn show_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Настройки");
        card(PANEL).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new("Модель и подключение").strong());
            let mut provider = self.agent.config.provider;
            egui::ComboBox::from_id_salt("provider")
                .selected_text(provider.title())
                .show_ui(ui, |ui| {
                    for choice in [ProviderKind::Ollama, ProviderKind::LlamaCpp] {
                        ui.selectable_value(&mut provider, choice, choice.title());
                    }
                });
            if provider != self.agent.config.provider {
                self.agent.config.set_model(self.model.clone());
                self.agent.config.provider = provider;
                self.host_input = self.agent.config.provider_url().to_string();
                self.model = self.agent.config.model().to_owned();
                self.model_choices.clear();
                self.error = None;
                self.refresh_models();
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.host_input)
                    .hint_text("http://127.0.0.1:11434")
                    .desired_width(f32::INFINITY),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.model)
                    .hint_text("Название модели")
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal_wrapped(|ui| {
                if ui.add(primary("Применить подключение")).clicked() {
                    match endpoint(&self.host_input) {
                        Ok(url) => {
                            self.agent.config.set_provider_url(url);
                            self.refresh_models();
                            self.toast = Some("Подключение обновлено".into());
                        }
                        Err(error) => self.error(error),
                    }
                }
                egui::ComboBox::from_id_salt("settings_model")
                    .selected_text("Доступные модели")
                    .show_ui(ui, |ui| {
                        for model in &self.model_choices {
                            ui.selectable_value(&mut self.model, model.clone(), model);
                        }
                    });
            });
            if !is_local(self.agent.config.provider_url()) {
                ui.colored_label(
                    MUTED,
                    if self.agent.config.remote_memory {
                        "Удалённый провайдер · доступ к истории и памяти включён"
                    } else {
                        "Удалённый провайдер · доступ к истории и памяти закрыт"
                    },
                );
            }
        });
        card(PANEL).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new("Голос").strong());
            ui.checkbox(&mut self.voice_enabled, "Озвучивать ответы");
            ui.label(
                RichText::new(if self.agent.config.voice_url.is_some() {
                    "Удалённый worker"
                } else {
                    "Локальный Faster‑Whisper и Qwen3‑TTS"
                })
                .color(MUTED),
            );
        });
        card(PANEL).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new("Игровые движки").strong());
            ui.label("Stockfish · UCI");
            ui.add(egui::TextEdit::singleline(&mut self.chess_input).desired_width(f32::INFINITY));
            ui.label("YaneuraOu · USI");
            ui.add(egui::TextEdit::singleline(&mut self.shogi_input).desired_width(f32::INFINITY));
            if ui.add(primary("Применить пути")).clicked() {
                let resolve = |text: &str| {
                    let path = PathBuf::from(text.trim());
                    if path.components().count() > 1 || self.agent.config.root.join(&path).is_file()
                    {
                        self.agent.config.root.join(&path)
                    } else {
                        path
                    }
                };
                self.agent.config.chess_engine = resolve(&self.chess_input);
                self.agent.config.shogi_engine = resolve(&self.shogi_input);
                self.game.request_failed = false;
                self.game.engine_note = None;
                self.toast = Some("Пути к движкам обновлены".into());
            }
        });
        if ui.button("Перезагрузить ассеты аватара").clicked() {
            self.art = AvatarArt::load(&self.agent.config.root, &self.ctx);
        }
    }

    fn show_status(&mut self, ui: &mut egui::Ui) {
        ui.heading("Система");
        card(PANEL).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(self.agent.config.provider.title()).strong());
            ui.label(self.agent.config.provider_url().as_str());
            ui.label(
                self.provider_status
                    .as_deref()
                    .unwrap_or("Проверяем подключение…"),
            );
            if ui.button("Проверить").clicked() {
                self.refresh_models();
            }
        });
        card(PANEL).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new("Ghost").strong());
            ui.label(format!(
                "{} · характер v{}",
                self.agent.ghost.soul.name, self.agent.ghost.character.version
            ));
            ui.label(RichText::new(&self.agent.ghost.digest).monospace().small());
            ui.label(self.agent.store.path().display().to_string());
            if ui.button("Резервная копия SQLite").clicked()
                && let Some(destination) = rfd::FileDialog::new()
                    .set_file_name("rey-memory.db")
                    .save_file()
            {
                let source = self.agent.store.path().to_path_buf();
                let tx = self.tx.clone();
                let ctx = self.ctx.clone();
                self.runtime.spawn_blocking(move || {
                    let result = copy_database(&source, &destination, false).map(|()| destination);
                    report(&tx, &ctx, Event::Backup(result));
                });
            }
        });
        card(PANEL).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new("Аватар").strong());
            ui.label(self.avatar.snapshot().0.label());
            if let Some(error) = &self.art.error {
                ui.colored_label(ERROR, error);
            }
        });
    }

    fn show_skills(ui: &mut egui::Ui) {
        ui.heading("Навыки");
        for (title, body) in [
            (
                "Альманах",
                "Чтение разделов о памяти, навыках, сценариях и среде.",
            ),
            (
                "Память",
                "Поиск и чтение Markdown-заметок, просмотр завершённых партий.",
            ),
            (
                "Шахматы и сёги",
                "Разбор позиции и доступных ходов с данными UCI и USI.",
            ),
            ("Голос", "Распознавание русской речи и озвучивание ответов."),
        ] {
            card(SIDE).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new(title).strong());
                ui.label(RichText::new(body).color(MUTED));
            });
        }
    }

    fn confirmation(&mut self, ctx: &egui::Context) {
        let Some((id, title)) = self.confirm_delete.clone() else {
            return;
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.add(primary("Удалить")).clicked() {
                        let result = if let Some(note) = id
                            .strip_prefix("note:")
                            .and_then(|value| value.parse().ok())
                        {
                            self.agent.store.delete_note(note)
                        } else {
                            self.agent.store.delete_session(&id)
                        };
                        match result {
                            Ok(()) => {
                                self.confirm_delete = None;
                                if id.starts_with("note:") {
                                    self.note = None;
                                    self.needs_note_refresh = true;
                                } else {
                                    self.sessions.retain(|session| session.id != id);
                                    self.select_chat(
                                        self.sessions
                                            .first()
                                            .map(|session| session.id.clone())
                                            .unwrap_or_else(new_id),
                                    );
                                }
                            }
                            Err(error) => self.error(error),
                        }
                    }
                    if ui.button("Отмена").clicked() {
                        self.confirm_delete = None;
                    }
                });
            });
    }

    fn show_games(&mut self, ui: &mut egui::Ui) {
        if self.game.refresh_history {
            self.game.refresh_history = false;
            match self.agent.store.games(30) {
                Ok(history) => self.game.history = history,
                Err(error) => self.error(error),
            }
        }
        let pending = self
            .pending
            .as_ref()
            .filter(|pending| pending.channel == Channel::Game);
        let mut view = RoomView {
            controller: &self.avatar,
            avatar: &mut self.art,
            engine_busy: self.engine_job.is_some(),
            chat_busy: self.pending.is_some() || self.recording.is_some(),
            chat_text: pending
                .filter(|pending| pending.emotion.is_none())
                .map(|pending| pending.input.as_str()),
            chat_phase: pending.map(|pending| pending.phase.label()),
        };
        if let Some(action) = self.game.show(ui, &mut self.markdown, &mut view)
            && let Err(error) = self.game_action(action)
        {
            self.error(error);
        }
    }

    fn game_action(&mut self, action: GameAction) -> Result<()> {
        match action {
            GameAction::New => {
                if self.game.mode != games_ui::Mode::Analysis {
                    engine::check_executable(self.game_engine(self.game.kind))?;
                }
                self.game.new_game()?;
                if let Some(job) = self.engine_job.take() {
                    job.task.abort();
                }
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.channel == Channel::Game)
                {
                    self.cancel(false);
                }
                self.game.active_mode = self.game.mode;
                if self.pending.is_none() && self.recording.is_none() {
                    self.avatar.set(State::Idle);
                }
            }
            GameAction::Move(movement) => {
                if self.engine_job.as_ref().is_some_and(|job| !job.play)
                    && let Some(job) = self.engine_job.take()
                {
                    job.task.abort();
                }
                let game = self.game.game.as_mut().context("Нет партии")?;
                if !game.study && game.side() != game.user {
                    return Ok(());
                }
                game.play(&movement)?;
                if game.outcome.is_none() && self.pending.is_none() && self.recording.is_none() {
                    self.avatar.set(if game.checked_king().is_some() {
                        State::Surprised
                    } else if game.study {
                        State::Idle
                    } else {
                        State::Thinking
                    });
                }
                self.game.moved();
            }
            GameAction::Hint => self.start_engine(false),
            GameAction::RetryEngine => self.start_engine(true),
            GameAction::Ask(text) => self.start(Channel::Game, Input::Text(text), false),
            GameAction::CancelChat => self.cancel(true),
            GameAction::Retry => self.game.save_error = None,
            action => {
                let game = self.game.game.as_mut().context("Нет партии")?;
                match action {
                    GameAction::Resign => game.resign(game.user),
                    GameAction::Draw => {
                        if game.study
                            || game.can_claim_draw(None)
                            || game
                                .analysis
                                .as_deref()
                                .is_some_and(|text| text.contains("≈"))
                        {
                            game.agreed_draw();
                        } else {
                            self.game.hint = Some("Рей пока продолжает партию.".into());
                        }
                    }
                    GameAction::Claim(movement) => game.claim_draw(movement.as_deref())?,
                    GameAction::Declare => game.declare()?,
                    GameAction::Impasse => game.impasse()?,
                    GameAction::Export(path) => {
                        std::fs::write(&path, game.export())?;
                        self.toast = Some(format!("Партия экспортирована · {}", path.display()));
                    }
                    _ => unreachable!("action handled above"),
                }
            }
        }
        Ok(())
    }

    fn game_engine(&self, kind: Kind) -> &std::path::Path {
        match kind {
            Kind::Chess => &self.agent.config.chess_engine,
            Kind::Shogi => &self.agent.config.shogi_engine,
        }
    }

    fn start_engine(&mut self, play: bool) {
        if self.engine_job.is_some() {
            return;
        }
        let Some(game) = self.game.game.as_mut() else {
            return;
        };
        if play {
            game.resume_clock();
        }
        let game_id = game.id;
        let id = Uuid::new_v4();
        let kind = game.kind;
        let depth = game.depth;
        let position = game.engine_position();
        let executable = self.game_engine(kind).to_path_buf();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let task = self.runtime.spawn(async move {
            let result = engine::analyze(&executable, kind, &position, depth).await;
            report(&tx, &ctx, Event::Engine(id, game_id, result));
        });
        self.engine_job = Some(EngineJob { id, task, play });
        if play {
            self.game.request_failed = false;
        }
        self.game.engine_note = None;
        if self.pending.is_none() && self.recording.is_none() {
            self.avatar.set(State::Thinking);
        }
    }

    fn finish_engine(&mut self, id: Uuid, game_id: Uuid, result: Result<engine::Analysis>) {
        if self.engine_job.as_ref().is_none_or(|job| job.id != id) {
            return;
        }
        let job = self.engine_job.take().expect("checked job");
        let Some(game) = self.game.game.as_mut().filter(|game| game.id == game_id) else {
            return;
        };
        let result = result.and_then(|analysis| {
            let mut automatic = false;
            if analysis.best == "win" {
                anyhow::ensure!(
                    game.can_declare(),
                    "Движок заявил недоступного входящего короля"
                );
            } else if !matches!(analysis.best.as_str(), "resign" | "0000" | "(none)") {
                anyhow::ensure!(
                    game.outcome.is_some()
                        || game
                            .legal()
                            .iter()
                            .any(|movement| movement.notation == analysis.best),
                    "Движок предложил нелегальный ход"
                );
            } else if job.play && analysis.best != "resign" {
                anyhow::bail!("Движок не выбрал ход");
            }
            let position = game.position();
            let side = game.side();
            if let Some(name) = &analysis.engine {
                game.engine_name = Some(name.clone());
            }
            let score = match analysis.score {
                Some(engine::Score::Centipawns(value)) => {
                    format!(
                        "{}{value:+} cp",
                        if value.unsigned_abs() <= 35 {
                            "≈ "
                        } else {
                            ""
                        }
                    )
                }
                Some(engine::Score::Mate(value)) => format!("мат {value:+}"),
                None => "без оценки".into(),
            };
            let line = game.verified_line(&analysis.pv);
            let detail = format!(
                "{} · {} · глубина {} · {} · PV {}",
                analysis.engine.as_deref().unwrap_or("Движок"),
                side.label(game.kind),
                analysis.depth.unwrap_or(game.depth),
                score,
                if line.is_empty() {
                    "—".into()
                } else {
                    line.join(" ")
                }
            );
            game.analysis = Some(format!("Позиция анализа: {position}. {detail}"));
            if job.play {
                match analysis.best.as_str() {
                    "resign" => game.resign(side),
                    "win" => game.declare()?,
                    movement => game.play(movement)?,
                }
                if self.pending.is_none() && self.recording.is_none() {
                    self.avatar.set(if game.checked_king().is_some() {
                        State::Surprised
                    } else {
                        State::Idle
                    });
                }
                automatic = !self.model_choices.is_empty()
                    && game.outcome.is_none()
                    && match self.game.active_mode {
                        games_ui::Mode::Companion => game.moves.len().is_multiple_of(10),
                        games_ui::Mode::Mentor => {
                            game.checked_king().is_some() || game.moves.len().is_multiple_of(4)
                        }
                        _ => false,
                    };
            } else {
                self.game.hint = Some(format!("Ход {} · {detail}", analysis.best));
                if self.pending.is_none() && self.recording.is_none() {
                    self.avatar.set(State::Idle);
                }
            }
            Ok(automatic)
        });
        match result {
            Ok(automatic) => {
                if job.play {
                    self.game.moved();
                }
                if automatic && self.pending.is_none() && self.recording.is_none() {
                    self.start(
                        Channel::Game,
                        Input::Text(
                            "Коротко прокомментируй последний ход и важный момент позиции.".into(),
                        ),
                        true,
                    );
                }
            }
            Err(error) => {
                if job.play {
                    game.pause_clock();
                    self.game.request_failed = true;
                }
                self.game.engine_note = Some(format!("{error:#}"));
                if self.pending.is_none() && self.recording.is_none() {
                    self.avatar.set(State::Error);
                }
            }
        }
    }

    fn tick_game(&mut self) {
        let Some(game) = self.game.game.as_mut() else {
            return;
        };
        let was_running = game.outcome.is_none();
        game.tick();
        if game.outcome.is_some() {
            if self.engine_job.as_ref().is_some_and(|job| job.play)
                && let Some(job) = self.engine_job.take()
            {
                job.task.abort();
            }
            if was_running
                || !self.game.saved && !self.game.save_pending && self.game.save_error.is_none()
            {
                if self.pending.is_none() && self.recording.is_none() {
                    let winner = game.outcome.as_ref().and_then(|outcome| outcome.winner);
                    self.avatar.set(match winner {
                        Some(side) if side == game.user => State::Happy,
                        Some(_) => State::Sad,
                        None => State::Surprised,
                    });
                }
                if !self.game.saved
                    && !self.game.save_pending
                    && self.game.save_error.is_none()
                    && let Some(record) = game.record()
                {
                    self.game.save_pending = true;
                    let store = self.agent.store.clone();
                    let tx = self.tx.clone();
                    let ctx = self.ctx.clone();
                    self.runtime.spawn_blocking(move || {
                        let result = store.save_game(&record);
                        report(&tx, &ctx, Event::SavedGame(record.id, result));
                    });
                }
            }
        } else {
            self.ctx.request_repaint_after(Duration::from_millis(250));
            if !game.study
                && game.side() != game.user
                && self.engine_job.is_none()
                && !self.game.request_failed
            {
                if game.can_declare() {
                    let _ = game.declare();
                } else if game.can_claim_draw(None) {
                    let _ = game.claim_draw(None);
                } else {
                    self.start_engine(true);
                }
            }
        }
    }

    fn main_panel(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = self.error.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(ERROR, error);
                if ui.button("Закрыть").clicked() {
                    self.error = None;
                }
            });
        }
        if let Some(toast) = &self.toast {
            ui.colored_label(MINT, toast);
        }
        if self.page == Page::Chat {
            self.show_chat(ui);
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("screen")
            .show(ui, |ui| {
                ui.set_min_width((ui.available_width() - 1.0).max(0.0));
                match self.page {
                    Page::Chat => unreachable!(),
                    Page::Games => self.show_games(ui),
                    Page::Memory => self.show_memory(ui),
                    Page::Skills => Self::show_skills(ui),
                    Page::Settings => self.show_settings(ui),
                    Page::Status => self.show_status(ui),
                }
            });
    }
}

impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if (self.pending.is_some() || self.recording.is_some())
            && ctx.input(|input| input.key_pressed(egui::Key::Escape))
        {
            self.cancel(true);
        }
        let width = ctx.screen_rect().width();
        let compact = width < 900.0;
        if compact {
            egui::TopBottomPanel::top("navigation")
                .frame(egui::Frame::new().fill(SIDE).inner_margin(12))
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("◈  Rey").size(20.0).color(ACCENT).strong());
                        self.nav(ui, true);
                    });
                });
        } else {
            egui::SidePanel::left("navigation")
                .exact_width(183.0)
                .resizable(false)
                .frame(egui::Frame::new().fill(SIDE).inner_margin(14))
                .show(ctx, |ui| self.sidebar(ui));
        }
        if self.page == Page::Chat {
            let avatar_width = if compact {
                (width * 0.25).clamp(155.0, 218.0)
            } else if width < 1130.0 {
                240.0
            } else {
                321.0
            };
            egui::SidePanel::right("avatar")
                .exact_width(avatar_width)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(BASE)
                        .inner_margin(if compact { 10 } else { 16 }),
                )
                .show(ctx, |ui| {
                    ui.heading("Рей");
                    ui.add_space(4.0);
                    let height = (ui.available_width() * 1.52)
                        .min(ui.available_height() - 46.0)
                        .max(120.0);
                    self.art.show(ui, &self.avatar, height);
                });
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BASE)
                    .inner_margin(if compact { 12 } else { 20 }),
            )
            .show(ctx, |ui| self.main_panel(ui));
        self.confirmation(ctx);
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        if let Some(pending) = self.pending.take() {
            pending.job.abort();
        }
        if let Some(job) = self.engine_job.take() {
            job.task.abort();
        }
    }
}

fn show_messages(
    ui: &mut egui::Ui,
    messages: &[Message],
    pending: Option<&str>,
    cache: &mut CommonMarkCache,
) {
    for (index, message) in messages.iter().enumerate() {
        ui.push_id(index, |ui| {
            message_card(ui, &message.role, &message.content, cache)
        });
    }
    if let Some(text) = pending {
        message_card(ui, "user", text, cache);
    }
}

fn message_card(ui: &mut egui::Ui, role: &str, text: &str, cache: &mut CommonMarkCache) {
    let user = role == "user";
    let width = ui.available_width().min(670.0);
    if user {
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.vertical(|ui| {
                ui.set_width(width.min(520.0) * 0.93);
                card(RAISED).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(RichText::new("Вы").color(ACCENT).small().strong());
                    ui.add(egui::Label::new(text).selectable(true).wrap());
                });
            });
        });
    } else {
        ui.label(RichText::new("◈  Рей").color(ACCENT).small().strong());
        CommonMarkViewer::new()
            .max_image_width(Some(width as usize))
            .show(ui, cache, text);
    }
    ui.add_space(8.0);
}

fn summary(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or_default();
    let mut chars = line.chars();
    let mut text: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        text.push('…');
    }
    text
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

fn report(tx: &Sender<Event>, ctx: &egui::Context, event: Event) {
    let _ = tx.send(event);
    ctx.request_repaint();
}

struct VoiceRequest {
    id: Uuid,
    input: Input,
    session: String,
    game_context: Option<GameSnapshot>,
    game_chat: bool,
    model: String,
    spoken: bool,
    agent: Agent,
    voice: Voice,
    tx: Sender<Event>,
    ctx: egui::Context,
}

impl VoiceRequest {
    async fn run(&self) -> Result<()> {
        let text = match &self.input {
            Input::Text(text) => text.trim().to_owned(),
            Input::File(path) => self.voice.transcribe(path).await?,
            Input::Recording(samples) => {
                let temp = tempfile::tempdir()?;
                let path = temp.path().join("recording.wav");
                audio::write_wav(&path, samples)?;
                self.voice.transcribe(&path).await?
            }
        };
        if !matches!(self.input, Input::Text(_)) {
            self.emit(Event::Transcript(self.id, text.clone()));
            self.emit(Event::Phase(self.id, Phase::Generating));
        }
        let turn = || Turn {
            session: &self.session,
            model: &self.model,
            text: &text,
            spoken: self.spoken,
            game_context: self
                .game_context
                .as_ref()
                .filter(|_| self.game_chat || GameSnapshot::concerns_game(&text)),
            game_chat: self.game_chat,
        };
        let (reply, provider) = if let Some(reply) = Agent::answer_game_facts(&turn()) {
            (reply, None)
        } else {
            let provider = self.agent.provider()?;
            let reply = self.agent.reply(provider.as_ref(), turn()).await?;
            (reply, Some(provider))
        };
        self.agent.save_reply(&turn(), &reply)?;
        self.emit(Event::Reply(self.id, text, reply.clone()));
        if self.spoken {
            self.emit(Event::Phase(self.id, Phase::Synthesizing));
            if self.agent.config.unload_ollama
                && let Some(provider) = provider
            {
                let _ = provider.unload(&self.model).await;
            }
            let audio = self.voice.synthesize(&reply.text).await?;
            let archive = if self.agent.config.keep_audio {
                Some(audio.archive(&self.agent.config.voice_output)?)
            } else {
                None
            };
            let pcm = audio::read_wav(&audio.wav())?;
            self.emit(Event::Voice(self.id, pcm, archive));
        } else {
            self.emit(Event::Finished(self.id));
        }
        Ok(())
    }

    fn emit(&self, event: Event) {
        report(&self.tx, &self.ctx, event);
    }
}
