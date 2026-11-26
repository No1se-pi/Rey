use crate::{avatar_ui::AvatarArt, show_messages, theme::*};
use eframe::egui::{self, Align, Color32, Layout, Pos2, Rect, RichText, Stroke, Vec2};
use egui_commonmark::CommonMarkCache;
use rey_core::{
    avatar::AvatarController,
    games::{Game, Kind, LegalMove, Origin, PieceView, Side},
    memory::{GameRecord, Message},
};
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Quiet,
    Companion,
    Mentor,
    Analysis,
}

impl Mode {
    pub fn title(self) -> &'static str {
        match self {
            Self::Quiet => "Тихая игра",
            Self::Companion => "Компаньон",
            Self::Mentor => "Наставник",
            Self::Analysis => "Анализ",
        }
    }
}

pub(crate) enum GameAction {
    RetryEngine,
    New,
    Move(String),
    Hint,
    Ask(String),
    CancelChat,
    Resign,
    Claim(Option<String>),
    Draw,
    Declare,
    Impasse,
    Export(PathBuf),
    Retry,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum RoomTab {
    #[default]
    Moves,
    Chat,
    Controls,
}

pub(crate) struct RoomView<'a> {
    pub controller: &'a AvatarController,
    pub avatar: &'a mut AvatarArt,
    pub engine_busy: bool,
    pub chat_busy: bool,
    pub chat_text: Option<&'a str>,
    pub chat_phase: Option<&'a str>,
    pub chat_preview: &'a str,
}

pub(crate) struct GameRoom {
    pub kind: Kind,
    pub mode: Mode,
    pub active_mode: Mode,
    pub user: Side,
    pub minutes: u64,
    pub depth: u8,
    pub position: String,
    pub simple: bool,
    pub game: Option<Game>,
    pub messages: Vec<Message>,
    pub draft: String,
    pub chat_id: String,
    pub request_failed: bool,
    pub saved: bool,
    pub save_pending: bool,
    pub save_error: Option<String>,
    pub history: Vec<GameRecord>,
    pub refresh_history: bool,
    pub hint: Option<String>,
    pub engine_note: Option<String>,
    selection: Option<Origin>,
    choices: Vec<LegalMove>,
    confirm_resign: bool,
    setup_open: bool,
    tab: RoomTab,
    flipped: bool,
}

impl Default for GameRoom {
    fn default() -> Self {
        let simple = cfg!(target_os = "windows")
            && std::env::var_os("WINDIR")
                .is_none_or(|dir| !PathBuf::from(dir).join("Fonts/meiryo.ttc").is_file());
        Self {
            kind: Kind::Chess,
            mode: Mode::Companion,
            active_mode: Mode::Companion,
            user: Side::First,
            minutes: 10,
            depth: 7,
            position: String::new(),
            simple,
            game: None,
            messages: Vec::new(),
            draft: String::new(),
            chat_id: crate::new_id(),
            request_failed: false,
            saved: false,
            save_pending: false,
            save_error: None,
            history: Vec::new(),
            refresh_history: true,
            hint: None,
            engine_note: None,
            selection: None,
            choices: Vec::new(),
            confirm_resign: false,
            setup_open: false,
            tab: RoomTab::Moves,
            flipped: false,
        }
    }
}

impl GameRoom {
    pub fn new_game(&mut self) -> anyhow::Result<()> {
        let position = self.position.trim_start_matches('\u{feff}').trim();
        let study = self.mode == Mode::Analysis;
        let game = match self.kind {
            Kind::Chess if position.starts_with('[') || position.starts_with("1.") => {
                Game::from_pgn(position, self.user, self.minutes, self.depth, study)
            }
            Kind::Shogi if position.starts_with("position ") || position.contains(" moves ") => {
                Game::from_usi(position, self.user, self.minutes, self.depth, study)
            }
            _ => Game::new(
                self.kind,
                position,
                self.user,
                self.minutes,
                self.depth,
                study,
            ),
        }?;
        self.selection = None;
        self.choices.clear();
        self.confirm_resign = false;
        self.setup_open = false;
        self.tab = RoomTab::Moves;
        self.flipped = false;
        self.game = Some(game);
        self.messages.clear();
        self.draft.clear();
        self.chat_id = crate::new_id();
        self.request_failed = false;
        self.saved = false;
        self.save_pending = false;
        self.save_error = None;
        self.hint = None;
        self.engine_note = None;
        Ok(())
    }

    pub fn show_board(&mut self) {
        self.tab = RoomTab::Moves;
    }

    pub fn moved(&mut self) {
        self.selection = None;
        self.choices.clear();
        self.hint = None;
        self.request_failed = false;
        self.engine_note = None;
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        cache: &mut CommonMarkCache,
        view: &mut RoomView<'_>,
    ) -> Option<GameAction> {
        let mut action = None;
        let kind = self.game.as_ref().map(|game| game.kind);
        ui.horizontal(|ui| {
            ui.heading(kind.map_or("Игры", Kind::name));
            if kind.is_some() {
                ui.label(RichText::new(self.active_mode.title()).color(MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(primary("Новая партия")).clicked() {
                        self.setup_open = true;
                    }
                });
            }
        });
        if let Some(error) = &self.engine_note {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(ERROR, error);
                if self.request_failed {
                    ui.label("Часы приостановлены");
                    if ui.button("Повторить расчёт").clicked() {
                        action = Some(GameAction::RetryEngine);
                    }
                }
            });
        }
        if let Some(error) = &self.save_error {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(ERROR, format!("Партия не сохранена: {error}"));
                if ui.button("Повторить").clicked() {
                    action = Some(GameAction::Retry);
                }
            });
        }
        let Some(game) = &self.game else {
            card(PANEL).show(ui, |ui| {
                ui.set_min_width(ui.available_width().min(480.0));
                ui.label(RichText::new("Новая партия").strong().size(18.0));
                self.setup_form(ui);
                if ui.add(primary("Начать партию")).clicked() {
                    action = Some(GameAction::New);
                }
            });
            self.history(ui);
            return action;
        };
        let legal = if game.outcome.is_none() && (game.study || game.side() == game.user) {
            game.legal()
        } else {
            Vec::new()
        };
        let space = ui.available_width();
        let wide = space >= 680.0;
        if !wide {
            self.tabs(ui);
        }
        if wide || self.tab == RoomTab::Moves {
            let screen = ui.ctx().screen_rect();
            let max_board =
                (screen.height() - if wide { 248.0 } else { 258.0 }).clamp(225.0, 620.0);
            let side_min = if wide { 300.0 } else { 235.0 };
            let board_width = (space * 0.6).min(space - side_min).min(max_board);
            let remaining = space - board_width - 16.0;
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(board_width, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(board_width);
                            if let Some(clicked) = self.board_panel(ui, board_width, &legal) {
                                action = Some(clicked);
                            }
                            if wide && let Some(clicked) = self.board_actions(ui, view.engine_busy)
                            {
                                action = Some(clicked);
                            }
                        },
                    );
                    ui.allocate_ui_with_layout(
                        Vec2::new(remaining, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(remaining);
                            ui.spacing_mut().item_spacing.x = 8.0;
                            self.status_card(ui, view);
                            if wide {
                                self.tabs(ui);
                                if let Some(clicked) = self.active_tab(ui, cache, view) {
                                    action = Some(clicked);
                                }
                            } else {
                                if let Some(clicked) = self.board_actions(ui, view.engine_busy) {
                                    action = Some(clicked);
                                }
                                self.move_table(ui);
                            }
                        },
                    );
                });
            });
        } else {
            self.status_card(ui, view);
            if let Some(clicked) = self.active_tab(ui, cache, view) {
                action = Some(clicked);
            }
        }

        if self.setup_open {
            egui::Window::new("Новая партия")
                .collapsible(false)
                .resizable(false)
                .default_width(440.0)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    if self.game.as_ref().is_some_and(|game| {
                        game.outcome.is_none() && !game.moves.is_empty() && !game.study
                    }) {
                        ui.colored_label(MUTED, "Новая партия закроет текущую незавершённую игру.");
                    }
                    self.setup_form(ui);
                    ui.horizontal(|ui| {
                        if ui.add(primary("Начать партию")).clicked() {
                            action = Some(GameAction::New);
                        }
                        if ui.button("Назад").clicked() {
                            self.setup_open = false;
                        }
                    });
                });
        }
        if !self.choices.is_empty() {
            egui::Window::new("Выбор превращения")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    for movement in &self.choices {
                        if ui.button(movement.option).clicked() {
                            action = Some(GameAction::Move(movement.notation.clone()));
                        }
                    }
                    if ui.button("Назад").clicked() {
                        self.choices.clear();
                    }
                });
        }
        if self.confirm_resign {
            egui::Window::new("Сдаться?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    ui.label("Победу получит соперник.");
                    ui.horizontal(|ui| {
                        if ui.add(primary("Подтвердить")).clicked() {
                            action = Some(GameAction::Resign);
                            self.confirm_resign = false;
                        }
                        if ui.button("Назад").clicked() {
                            self.confirm_resign = false;
                        }
                    });
                });
        }
        action
    }

    fn setup_form(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for kind in [Kind::Chess, Kind::Shogi] {
                if ui
                    .selectable_value(&mut self.kind, kind, kind.name())
                    .clicked()
                {
                    self.position.clear();
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Режим");
            egui::ComboBox::from_id_salt("setup-mode")
                .selected_text(self.mode.title())
                .show_ui(ui, |ui| {
                    for mode in [Mode::Quiet, Mode::Companion, Mode::Mentor, Mode::Analysis] {
                        ui.selectable_value(&mut self.mode, mode, mode.title());
                    }
                });
            ui.label("Сторона");
            egui::ComboBox::from_id_salt("setup-side")
                .selected_text(self.user.label(self.kind))
                .show_ui(ui, |ui| {
                    for side in [Side::First, Side::Second] {
                        ui.selectable_value(&mut self.user, side, side.label(self.kind));
                    }
                });
        });
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.minutes)
                    .range(1..=120)
                    .suffix(" мин"),
            );
            ui.add(
                egui::DragValue::new(&mut self.depth)
                    .range(1..=22)
                    .prefix("Глубина "),
            );
        });
        ui.collapsing("Импорт позиции", |ui| {
            if ui.button("Открыть файл").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Партия", &["pgn", "fen", "sfen", "usi", "txt"])
                    .pick_file()
            {
                match read_position(path) {
                    Ok(value) => self.position = value,
                    Err(error) => self.engine_note = Some(error.to_string()),
                }
            }
            ui.add(
                egui::TextEdit::multiline(&mut self.position)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text(if self.kind == Kind::Chess {
                        "FEN или PGN"
                    } else {
                        "SFEN или USI"
                    }),
            );
        });
    }

    fn board_actions(&mut self, ui: &mut egui::Ui, engine_busy: bool) -> Option<GameAction> {
        let mut action = None;
        let game = self.game.as_ref().expect("game present");
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Развернуть")
                .on_hover_text("Развернуть доску")
                .clicked()
            {
                self.flipped = !self.flipped;
                self.selection = None;
            }
            if ui
                .add_enabled(
                    !engine_busy && (game.outcome.is_none() || !game.moves.is_empty()),
                    egui::Button::new("Подсказка"),
                )
                .clicked()
            {
                action = Some(GameAction::Hint);
            }
        });
        action
    }

    fn status_card(&self, ui: &mut egui::Ui, view: &mut RoomView<'_>) {
        let game = self.game.as_ref().expect("game present");
        let screen = ui.ctx().screen_rect();
        let compact =
            ui.available_width() < 360.0 || screen.width() < 900.0 || screen.height() < 650.0;
        let portrait_width = if compact { 65.0 } else { 118.0 };
        card(SIDE).show(ui, |ui| {
            let width = ui.available_width();
            let status_width = (width - portrait_width - ui.spacing().item_spacing.x).max(100.0);
            ui.set_min_width(width);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(status_width, 0.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(status_width);
                        ui.add_space(4.0);
                        let status = if self.request_failed {
                            "Пауза".into()
                        } else {
                            game.turn_status()
                        };
                        ui.label(RichText::new(status).size(22.0).strong().color(MINT));
                        if game.checked_king().is_some() && game.outcome.is_none() {
                            ui.colored_label(ERROR, "Шах");
                        }
                        if view.engine_busy {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Движок анализирует позицию");
                            });
                        }
                        if let Some(last) = game.labels.last() {
                            let index = game.moves.len() - 1;
                            ui.label(
                                RichText::new(format!(
                                    "Последний ход · {}. {last}",
                                    game.move_number(index)
                                ))
                                .color(MUTED),
                            );
                        }
                    },
                );
                ui.allocate_ui_with_layout(
                    Vec2::new(portrait_width, 0.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(portrait_width);
                        if compact {
                            view.avatar.show_portrait(ui, view.controller, 62.0);
                        } else {
                            view.avatar.show(ui, view.controller, 118.0);
                        }
                    },
                );
            });
        });
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        if let Some(hint) = &self.hint {
            ui.colored_label(ACCENT, hint);
        }
        ui.horizontal_wrapped(|ui| {
            for (tab, label) in [
                (RoomTab::Moves, "Ходы"),
                (RoomTab::Chat, "Диалог с Рей"),
                (RoomTab::Controls, "Партия"),
            ] {
                ui.selectable_value(&mut self.tab, tab, label);
            }
        });
    }

    fn active_tab(
        &mut self,
        ui: &mut egui::Ui,
        cache: &mut CommonMarkCache,
        view: &RoomView<'_>,
    ) -> Option<GameAction> {
        match self.tab {
            RoomTab::Moves => {
                self.move_table(ui);
                None
            }
            RoomTab::Chat => self.game_chat(ui, cache, view),
            RoomTab::Controls => self.game_controls(ui),
        }
    }

    fn move_table(&self, ui: &mut egui::Ui) {
        let game = self.game.as_ref().expect("game present");
        card(SIDE).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let width = ((ui.available_width() - 62.0) * 0.5).max(50.0);
            ui.horizontal(|ui| {
                ui.add_sized(
                    [36.0, 22.0],
                    egui::Label::new(RichText::new("№").color(MUTED)),
                );
                for side in [Side::First, Side::Second] {
                    ui.add_sized(
                        [width, 22.0],
                        egui::Label::new(RichText::new(side.label(game.kind)).color(MUTED)),
                    );
                }
            });
            ui.separator();
            if game.moves.is_empty() {
                ui.label(RichText::new("Ходов пока нет").color(MUTED));
                return;
            }
            let mut rows: Vec<(u32, [Option<usize>; 2])> = Vec::new();
            for index in 0..game.moves.len() {
                let number = game.move_number(index);
                if rows.last().is_none_or(|row| row.0 != number) {
                    rows.push((number, [None; 2]));
                }
                rows.last_mut().expect("move row exists").1
                    [usize::from(game.move_side(index) == Side::Second)] = Some(index);
            }
            egui::ScrollArea::vertical()
                .id_salt("move-table")
                .max_height(240.0)
                .stick_to_bottom(true)
                .show_rows(ui, 27.0, rows.len(), |ui, range| {
                    for &(number, slots) in &rows[range] {
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [36.0, 27.0],
                                egui::Label::new(
                                    RichText::new(number.to_string()).color(MUTED).monospace(),
                                ),
                            );
                            for (column, slot) in slots.into_iter().enumerate() {
                                let label = slot.map_or("·", |index| game.labels[index].as_str());
                                let color = match slot {
                                    Some(index) if index + 1 == game.moves.len() => ACCENT,
                                    Some(_) => TEXT,
                                    None => MUTED,
                                };
                                let element = ui.add_sized(
                                    [width, 27.0],
                                    egui::Label::new(RichText::new(label).color(color).monospace())
                                        .selectable(true),
                                );
                                if let Some(index) = slot {
                                    let side = if column == 0 {
                                        Side::First
                                    } else {
                                        Side::Second
                                    };
                                    element.on_hover_text(format!(
                                        "{} · {}",
                                        side.label(game.kind),
                                        game.moves[index]
                                    ));
                                }
                            }
                        });
                    }
                });
        });
    }

    fn history(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Завершённые партии", |ui| {
            if self.history.is_empty() {
                ui.label(RichText::new("Сохранённых партий пока нет").color(MUTED));
            }
            for game in &self.history {
                let name = if game.kind == "chess" {
                    "Шахматы"
                } else {
                    "Сёги"
                };
                ui.collapsing(
                    format!(
                        "{} · {} · {}",
                        game.date.as_deref().unwrap_or(""),
                        name,
                        game.result
                    ),
                    |ui| {
                        ui.label(format!(
                            "{} · ходов: {} · {}",
                            game.user_side,
                            game.moves.len(),
                            clock(Duration::from_secs(game.seconds))
                        ));
                        ui.label(game.moves.join(" "));
                        if let Some(engine) = &game.engine {
                            ui.label(engine);
                        }
                        if let Some(analysis) = &game.analysis {
                            ui.label(analysis);
                        }
                    },
                );
            }
        });
    }

    fn board_panel(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        legal: &[LegalMove],
    ) -> Option<GameAction> {
        let game = self.game.as_ref().expect("game present");
        let size = game.kind.width();
        let lower = if self.flipped {
            game.user.other()
        } else {
            game.user
        };
        let upper = lower.other();
        hand(ui, game, upper, self.simple, legal, &mut self.selection);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(width), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let cell = width / size as f32;
        let last = game.last;
        let checked = game.checked_king();
        let mut picked = None;
        let mut started = None;
        let mut drag_released = false;
        for displayed in 0..size * size {
            let index = if lower == Side::Second {
                size * size - 1 - displayed
            } else {
                displayed
            };
            let row = displayed / size;
            let col = displayed % size;
            let point = Pos2::new(
                rect.left() + col as f32 * cell,
                rect.top() + row as f32 * cell,
            );
            let square = Rect::from_min_size(point, Vec2::splat(cell));
            let response = ui.interact(
                square,
                ui.id().with(("square", index)),
                egui::Sense::click_and_drag(),
            );
            if response.clicked() {
                picked = Some(index);
            }
            if response.drag_started() {
                started = Some(index);
            }
            drag_released |= response.drag_stopped();
            let base = if game.kind == Kind::Shogi {
                if (row + col).is_multiple_of(2) {
                    Color32::from_rgb(192, 155, 123)
                } else {
                    Color32::from_rgb(178, 141, 112)
                }
            } else if (row + col).is_multiple_of(2) {
                Color32::from_rgb(185, 164, 195)
            } else {
                Color32::from_rgb(92, 72, 113)
            };
            let color = if checked == Some(index) {
                Color32::from_rgb(162, 81, 103)
            } else if self.selection == Some(Origin::Square(index)) {
                Color32::from_rgb(138, 95, 171)
            } else if last.is_some_and(|(from, to)| from == Origin::Square(index) || to == index) {
                Color32::from_rgb(131, 108, 161)
            } else {
                base
            };
            painter.rect_filled(square, 0.0, color);
            if game.kind == Kind::Shogi {
                painter.rect_stroke(
                    square,
                    0.0,
                    Stroke::new(0.7_f32, Color32::from_rgb(131, 97, 79)),
                    egui::StrokeKind::Inside,
                );
            }
            if let Some(piece) = game.piece(index) {
                draw_piece(
                    &painter,
                    square,
                    game.kind,
                    piece,
                    self.simple,
                    piece.side != lower,
                );
            }
            if legal
                .iter()
                .any(|movement| Some(movement.origin) == self.selection && movement.target == index)
            {
                if game.piece(index).is_some() {
                    painter.circle_stroke(
                        square.center(),
                        cell * 0.41,
                        Stroke::new(3.0_f32, Color32::from_rgb(227, 211, 248)),
                    );
                } else {
                    painter.circle_filled(
                        square.center(),
                        cell * 0.114,
                        Color32::from_rgb(221, 206, 240),
                    );
                }
            }
            if col == 0 {
                let rank = if game.kind == Kind::Chess {
                    format!("{}", 8 - index / 8)
                } else {
                    ((b'a' + (index / 9) as u8) as char).to_string()
                };
                painter.text(
                    square.left_top() + egui::vec2(3.0, 2.0),
                    egui::Align2::LEFT_TOP,
                    rank,
                    egui::FontId::proportional(10.0),
                    Color32::from_rgb(33, 24, 43),
                );
            }
            if row == size - 1 {
                let file = if game.kind == Kind::Chess {
                    ((b'a' + (index % 8) as u8) as char).to_string()
                } else {
                    format!("{}", 9 - index % 9)
                };
                painter.text(
                    square.right_bottom() - egui::vec2(3.0, 2.0),
                    egui::Align2::RIGHT_BOTTOM,
                    file,
                    egui::FontId::proportional(10.0),
                    Color32::from_rgb(33, 24, 43),
                );
            }
        }
        if let Some(index) = started
            && legal
                .iter()
                .any(|movement| movement.origin == Origin::Square(index))
        {
            self.selection = Some(Origin::Square(index));
        }
        if drag_released
            && let Some(pointer) = ui
                .ctx()
                .pointer_latest_pos()
                .filter(|pointer| rect.contains(*pointer))
        {
            let col = (((pointer.x - rect.left()) / cell) as usize).min(size - 1);
            let row = (((pointer.y - rect.top()) / cell) as usize).min(size - 1);
            let displayed = row * size + col;
            picked = Some(if lower == Side::Second {
                size * size - 1 - displayed
            } else {
                displayed
            });
        }
        hand(ui, game, lower, self.simple, legal, &mut self.selection);
        if let Some(index) = picked {
            let matches: Vec<_> = legal
                .iter()
                .filter(|movement| {
                    Some(movement.origin) == self.selection && movement.target == index
                })
                .cloned()
                .collect();
            match matches.as_slice() {
                [] => {
                    self.selection = legal
                        .iter()
                        .any(|movement| movement.origin == Origin::Square(index))
                        .then_some(Origin::Square(index))
                }
                [movement] => return Some(GameAction::Move(movement.notation.clone())),
                _ => self.choices = matches,
            }
        }
        None
    }

    fn game_controls(&mut self, ui: &mut egui::Ui) -> Option<GameAction> {
        let game = self.game.as_ref().expect("game present");
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(if game.kind == Kind::Chess {
                    "PGN"
                } else {
                    "CSA"
                })
                .clicked()
            {
                let format = if game.kind == Kind::Chess {
                    "pgn"
                } else {
                    "csa"
                };
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name(format!("rey.{format}"))
                    .add_filter(format, &[format])
                    .save_file()
                {
                    action = Some(GameAction::Export(path));
                }
            }
            if ui
                .button(if game.kind == Kind::Chess {
                    "FEN"
                } else {
                    "SFEN"
                })
                .clicked()
            {
                ui.ctx().copy_text(game.position());
            }
            if game.kind == Kind::Shogi && ui.button("USI").clicked() {
                ui.ctx()
                    .copy_text(format!("position {}", game.engine_position()));
            }
        });
        if game.outcome.is_none() {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Сдаться").clicked() {
                    self.confirm_resign = true;
                }
                if game.kind == Kind::Chess
                    && game.study
                    && ui.button("Ничья по соглашению").clicked()
                {
                    action = Some(GameAction::Draw);
                }
                if game.kind == Kind::Chess
                    && !game.study
                    && game.side() == game.user
                    && ui.button("Предложить ничью").clicked()
                {
                    action = Some(GameAction::Draw);
                }
                if (game.study || game.side() == game.user)
                    && game.can_claim_draw(None)
                    && ui.button("Заявить ничью").clicked()
                {
                    action = Some(GameAction::Claim(None));
                }
                if (game.study || game.side() == game.user) && !game.can_claim_draw(None) {
                    let qualifying: Vec<_> = game
                        .legal()
                        .into_iter()
                        .filter(|movement| {
                            Some(movement.origin) == self.selection
                                && game.can_claim_draw(Some(&movement.notation))
                        })
                        .collect();
                    if !qualifying.is_empty() {
                        ui.menu_button("Ничья с ходом", |ui| {
                            for movement in qualifying {
                                if ui.button(&movement.notation).clicked() {
                                    action = Some(GameAction::Claim(Some(movement.notation)));
                                    ui.close();
                                }
                            }
                        });
                    }
                }
                if (game.study || game.side() == game.user)
                    && game.can_declare()
                    && ui.button("Объявить входящего короля").clicked()
                {
                    action = Some(GameAction::Declare);
                }
                if game.can_impasse() && ui.button("Дзисёги").clicked() {
                    action = Some(GameAction::Impasse);
                }
            });
        }
        if game.kind == Kind::Shogi {
            ui.checkbox(&mut self.simple, "Латинские обозначения");
        }
        self.history(ui);
        action
    }

    fn game_chat(
        &mut self,
        ui: &mut egui::Ui,
        cache: &mut CommonMarkCache,
        view: &RoomView<'_>,
    ) -> Option<GameAction> {
        let game = self.game.as_ref().expect("game present");
        let mut action = None;
        ui.label(
            RichText::new("Рей видит позицию и ходы этой партии")
                .color(MINT)
                .small(),
        );
        let chat_height =
            (ui.ctx().screen_rect().bottom() - ui.cursor().top() - 166.0).clamp(48.0, 210.0);
        card(SIDE).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt(("game-chat", game.id))
                .max_height(chat_height)
                .min_scrolled_height(chat_height)
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if self.messages.is_empty() && view.chat_text.is_none() {
                        ui.label(
                            RichText::new("Спроси о последнем ходе или попроси разобрать позицию.")
                                .color(MUTED),
                        );
                    } else {
                        show_messages(ui, &self.messages, view.chat_text, cache);
                        super::show_reply_preview(ui, view.chat_preview);
                    }
                });
        });
        ui.horizontal(|ui| {
            let field = ui.add_enabled(
                !view.chat_busy,
                egui::TextEdit::singleline(&mut self.draft)
                    .char_limit(4000)
                    .desired_width((ui.available_width() - 110.0).max(70.0))
                    .hint_text("Вопрос о партии…"),
            );
            let enter = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let enabled = !view.chat_busy && !self.draft.trim().is_empty();
            if (ui.add_enabled(enabled, primary("Спросить")).clicked() || enter) && enabled
            {
                action = Some(GameAction::Ask(std::mem::take(&mut self.draft)));
                field.request_focus();
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!view.chat_busy, egui::Button::new("Последние ходы"))
                .clicked()
            {
                action = Some(GameAction::Ask("Покажи последние ходы".into()));
            }
            if ui
                .add_enabled(!view.chat_busy, egui::Button::new("Разобрать позицию"))
                .clicked()
            {
                action = Some(GameAction::Ask("Разбери текущую позицию".into()));
            }
        });
        if let Some(phase) = view.chat_phase {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(phase);
                if ui.button("Стоп").clicked() {
                    action = Some(GameAction::CancelChat);
                }
            });
        }
        action
    }
}

fn read_position(path: PathBuf) -> anyhow::Result<String> {
    anyhow::ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "Файл позиции превышает 1 МиБ"
    );
    Ok(std::fs::read_to_string(path)?)
}

fn hand(
    ui: &mut egui::Ui,
    game: &Game,
    side: Side,
    simple: bool,
    legal: &[LegalMove],
    selected: &mut Option<Origin>,
) {
    let on_move = game.outcome.is_none() && game.side() == side;
    let name = if game.study {
        side.label(game.kind)
    } else if side == game.user {
        "Ты"
    } else {
        "Рей"
    };
    egui::Frame::new()
        .fill(if on_move { RAISED } else { SIDE })
        .corner_radius(9)
        .inner_margin(egui::Margin::symmetric(11, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(name)
                        .strong()
                        .color(if on_move { MINT } else { TEXT }),
                );
                if !game.study {
                    ui.label(RichText::new(side.label(game.kind)).color(MUTED));
                }
                if !game.study {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(clock(game.clock(side)))
                                .monospace()
                                .size(19.0)
                                .color(if on_move { MINT } else { TEXT }),
                        );
                    });
                }
            });
        });
    let items = game.hand(side);
    if items.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for (letter, piece, count) in items {
            let origin = Origin::Hand(letter);
            if ui
                .add_enabled(
                    on_move && legal.iter().any(|movement| movement.origin == origin),
                    egui::Button::selectable(
                        *selected == Some(origin),
                        format!(
                            "{}×{}",
                            if simple { piece.simple } else { piece.symbol },
                            count
                        ),
                    ),
                )
                .clicked()
            {
                *selected = if *selected == Some(origin) {
                    None
                } else {
                    Some(origin)
                };
            }
        }
    });
}

fn clock(duration: Duration) -> String {
    let seconds = duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() > 0));
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn draw_piece(
    painter: &egui::Painter,
    square: Rect,
    kind: Kind,
    piece: PieceView,
    simple: bool,
    opponent: bool,
) {
    let center = square.center();
    let scale = square.width();
    if kind == Kind::Chess {
        let color = if piece.side == Side::First {
            Color32::from_rgb(255, 247, 240)
        } else {
            Color32::from_rgb(49, 29, 60)
        };
        let outline = if piece.side == Side::First {
            Color32::from_rgb(60, 44, 77)
        } else {
            Color32::from_rgb(217, 192, 227)
        };
        let font = egui::FontId::proportional(scale * 0.77);
        let symbol = if simple { piece.simple } else { piece.symbol };
        painter.text(
            center + egui::vec2(0.0, 1.4),
            egui::Align2::CENTER_CENTER,
            symbol,
            font.clone(),
            outline,
        );
        painter.text(center, egui::Align2::CENTER_CENTER, symbol, font, color);
        return;
    }
    let direction = if opponent { -1.0 } else { 1.0 };
    let vertices = [
        (0.0, -0.39),
        (0.29, -0.24),
        (0.34, 0.36),
        (-0.34, 0.36),
        (-0.29, -0.24),
    ]
    .into_iter()
    .map(|(x, y)| center + egui::vec2(x * scale * direction, y * scale * direction))
    .collect();
    painter.add(egui::Shape::convex_polygon(
        vertices,
        Color32::from_rgb(244, 208, 161),
        Stroke::new(1.0_f32, Color32::from_rgb(103, 65, 48)),
    ));
    let text = if simple { piece.simple } else { piece.symbol };
    let color = if piece.promoted {
        Color32::from_rgb(158, 51, 58)
    } else {
        Color32::from_rgb(51, 33, 38)
    };
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(scale * 0.43),
        color,
    );
    let pos = center - galley.size() * 0.5;
    if opponent {
        painter.add(
            egui::epaint::TextShape::new(pos, galley, color)
                .with_angle_and_anchor(std::f32::consts::PI, egui::Align2::CENTER_CENTER),
        );
    } else {
        painter.galley(pos, galley, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_game_chat_paints_the_streamed_reply_inside_the_transcript() {
        let ctx = egui::Context::default();
        let empty_root = tempfile::tempdir().unwrap();
        let controller = AvatarController::default();
        let mut avatar = AvatarArt::load(empty_root.path(), &ctx);
        let mut room = GameRoom::default();
        room.new_game().unwrap();
        room.tab = RoomTab::Chat;
        let mut cache = CommonMarkCache::default();
        let preview = "The e-pawn reaches e4.";
        let mut view = RoomView {
            controller: &controller,
            avatar: &mut avatar,
            engine_busy: false,
            chat_busy: true,
            chat_text: Some("Why e4?"),
            chat_phase: Some("Generating"),
            chat_preview: preview,
        };
        let mut frame = egui::FullOutput::default();
        for _ in 0..2 {
            frame = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1260.0, 790.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            room.show(ui, &mut cache, &mut view);
                        });
                    });
                },
            );
        }
        assert!(frame.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text)
                if text.galley.text() == preview
                    && shape.clip_rect.contains(text.visual_bounding_rect().center()))
        }));
    }
}
