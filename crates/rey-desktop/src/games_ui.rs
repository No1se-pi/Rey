use crate::{show_messages, theme::*};
use eframe::egui::{self, Align, Color32, Layout, Pos2, Rect, RichText, Stroke, Vec2};
use egui_commonmark::CommonMarkCache;
use rey_core::{
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
    confirm: Option<&'static str>,
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
            confirm: None,
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
        self.confirm = None;
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
        engine: bool,
        chat_busy: bool,
        game_pending: Option<&str>,
    ) -> Option<GameAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            ui.heading("Игры");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(primary("Начать партию")).clicked() {
                    if self.game.as_ref().is_some_and(|game| {
                        game.outcome.is_none() && !game.moves.is_empty() && !game.study
                    }) {
                        self.confirm = Some("new");
                    } else {
                        action = Some(GameAction::New);
                    }
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            for kind in [Kind::Chess, Kind::Shogi] {
                if ui
                    .selectable_value(&mut self.kind, kind, kind.name())
                    .clicked()
                {
                    self.position.clear();
                }
            }
            egui::ComboBox::from_id_salt("mode")
                .selected_text(self.mode.title())
                .show_ui(ui, |ui| {
                    for mode in [Mode::Quiet, Mode::Companion, Mode::Mentor, Mode::Analysis] {
                        ui.selectable_value(&mut self.mode, mode, mode.title());
                    }
                });
            egui::ComboBox::from_id_salt("side")
                .selected_text(self.user.label(self.kind))
                .show_ui(ui, |ui| {
                    for side in [Side::First, Side::Second] {
                        ui.selectable_value(&mut self.user, side, side.label(self.kind));
                    }
                });
        });
        ui.collapsing("Настройки новой партии", |ui| {
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
                if ui.button("Импорт из файла").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Партия", &["pgn", "fen", "sfen", "usi", "txt"])
                        .pick_file()
                {
                    match read_position(path) {
                        Ok(value) => self.position = value,
                        Err(error) => self.engine_note = Some(error.to_string()),
                    }
                }
            });
            ui.add(
                egui::TextEdit::multiline(&mut self.position)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text(if self.kind == Kind::Chess {
                        "Начальная позиция · FEN или PGN"
                    } else {
                        "Начальная позиция · SFEN или USI"
                    }),
            );
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
            ui.horizontal(|ui| {
                ui.colored_label(ERROR, format!("Партия не сохранена: {error}"));
                if ui.button("Повторить").clicked() {
                    action = Some(GameAction::Retry);
                }
            });
        }
        if self.game.is_none() {
            card(SIDE).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.add_space(34.0);
                    ui.label(RichText::new("♘").color(ACCENT).size(45.0));
                    ui.heading("Сыграем партию?");
                    ui.label(RichText::new("Выбери игру, сторону и режим.").color(MUTED));
                    ui.add_space(34.0);
                });
            });
            self.history(ui);
            return action;
        }
        let game = self.game.as_ref().expect("game present");
        let moving = game.outcome.is_none() && (game.study || game.side() == game.user);
        let legal = if moving { game.legal() } else { Vec::new() };
        if let Some(outcome) = &game.outcome {
            let result = match outcome.winner {
                Some(side) => format!("{} · {}", side.label(game.kind), outcome.reason),
                None => format!("Ничья · {}", outcome.reason),
            };
            ui.colored_label(MINT, result);
        }
        if game.kind == Kind::Shogi {
            ui.checkbox(&mut self.simple, "Латинские обозначения");
        }
        let space = ui.available_width();
        let wide = space >= 615.0;
        let board_width = (if wide { space * 0.64 } else { space }).clamp(240.0, 490.0);
        if wide {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(board_width);
                    if let Some(clicked) = self.board_panel(ui, board_width, &legal) {
                        action = Some(clicked);
                    }
                });
                ui.vertical(|ui| {
                    ui.set_min_width((space - board_width - 10.0).max(120.0));
                    if let Some(clicked) =
                        self.game_details(ui, cache, engine, chat_busy, game_pending)
                    {
                        action = Some(clicked);
                    }
                });
            });
        } else {
            if let Some(clicked) = self.board_panel(ui, board_width, &legal) {
                action = Some(clicked);
            }
            if let Some(clicked) = self.game_details(ui, cache, engine, chat_busy, game_pending) {
                action = Some(clicked);
            }
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
        if let Some(confirm) = self.confirm {
            egui::Window::new(if confirm == "new" {
                "Начать новую партию?"
            } else {
                "Сдаться?"
            })
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                ui.label(if confirm == "new" {
                    "Текущая незавершённая партия будет закрыта."
                } else {
                    "Победу получит соперник."
                });
                ui.horizontal(|ui| {
                    if ui.add(primary("Подтвердить")).clicked() {
                        action = Some(if confirm == "new" {
                            GameAction::New
                        } else {
                            GameAction::Resign
                        });
                        self.confirm = None;
                    }
                    if ui.button("Назад").clicked() {
                        self.confirm = None;
                    }
                });
            });
        }
        self.history(ui);
        action
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
        let upper = game.user.other();
        let lower = game.user;
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
            let index = if game.user == Side::Second {
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
                    piece.side != game.user,
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
            picked = Some(if game.user == Side::Second {
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

    fn game_details(
        &mut self,
        ui: &mut egui::Ui,
        cache: &mut CommonMarkCache,
        engine: bool,
        chat_busy: bool,
        pending: Option<&str>,
    ) -> Option<GameAction> {
        let game = self.game.as_ref().expect("game present");
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !engine && (game.outcome.is_none() || !game.moves.is_empty()),
                    egui::Button::new("Подсказка"),
                )
                .clicked()
            {
                action = Some(GameAction::Hint);
            }
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
                    self.confirm = Some("resign");
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
        if engine {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Движок рассчитывает позицию");
            });
        }
        if let Some(hint) = &self.hint {
            ui.colored_label(ACCENT, hint);
        }
        ui.collapsing("Ходы", |ui| {
            egui::ScrollArea::vertical()
                .id_salt("moves")
                .max_height(100.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.label(game.labels.join("  "));
                });
        });
        ui.label(RichText::new("Обсудить партию").strong());
        card(SIDE).show(ui, |ui| {
            ui.set_min_width((ui.available_width() - 1.0).max(0.0));
            egui::ScrollArea::vertical()
                .id_salt("game-chat")
                .max_height(220.0)
                .min_scrolled_height(90.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if self.messages.is_empty() && pending.is_none() {
                        ui.label(RichText::new("Спроси Рей о ходе или позиции.").color(MUTED));
                    } else {
                        show_messages(ui, &self.messages, pending, cache);
                    }
                });
        });
        ui.horizontal(|ui| {
            let input = ui.add_enabled(
                !chat_busy,
                egui::TextEdit::singleline(&mut self.draft)
                    .char_limit(4000)
                    .desired_width((ui.available_width() - 105.0).max(70.0))
                    .hint_text("Сообщение Рей…"),
            );
            let enter = input.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if !chat_busy
                && (ui.add(primary("Спросить")).clicked() || enter)
                && !self.draft.trim().is_empty()
            {
                action = Some(GameAction::Ask(std::mem::take(&mut self.draft)));
            }
            if pending.is_some() && ui.button("Стоп").clicked() {
                action = Some(GameAction::CancelChat);
            }
        });
        if !chat_busy
            && ui
                .button(if game.outcome.is_some() {
                    "Разобрать партию"
                } else {
                    "Объяснить позицию"
                })
                .clicked()
        {
            action = Some(GameAction::Ask(
                if game.outcome.is_some() {
                    "Разбери эту партию и объясни критические решения."
                } else {
                    "Объясни позицию, проверенные угрозы и доступные варианты."
                }
                .into(),
            ));
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
    ui.horizontal_wrapped(|ui| {
        let on_move = game.outcome.is_none() && game.side() == side;
        ui.label(
            RichText::new(format!(
                "{}  {}",
                side.label(game.kind),
                clock(game.clock(side))
            ))
            .strong()
            .color(if on_move { MINT } else { TEXT }),
        );
        for (letter, piece, count) in game.hand(side) {
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
