use super::{Game, Kind, Outcome, PieceView, Side, engine};
use regex::Regex;
use std::{fmt::Write, path::Path, sync::LazyLock, time::Duration};

pub struct GameSnapshot {
    kind: Kind,
    side: Side,
    user: Side,
    study: bool,
    position: String,
    engine_position: String,
    depth: u8,
    moves: Vec<PlayedMove>,
    last_piece: Option<PieceView>,
    legal: Vec<String>,
    check: bool,
    outcome: Option<Outcome>,
    analysis: Option<String>,
}

struct PlayedMove {
    number: u32,
    side: Side,
    label: String,
    notation: String,
}

impl Game {
    pub fn move_side(&self, index: usize) -> Side {
        let distance = self.moves.len().saturating_sub(index);
        if distance.is_multiple_of(2) {
            self.side()
        } else {
            self.side().other()
        }
    }

    pub fn move_number(&self, index: usize) -> u32 {
        let start = self
            .initial
            .split_ascii_whitespace()
            .last()
            .and_then(|number| number.parse::<u32>().ok())
            .unwrap_or(1);
        let advance = if self.kind == Kind::Chess {
            (index + usize::from(self.move_side(0) == Side::Second)) / 2
        } else {
            index
        };
        start.saturating_add(advance as u32)
    }

    pub fn turn_status(&self) -> String {
        turn_status(
            self.kind,
            self.side(),
            self.user,
            self.study,
            self.outcome.as_ref(),
        )
    }

    pub fn snapshot(&self) -> GameSnapshot {
        GameSnapshot {
            kind: self.kind,
            side: self.side(),
            user: self.user,
            study: self.study,
            position: self.position(),
            engine_position: self.engine_position(),
            depth: self.depth,
            moves: self
                .moves
                .iter()
                .zip(&self.labels)
                .enumerate()
                .map(|(index, (notation, label))| PlayedMove {
                    number: self.move_number(index),
                    side: self.move_side(index),
                    label: label.clone(),
                    notation: notation.clone(),
                })
                .collect(),
            last_piece: self.last.and_then(|(_, target)| self.piece(target)),
            legal: self
                .legal()
                .into_iter()
                .map(|movement| movement.notation)
                .collect(),
            check: self.checked_king().is_some(),
            outcome: self.outcome.clone(),
            analysis: self.analysis.clone(),
        }
    }
}

fn turn_status(
    kind: Kind,
    side: Side,
    user: Side,
    study: bool,
    outcome: Option<&Outcome>,
) -> String {
    if let Some(result) = outcome {
        let label = result.winner.map_or("Ничья", |winner| winner.label(kind));
        format!("{label} · {}", result.reason)
    } else if study {
        format!("Ходят {}", side.label(kind).to_lowercase())
    } else if side == user {
        "Твой ход".into()
    } else {
        "Ход Рей".into()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Player {
    Both,
    User,
    Rey,
}

enum Query {
    Last(Player),
    History(Player),
    Turn,
    Legal,
}

static REY_SIDE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"\b(?:твой|твои|твоим|твоими|твоего|твоих|ты (?:сделала?|походила?|ходила?|сыграла?|совершила?)|ход(?:ы)? рей)\b"
).unwrap()
});
static USER_SIDE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"\b(?:мой|мои|моим|моими|моего|моих|я (?:сделала?|походила?|ходила?|сыграла?|совершила?)|ход(?:ы)? пользователя)\b"
).unwrap()
});
static ANALYSIS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"\b(?:почему|зачем|объясни\w*|проанализируй\w*|оцени\w*|разбери\w*|посоветуй\w*|лучш\w*|хорош\w*|плох\w*|ошиб\w*)\b"
).unwrap()
});
static NO_BOARD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?is)(?:не (?:могу|могла) (?:получить доступ|видеть|посмотреть|увидеть|проверить.{0,30}ходы)|не вижу|нет доступа|нет (?:данных|информации|истории)).{0,70}(?:парт|доск|ход|позици)|(?:пришли|предоставь|отправь).{0,40}(?:список ходов|историю партии|текущую позицию)|(?:cannot|can.t|don.t|no access).{0,40}(?:see|get|access|view)?.{0,30}(?:board|game|moves)"
).unwrap()
});

fn query(text: &str) -> Option<Query> {
    let text = text.to_lowercase().replace('ё', "е");
    if ANALYSIS.is_match(&text) {
        return None;
    }
    let contains = |markers: &[&str]| markers.iter().any(|marker| text.contains(marker));
    let move_related = contains(&["ход", "сыграл", "походил", "двинул"]);
    let user = USER_SIDE.is_match(&text);
    let rey = REY_SIDE.is_match(&text);
    let player = match (user, rey) {
        (true, false) => Player::User,
        (false, true) => Player::Rey,
        _ => Player::Both,
    };
    if contains(&[
        "чей ход",
        "кто ходит",
        "кто сейчас ходит",
        "моя очередь",
        "твоя очередь",
    ]) || matches!(text.trim(), "мой ход?" | "твой ход?")
    {
        return Some(Query::Turn);
    }
    if move_related
        && contains(&[
            "легальн",
            "допустим",
            "доступн",
            "могу сделать",
            "у меня есть",
            "можно сделать",
            "могу сейчас",
        ])
        || contains(&[
            "куда я могу",
            "куда можно походить",
            "куда мне можно походить",
        ])
    {
        return Some(Query::Legal);
    }
    if move_related
        && contains(&["последн", "только что", "каким ходом"])
        && !contains(&["последние", "последних"])
    {
        return Some(Query::Last(player));
    }
    if (move_related
        && contains(&[
            "какие",
            "каким",
            "покажи",
            "перечисли",
            "список",
            "истори",
            "запис",
            "видишь",
            "знаешь",
            "что я сделал",
            "что я походил",
            "что ты сделала",
            "что ты походила",
            "что сыграл",
            "что я сыграл",
            "что ты сыграла",
            "как я ходил",
            "как ты ходила",
            "как ты сходила",
        ]))
        || contains(&[
            "видишь партию",
            "видишь мою партию",
            "видишь эту партию",
            "видишь доску",
            "видишь позицию",
            "история партии",
            "ходы партии",
            "последние ходы",
            "последних ходов",
            "мои ходы",
            "твои ходы",
            "что на доске",
            "что сейчас на доске",
        ])
    {
        return Some(Query::History(player));
    }
    None
}

impl GameSnapshot {
    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn asks_for_engine(text: &str) -> bool {
        let text = text.trim().trim_end_matches(['?', '.', '!']).to_lowercase();
        matches!(
            text.as_str(),
            "разбери позицию"
                | "разбери текущую позицию"
                | "оцени позицию"
                | "оцени текущую позицию"
                | "объясни позицию"
                | "объясни текущую позицию"
                | "какой ход посоветуешь"
                | "какой ход лучше"
                | "какой ход лучше сделать"
                | "дай подсказку"
        )
    }

    pub fn concerns_game(text: &str) -> bool {
        static TOPIC: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
            r"\b(?:шахмат\w*|сёги|сеги|(?:мою|моей|эту|этой|нашу|нашей|текущую|текущей) парти\w*|(?:на|с) (?:игровой |шахматной )?доск\w*|(?:текущую|текущей|эту|этой) позици\w*|ход(?:ы|а)? (?:в|из) (?:этой |нашей )?парти\w*|(?:мой|твой|мои|твои|моего|твоего|последний|последние|последнего) ход[ыа]?\b)"
        ).unwrap()
        });
        query(text).is_some() || Self::asks_for_engine(text) || TOPIC.is_match(&text.to_lowercase())
    }

    pub fn factual_answer(&self, text: &str) -> Option<String> {
        match query(text)? {
            Query::Turn => Some(self.status()),
            Query::Legal if self.outcome.is_some() => {
                Some(format!("Партия завершена. {}", self.status()))
            }
            Query::Legal => Some(format!(
                "{}. Допустимые ходы ({})\n\n{}",
                self.status(),
                self.legal.len(),
                self.legal
                    .iter()
                    .map(|notation| format!("`{notation}`"))
                    .collect::<Vec<_>>()
                    .join(" · ")
            )),
            Query::Last(player) => Some(self.move_answer(player, 1)),
            Query::History(player) => {
                let limit = if text.to_lowercase().replace('ё', "е").contains("все ход") {
                    self.moves.len().max(2)
                } else {
                    12
                };
                Some(self.move_answer(player, limit))
            }
        }
    }

    pub fn check_model_answer(&self, answer: &str) -> Option<String> {
        NO_BOARD.is_match(answer).then(|| self.board_summary())
    }

    pub fn board_summary(&self) -> String {
        let mut text = self.move_answer(Player::Both, 4);
        if self.check {
            text.push_str(" На доске шах.");
        }
        if self.outcome.is_none() {
            let _ = write!(text, " Легальных ходов в позиции: {}.", self.legal.len());
        }
        text
    }

    pub async fn quick_analysis(&self, executable: &Path) -> String {
        let mut text = self.board_summary();
        if self.outcome.is_some() || self.legal.is_empty() {
            return text;
        }
        let depth = self.depth.min(7);
        let analysis = tokio::time::timeout(
            Duration::from_secs(5),
            engine::analyze(executable, self.kind, &self.engine_position, depth),
        )
        .await;
        let Ok(Ok(analysis)) = analysis else {
            text.push_str("\n\nДвижок не успел рассчитать позицию или недоступен.");
            return text;
        };
        let scored_for = self.side.label(self.kind).to_lowercase();
        let name = analysis.engine.as_deref().unwrap_or("Движок");
        let _ = write!(
            text,
            "\n\n**{name} · глубина {}.**",
            analysis.depth.unwrap_or(depth)
        );
        if let Some(score) = analysis.score {
            match score {
                engine::Score::Centipawns(score) => {
                    let _ = write!(
                        text,
                        " Оценка за сторону хода ({scored_for}) — `{score:+} cp`."
                    );
                }
                engine::Score::Mate(mate) => {
                    let _ = write!(
                        text,
                        " Мат по расчёту за сторону хода ({scored_for}) — `{mate:+}`."
                    );
                }
            }
        }
        if analysis.best == "win"
            && let Ok(game) = Game::new(self.kind, &self.position, self.user, 1, depth, true)
            && game.can_declare()
        {
            text.push_str("\n\nДвижок предлагает заявить победу входящего короля.");
        } else if self.legal.contains(&analysis.best) {
            let _ = write!(text, "\n\nРекомендуемый ход — `{}`.", analysis.best);
            if let Ok(game) = Game::new(self.kind, &self.position, self.user, 1, depth, true) {
                let line = game.verified_line(&analysis.pv[..analysis.pv.len().min(4)]);
                if line.len() > 1 {
                    let _ = write!(text, " Вариант — `{}`.", line.join(" "));
                }
            }
        }
        text
    }

    pub fn model_context(&self) -> String {
        let status = if self.outcome.is_some() {
            self.status()
        } else {
            format!("Ходят {}", self.side.label(self.kind).to_lowercase())
        };
        let roles = if self.study {
            "Режим анализа, обе стороны доступны игроку.".to_owned()
        } else {
            format!(
                "Рей играет за {}. Пользователь играет за {}.",
                self.user.other().label(self.kind),
                self.user.label(self.kind)
            )
        };
        let mut facts = format!(
            "{} · {status}\n{roles}\nНа доске {}. Код позиции: {}.\n",
            self.kind.name(),
            if self.check {
                "шах"
            } else {
                "нет шаха"
            },
            self.position
        );
        let _ = writeln!(facts, "Недавние ходы (всего {}):", self.moves.len());
        for movement in self.moves.iter().skip(self.moves.len().saturating_sub(6)) {
            let _ = writeln!(facts, "- {}", self.move_line(movement, true));
        }
        if let Some(piece) = self.last_piece {
            let name = match piece.simple.trim_start_matches('+') {
                "P" => "пешка",
                "N" => "конь",
                "B" => "слон",
                "R" => "ладья",
                "Q" => "ферзь",
                "K" => "король",
                "L" => "копьё",
                "G" => "золотой генерал",
                "S" => "серебряный генерал",
                _ => piece.symbol,
            };
            let _ = writeln!(
                facts,
                "На клетке назначения последнего хода — {name}{}.",
                if piece.promoted {
                    " в превращённом виде"
                } else {
                    ""
                }
            );
        }
        let _ = writeln!(
            facts,
            "Легальных ходов {}. Примеры: {}.",
            self.legal.len(),
            self.legal
                .iter()
                .take(8)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        );
        if let Some(analysis) = self.analysis.as_deref().and_then(|analysis| {
            analysis
                .strip_prefix("Позиция анализа: ")?
                .strip_prefix(&self.position)?
                .strip_prefix(". ")
        }) {
            let _ = writeln!(facts, "Расчёт движка: {analysis}");
        }
        facts
    }

    fn status(&self) -> String {
        turn_status(
            self.kind,
            self.side,
            self.user,
            self.study,
            self.outcome.as_ref(),
        )
    }

    fn move_line(&self, movement: &PlayedMove, model: bool) -> String {
        let player = if self.study {
            "анализ"
        } else if movement.side == self.user {
            if model {
                "пользователь"
            } else {
                "ты"
            }
        } else {
            "Рей"
        };
        let side = movement.side.label(self.kind);
        if movement.label == movement.notation {
            format!(
                "{} · {side} ({player}) — `{}`",
                movement.number, movement.label
            )
        } else {
            format!(
                "{} · {side} ({player}) — `{}` (`{}`)",
                movement.number, movement.label, movement.notation
            )
        }
    }

    fn move_answer(&self, player: Player, limit: usize) -> String {
        let mut matching = self
            .moves
            .iter()
            .rev()
            .filter(|movement| match player {
                Player::Both => true,
                Player::User => movement.side == self.user,
                Player::Rey => movement.side != self.user,
            })
            .take(limit)
            .collect::<Vec<_>>();
        if matching.is_empty() {
            if self.moves.is_empty() {
                return format!("Партия открыта. Пока ходов нет. {}.", self.status());
            }
            let side = if player == Player::Rey {
                self.user.other()
            } else {
                self.user
            };
            return format!(
                "{} пока не сделали ход. {}.",
                side.label(self.kind),
                self.status()
            );
        }
        matching.reverse();
        let heading = match (player, limit) {
            (Player::Both, 1) => "Последний ход".to_owned(),
            (Player::User, 1) => "Твой последний ход".to_owned(),
            (Player::Rey, 1) => "Последний ход Рей".to_owned(),
            (Player::Both, _) => format!(
                "Ходы текущей партии · показано {} из {}",
                matching.len(),
                self.moves.len()
            ),
            (Player::User, _) => format!("Твои последние ходы · {}", matching.len()),
            (Player::Rey, _) => format!("Последние ходы Рей · {}", matching.len()),
        };
        let rows = matching
            .into_iter()
            .map(|movement| format!("- {}", self.move_line(movement, false)))
            .collect::<Vec<_>>()
            .join("\n");
        format!("**{heading}**\n\n{rows}\n\n{}.", self.status())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_from_current_moves_instead_of_asking_the_model() {
        let mut game = Game::new(Kind::Chess, "", Side::First, 10, 7, false).unwrap();
        game.play("e2e4").unwrap();
        game.play("e7e5").unwrap();
        let snapshot = game.snapshot();
        let seen = snapshot
            .factual_answer("Ты видишь, какими были мои ходы?")
            .unwrap();
        assert!(seen.contains("e2e4") && !seen.contains("e7e5"));
        let last = snapshot
            .factual_answer("Каким был твой последний ход?")
            .unwrap();
        assert!(last.contains("e7e5") && !last.contains("e2e4"));
        assert_eq!(snapshot.factual_answer("Чей ход?").unwrap(), "Твой ход");
        assert!(
            snapshot
                .factual_answer("Какие допустимые ходы?")
                .unwrap()
                .contains("g1f3")
        );
        assert!(
            snapshot
                .factual_answer("Почему твой последний ход был хорошим?")
                .is_none()
        );
        assert!(
            snapshot
                .check_model_answer("К сожалению, я не могу получить доступ к вашей партии шахмат")
                .unwrap()
                .contains("e2e4")
        );
        assert!(
            snapshot
                .check_model_answer("Не могу дать точную оценку: движок ещё не считал эту позицию.")
                .is_none()
        );
        assert!(
            snapshot
                .model_context()
                .contains("Белые (пользователь) — `e4` (`e2e4`)")
        );
        assert!(GameSnapshot::concerns_game("Объясни мой последний ход"));
        assert!(!GameSnapshot::concerns_game(
            "Расскажи про архитектуру проекта"
        ));
    }

    #[test]
    fn imported_games_keep_starting_side_and_turn_number() {
        let mut chess = Game::new(
            Kind::Chess,
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 7",
            Side::Second,
            10,
            7,
            true,
        )
        .unwrap();
        chess.play("e7e5").unwrap();
        chess.play("g1f3").unwrap();
        assert_eq!(
            (chess.move_number(0), chess.move_side(0)),
            (7, Side::Second)
        );
        assert_eq!((chess.move_number(1), chess.move_side(1)), (8, Side::First));
        let mut shogi = Game::new(Kind::Shogi, "", Side::First, 10, 7, true).unwrap();
        shogi.play("7g7f").unwrap();
        shogi.play("3c3d").unwrap();
        assert_eq!(shogi.move_number(1), 2);
        assert!(
            shogi
                .snapshot()
                .factual_answer("Покажи ходы")
                .unwrap()
                .contains("7g7f")
        );
    }
}
