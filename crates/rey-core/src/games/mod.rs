mod chess;
mod dialogue;
pub use dialogue::GameSnapshot;
pub mod engine;
mod shogi;

use anyhow::{Context, Result, ensure};
use cozy_chess as c;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::memory::GameRecord;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Chess,
    Shogi,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Chess => "Шахматы",
            Self::Shogi => "Сёги",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Chess => "chess",
            Self::Shogi => "shogi",
        }
    }
    pub fn width(self) -> usize {
        match self {
            Self::Chess => 8,
            Self::Shogi => 9,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    First,
    Second,
}

impl Side {
    pub fn other(self) -> Self {
        match self {
            Self::First => Self::Second,
            Self::Second => Self::First,
        }
    }
    pub fn label(self, kind: Kind) -> &'static str {
        match (self, kind) {
            (Self::First, Kind::Chess) => "Белые",
            (Self::Second, Kind::Chess) => "Чёрные",
            (Self::First, Kind::Shogi) => "Сэнтэ",
            (Self::Second, Kind::Shogi) => "Готэ",
        }
    }
    fn index(self) -> usize {
        usize::from(self == Self::Second)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub winner: Option<Side>,
    pub reason: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Square(usize),
    Hand(char),
}

#[derive(Clone, Debug)]
pub struct LegalMove {
    pub origin: Origin,
    pub target: usize,
    pub notation: String,
    pub option: &'static str,
}

#[derive(Clone, Copy)]
pub struct PieceView {
    pub side: Side,
    pub symbol: &'static str,
    pub simple: &'static str,
    pub promoted: bool,
}

enum Rules {
    Chess(Box<chess::Chess>),
    Shogi(Box<shogi::Shogi>),
}

pub struct Game {
    pub id: Uuid,
    pub kind: Kind,
    pub user: Side,
    pub outcome: Option<Outcome>,
    pub moves: Vec<String>,
    pub labels: Vec<String>,
    pub last: Option<(Origin, usize)>,
    pub engine_name: Option<String>,
    pub analysis: Option<String>,
    pub depth: u8,
    pub study: bool,
    initial: String,
    started: Instant,
    turn_started: Instant,
    clock_paused: bool,
    finished_after: Option<Duration>,
    clock: [Duration; 2],
    minutes: u64,
    rules: Rules,
}

impl Game {
    pub fn new(
        kind: Kind,
        position: &str,
        user: Side,
        minutes: u64,
        depth: u8,
        study: bool,
    ) -> Result<Self> {
        let rules = match kind {
            Kind::Chess => Rules::Chess(Box::new(chess::Chess::new(position)?)),
            Kind::Shogi => Rules::Shogi(Box::new(shogi::Shogi::new(position)?)),
        };
        let now = Instant::now();
        let mut game = Self {
            id: Uuid::new_v4(),
            kind,
            user,
            rules,
            started: now,
            turn_started: now,
            clock_paused: false,
            finished_after: None,
            clock: [Duration::from_secs(minutes.clamp(1, 120) * 60); 2],
            minutes,
            study,
            depth: depth.clamp(1, 22),
            outcome: None,
            moves: Vec::new(),
            labels: Vec::new(),
            last: None,
            engine_name: None,
            analysis: None,
            initial: String::new(),
        };
        game.initial = game.position();
        game.apply_rules();
        Ok(game)
    }

    pub fn position(&self) -> String {
        match &self.rules {
            Rules::Chess(game) => game.fen(),
            Rules::Shogi(game) => game
                .board
                .to_sfen_owned()
                .trim_start_matches("sfen ")
                .to_owned(),
        }
    }

    pub fn engine_position(&self) -> String {
        let format = if self.kind == Kind::Chess {
            "fen"
        } else {
            "sfen"
        };
        if self.moves.is_empty() {
            format!("{format} {}", self.initial)
        } else {
            format!("{format} {} moves {}", self.initial, self.moves.join(" "))
        }
    }

    pub fn side(&self) -> Side {
        match &self.rules {
            Rules::Chess(game) => chess::side(game.board.side_to_move()),
            Rules::Shogi(game) => shogi::side(game.board.side_to_move()),
        }
    }

    pub fn piece(&self, square: usize) -> Option<PieceView> {
        match &self.rules {
            Rules::Chess(game) => game.piece(square),
            Rules::Shogi(game) => game.piece(square),
        }
    }

    pub fn hand(&self, side: Side) -> Vec<(char, PieceView, u8)> {
        match &self.rules {
            Rules::Chess(_) => Vec::new(),
            Rules::Shogi(game) => game.hand(side),
        }
    }

    pub fn checked_king(&self) -> Option<usize> {
        match &self.rules {
            Rules::Chess(game) => game.checked_king(),
            Rules::Shogi(game) => game.checked_king(),
        }
    }

    pub fn legal(&self) -> Vec<LegalMove> {
        if self.outcome.is_some() {
            return Vec::new();
        }
        match &self.rules {
            Rules::Chess(game) => game.legal(),
            Rules::Shogi(game) => game.legal(),
        }
    }

    pub fn play(&mut self, notation: &str) -> Result<()> {
        self.tick();
        let movement = self
            .legal()
            .into_iter()
            .find(|movement| movement.notation == notation)
            .context("Нелегальный ход или партия завершена")?;
        let side = self.side();
        let time_left = self.clock(side);
        let label = match &mut self.rules {
            Rules::Chess(game) => game.play(&movement.notation)?,
            Rules::Shogi(game) => game.play(&movement.notation)?,
        };
        if !self.study {
            self.clock[side.index()] = time_left;
        }
        self.turn_started = Instant::now();
        self.last = Some((movement.origin, movement.target));
        self.moves.push(movement.notation);
        self.labels.push(label);
        self.apply_rules();
        Ok(())
    }

    pub fn can_claim_draw(&self, intended: Option<&str>) -> bool {
        self.outcome.is_none()
            && matches!(&self.rules, Rules::Chess(chess) if chess.can_claim(intended))
    }

    pub fn claim_draw(&mut self, intended: Option<&str>) -> Result<()> {
        self.tick();
        ensure!(
            self.can_claim_draw(intended),
            "Условия заявления ничьей не выполнены"
        );
        self.finish(None, "Заявленная ничья");
        Ok(())
    }

    pub fn can_declare(&self) -> bool {
        self.outcome.is_none() && matches!(&self.rules, Rules::Shogi(shogi) if shogi.can_declare())
    }

    pub fn declare(&mut self) -> Result<()> {
        self.tick();
        ensure!(self.can_declare(), "Условия входящего короля не выполнены");
        self.finish(Some(self.side()), "Входящий король");
        Ok(())
    }

    pub fn can_impasse(&self) -> bool {
        self.outcome.is_none()
            && matches!(&self.rules, Rules::Shogi(shogi) if shogi.impasse().is_some())
    }

    pub fn impasse(&mut self) -> Result<()> {
        self.tick();
        ensure!(self.outcome.is_none(), "Партия уже завершена");
        let Rules::Shogi(shogi) = &self.rules else {
            anyhow::bail!("Правило действует в сёги")
        };
        let winner = shogi
            .impasse()
            .context("Оба короля должны быть в зоне превращения без шаха")?;
        self.finish(winner, "Дзисёги");
        Ok(())
    }

    pub fn resign(&mut self, side: Side) {
        self.tick();
        self.finish(Some(side.other()), "Сдача");
    }
    pub fn agreed_draw(&mut self) {
        self.tick();
        self.finish(None, "Ничья по соглашению");
    }

    fn apply_rules(&mut self) {
        let outcome = match &self.rules {
            Rules::Chess(game) => game.outcome(),
            Rules::Shogi(game) => game.outcome(),
        };
        if let Some(outcome) = outcome {
            self.finish(outcome.winner, outcome.reason);
        }
    }

    fn finish(&mut self, winner: Option<Side>, reason: &'static str) {
        if self.outcome.is_none() {
            let side = self.side();
            self.clock[side.index()] = self.clock(side);
            self.outcome = Some(Outcome { winner, reason });
            self.finished_after = Some(self.started.elapsed());
        }
    }

    pub fn clock(&self, side: Side) -> Duration {
        let time = self.clock[side.index()];
        if self.outcome.is_none() && side == self.side() && !self.study && !self.clock_paused {
            time.saturating_sub(self.turn_started.elapsed())
        } else {
            time
        }
    }

    pub fn pause_clock(&mut self) {
        let side = self.side();
        self.clock[side.index()] = self.clock(side);
        self.clock_paused = true;
    }

    pub fn resume_clock(&mut self) {
        if self.clock_paused {
            self.clock_paused = false;
            self.turn_started = Instant::now();
        }
    }

    pub fn tick(&mut self) {
        if self.outcome.is_some()
            || self.study
            || self.clock_paused
            || !self.clock(self.side()).is_zero()
        {
            return;
        }
        let winner = self.side().other();
        if matches!(&self.rules, Rules::Chess(game) if !game.has_mating_material(winner)) {
            self.finish(None, "Время истекло, мат невозможен");
        } else {
            self.finish(Some(winner), "Время истекло");
        }
    }

    pub fn result_code(&self) -> &'static str {
        match self.outcome.as_ref().map(|result| result.winner) {
            Some(Some(Side::First)) => "1-0",
            Some(Some(Side::Second)) => "0-1",
            Some(None) => "1/2-1/2",
            None => "*",
        }
    }

    pub fn verified_line(&self, line: &[String]) -> Vec<String> {
        let Ok(mut position) = Self::new(
            self.kind,
            &self.position(),
            self.user,
            self.minutes,
            self.depth,
            true,
        ) else {
            return Vec::new();
        };
        line.iter()
            .take(12)
            .take_while(|movement| position.play(movement).is_ok())
            .cloned()
            .collect()
    }

    pub fn record(&self) -> Option<GameRecord> {
        let outcome = self.outcome.as_ref()?;
        Some(GameRecord {
            id: self.id,
            kind: self.kind.key().to_owned(),
            user_side: self.user.label(self.kind).to_owned(),
            moves: self.moves.clone(),
            initial_position: self.initial.clone(),
            result: format!("{} ({})", self.result_code(), outcome.reason),
            seconds: self.finished_after.unwrap_or_default().as_secs(),
            engine: self.engine_name.clone(),
            engine_settings: format!(
                "depth {}; {} min; study {}",
                self.depth, self.minutes, self.study
            ),
            analysis: self.analysis.clone(),
            date: None,
        })
    }

    pub fn export(&self) -> String {
        match self.kind {
            Kind::Chess => {
                let mut pgn = format!(
                    "[Event \"Rey Desktop\"]\n[Site \"Local\"]\n[Date \"????.??.??\"]\n[Round \"-\"]\n[White \"{}\"]\n[Black \"{}\"]\n[Result \"{}\"]\n",
                    if self.user == Side::First {
                        "Player"
                    } else {
                        "Rey"
                    },
                    if self.user == Side::Second {
                        "Player"
                    } else {
                        "Rey"
                    },
                    self.result_code()
                );
                if self.initial != c::Board::default().to_string() {
                    pgn.push_str(&format!("[SetUp \"1\"]\n[FEN \"{}\"]\n", self.initial));
                }
                pgn.push('\n');
                let initial = chess::Chess::new(&self.initial)
                    .expect("validated FEN")
                    .board;
                let start_ply = 2 * (u32::from(initial.fullmove_number()) - 1)
                    + u32::from(initial.side_to_move() == c::Color::Black);
                for (index, label) in self.labels.iter().enumerate() {
                    let ply = start_ply + index as u32;
                    if ply.is_multiple_of(2) {
                        pgn.push_str(&format!("{}. ", ply / 2 + 1));
                    } else if index == 0 {
                        pgn.push_str(&format!("{}... ", ply / 2 + 1));
                    }
                    pgn.push_str(label);
                    pgn.push(' ');
                }
                pgn.push_str(self.result_code());
                pgn.push('\n');
                pgn
            }
            Kind::Shogi => {
                shogi::export_csa(&self.initial, &self.moves, self.user, self.outcome.as_ref())
            }
        }
    }

    pub fn from_pgn(pgn: &str, user: Side, minutes: u64, depth: u8, study: bool) -> Result<Self> {
        let pgn = chess::pgn_moves(pgn)?;
        let mut game = Self::new(Kind::Chess, &pgn.fen, user, minutes, depth, true)?;
        for notation in pgn.moves {
            let Rules::Chess(chess) = &game.rules else {
                unreachable!()
            };
            let movement = c::util::parse_san_move(&chess.board, &notation)
                .context("Некорректный ход SAN в PGN")?;
            let uci = c::util::display_uci_move(&chess.board, movement).to_string();
            game.play(&uci)?;
        }
        if let Some(result) = pgn.result {
            if game.outcome.is_some() {
                ensure!(
                    game.result_code() == result,
                    "Результат PGN противоречит позиции"
                );
            } else {
                let winner = match result {
                    "1-0" => Some(Side::First),
                    "0-1" => Some(Side::Second),
                    _ => None,
                };
                game.finish(winner, "Результат из PGN");
            }
        }
        game.study = study;
        game.turn_started = Instant::now();
        Ok(game)
    }

    pub fn from_usi(usi: &str, user: Side, minutes: u64, depth: u8, study: bool) -> Result<Self> {
        let args = usi.trim().strip_prefix("position ").unwrap_or(usi.trim());
        let (initial, moves) = args.split_once(" moves ").unwrap_or((args, ""));
        let initial = if initial == "startpos" { "" } else { initial };
        let mut game = Self::new(Kind::Shogi, initial, user, minutes, depth, true)?;
        for movement in moves.split_whitespace() {
            game.play(movement)?;
        }
        game.study = study;
        game.turn_started = Instant::now();
        Ok(game)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_positions_and_formats() {
        for (kind, first_move, count) in [(Kind::Chess, "e2e4", 20), (Kind::Shogi, "7g7f", 30)] {
            let mut game = Game::new(kind, "", Side::First, 10, 5, true).unwrap();
            assert_eq!(game.legal().len(), count);
            assert!(game.play("bad").is_err());
            game.play(first_move).unwrap();
            assert_eq!(game.side(), Side::Second);
            assert!(game.engine_position().ends_with(first_move));
            let imported = match kind {
                Kind::Chess => Game::from_pgn(&game.export(), Side::First, 10, 5, true),
                Kind::Shogi => Game::from_usi(&game.engine_position(), Side::First, 10, 5, true),
            }
            .unwrap();
            assert_eq!(imported.position(), game.position());
        }
    }

    #[test]
    fn chess_draw_claim_is_distinct_from_the_automatic_draw() {
        let mut game = Game::new(Kind::Chess, "", Side::First, 10, 5, true).unwrap();
        let cycle = ["g1f3", "g8f6", "f3g1", "f6g8"];
        for movement in cycle.into_iter().cycle().take(7) {
            game.play(movement).unwrap();
        }
        assert!(game.can_claim_draw(Some("f6g8")));
        assert!(!game.can_claim_draw(None));
        game.play("f6g8").unwrap();
        assert!(game.can_claim_draw(None));
        assert!(game.outcome.is_none());
        for movement in cycle.into_iter().cycle().take(8) {
            game.play(movement).unwrap();
        }
        assert_eq!(game.result_code(), "1/2-1/2");
    }

    #[test]
    fn an_engine_failure_can_freeze_and_resume_the_active_clock() {
        let mut game = Game::new(Kind::Chess, "", Side::First, 10, 5, false).unwrap();
        game.play("e2e4").unwrap();
        game.turn_started -= Duration::from_secs(4);
        game.pause_clock();
        let paused = game.clock(Side::Second);
        assert!(paused <= Duration::from_secs(596));
        game.turn_started -= Duration::from_secs(700);
        game.tick();
        assert_eq!(game.clock(Side::Second), paused);
        assert!(game.outcome.is_none());
        game.resume_clock();
        assert!(game.clock(Side::Second) > Duration::from_secs(595));
    }

    #[test]
    fn completed_game_can_be_saved_again_without_duplicates() {
        let folder = tempfile::tempdir().unwrap();
        let store = crate::memory::Store::open(folder.path().join("game.db")).unwrap();
        let mut game = Game::new(Kind::Chess, "", Side::First, 10, 5, false).unwrap();
        game.play("e2e4").unwrap();
        game.resign(Side::Second);
        let record = game.record().unwrap();
        store.save_game(&record).unwrap();
        store.save_game(&record).unwrap();
        let records = store.games(10).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].moves, ["e2e4"]);
        assert!(records[0].date.is_some());
    }
}
