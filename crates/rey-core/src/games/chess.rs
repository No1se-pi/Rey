use anyhow::{Context, Result, ensure};
use cozy_chess::{Board, Color, Piece, Square, util};
use regex::Regex;
use std::sync::LazyLock;

use super::{LegalMove, Origin, Outcome, PieceView, Side};

pub(super) struct Chess {
    pub board: Board,
    history: Vec<Board>,
    halfmoves: u32,
}

pub(super) fn side(color: Color) -> Side {
    if color == Color::White {
        Side::First
    } else {
        Side::Second
    }
}

fn ui_square(square: Square) -> usize {
    (7 - square.rank() as usize) * 8 + square.file() as usize
}

impl Chess {
    pub fn new(fen: &str) -> Result<Self> {
        let (board, halfmoves) = if fen.trim().is_empty() {
            (Board::default(), 0)
        } else {
            let mut fields: Vec<_> = fen.split_whitespace().collect();
            ensure!(fields.len() == 6, "FEN должен содержать шесть полей");
            let halfmoves: u32 = fields[4]
                .parse()
                .context("Некорректный счётчик полуходов FEN")?;
            // Cozy Chess caps its internal counter at the 50-move claim threshold.
            fields[4] = "0";
            (
                fields.join(" ").parse().context("Некорректный FEN")?,
                halfmoves,
            )
        };
        Ok(Self {
            history: vec![board.clone()],
            board,
            halfmoves,
        })
    }

    pub fn fen(&self) -> String {
        let raw = self.board.to_string();
        let mut fields: Vec<_> = raw.split_whitespace().collect();
        let count = self.halfmoves.to_string();
        fields[4] = &count;
        fields.join(" ")
    }

    pub fn piece(&self, square: usize) -> Option<PieceView> {
        let rank = 7_usize.checked_sub(square / 8)?;
        let square = Square::try_index(rank * 8 + square % 8)?;
        let color = self.board.color_on(square)?;
        let piece = self.board.piece_on(square)?;
        let glyphs = match color {
            Color::White => ["♙", "♘", "♗", "♖", "♕", "♔"],
            Color::Black => ["♟", "♞", "♝", "♜", "♛", "♚"],
        };
        Some(PieceView {
            side: side(color),
            symbol: glyphs[piece as usize],
            simple: ["P", "N", "B", "R", "Q", "K"][piece as usize],
            promoted: false,
        })
    }

    pub fn checked_king(&self) -> Option<usize> {
        (!self.board.checkers().is_empty())
            .then(|| ui_square(self.board.king(self.board.side_to_move())))
    }

    pub fn legal(&self) -> Vec<LegalMove> {
        let mut moves = Vec::new();
        self.board.generate_moves(|set| {
            moves.extend(set.into_iter().map(|movement| {
                let notation = util::display_uci_move(&self.board, movement).to_string();
                let target = notation[2..4].parse::<Square>().expect("UCI target square");
                LegalMove {
                    origin: Origin::Square(ui_square(movement.from)),
                    target: ui_square(target),
                    notation,
                    option: match movement.promotion {
                        Some(Piece::Queen) => "Ферзь",
                        Some(Piece::Rook) => "Ладья",
                        Some(Piece::Bishop) => "Слон",
                        Some(Piece::Knight) => "Конь",
                        _ => "Ход",
                    },
                }
            }));
            false
        });
        moves
    }

    pub fn play(&mut self, notation: &str) -> Result<String> {
        let movement = util::parse_uci_move(&self.board, notation)?;
        ensure!(self.board.is_legal(movement), "Нелегальный ход");
        let label = util::display_san_move(&self.board, movement).to_string();
        self.board.try_play(movement)?;
        self.halfmoves = if self.board.halfmove_clock() == 0 {
            0
        } else {
            self.halfmoves.saturating_add(1)
        };
        self.history.push(self.board.clone());
        Ok(label)
    }

    pub fn can_claim(&self, intended: Option<&str>) -> bool {
        if let Some(uci) = intended {
            let Ok(movement) = util::parse_uci_move(&self.board, uci) else {
                return false;
            };
            let mut next = self.board.clone();
            if next.try_play(movement).is_err() {
                return false;
            }
            self.repetitions(&next) >= 2 || (next.halfmove_clock() > 0 && self.halfmoves >= 99)
        } else {
            self.repetitions(&self.board) >= 3 || self.halfmoves >= 100
        }
    }

    fn repetitions(&self, board: &Board) -> usize {
        self.history
            .iter()
            .filter(|past| past.same_position(board))
            .count()
    }

    pub fn outcome(&self) -> Option<Outcome> {
        let result = |winner, reason| Some(Outcome { winner, reason });
        if !self.board.generate_moves(|_| true) {
            if self.board.checkers().is_empty() {
                return result(None, "Пат");
            }
            return result(Some(side(!self.board.side_to_move())), "Мат");
        }
        if self.halfmoves >= 150 {
            return result(None, "Правило 75 ходов");
        }
        if self.repetitions(&self.board) >= 5 {
            return result(None, "Пятикратное повторение");
        }
        if !self.has_mating_material(Side::First) && !self.has_mating_material(Side::Second) {
            return result(None, "Недостаточный материал");
        }
        None
    }

    pub fn has_mating_material(&self, side: Side) -> bool {
        let color = if side == Side::First {
            Color::White
        } else {
            Color::Black
        };
        let ours = self.board.colors(color);
        let theirs = self.board.colors(!color);
        let pieces = |kind| self.board.pieces(kind);
        if !(ours & (pieces(Piece::Pawn) | pieces(Piece::Queen) | pieces(Piece::Rook))).is_empty() {
            return true;
        }
        if !(ours & pieces(Piece::Knight)).is_empty() {
            return (ours & !pieces(Piece::King)).len() > 1
                || !(theirs & !(pieces(Piece::King) | pieces(Piece::Queen))).is_empty();
        }
        if (ours & pieces(Piece::Bishop)).is_empty() {
            return false;
        }
        let mut colors = self
            .board
            .pieces(Piece::Bishop)
            .into_iter()
            .map(|sq| (sq.file() as u8 + sq.rank() as u8) % 2);
        let first = colors.next().unwrap_or_default();
        colors.any(|color| color != first)
            || !(self.board.occupied() & (pieces(Piece::Pawn) | pieces(Piece::Knight))).is_empty()
    }
}

static FEN_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^\s*\[FEN\s+"([^"]+)"\s*\]"#).unwrap());
static MOVE_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+\.{1,3}").unwrap());

pub(super) struct Pgn {
    pub fen: String,
    pub moves: Vec<String>,
    pub result: Option<&'static str>,
}

pub(super) fn pgn_moves(pgn: &str) -> Result<Pgn> {
    ensure!(pgn.len() <= 1024 * 1024, "PGN превышает 1 МиБ");
    let headers = FEN_TAG
        .captures(pgn)
        .map(|m| m[1].to_owned())
        .unwrap_or_default();
    let mut main = String::new();
    let mut variation = 0_u32;
    let mut comment = false;
    let mut line = false;
    for text in pgn
        .lines()
        .filter(|line| !line.trim_start().starts_with(['[', '%']))
    {
        for ch in text.chars() {
            if line {
                continue;
            }
            if comment {
                if ch == '}' {
                    comment = false;
                }
                continue;
            }
            match ch {
                ';' => {
                    line = true;
                    main.push(' ');
                }
                '{' => {
                    comment = true;
                    main.push(' ');
                }
                '(' => {
                    variation += 1;
                    main.push(' ');
                }
                ')' => {
                    variation = variation.checked_sub(1).context("Лишняя скобка в PGN")?;
                    main.push(' ');
                }
                _ if variation == 0 => main.push(ch),
                _ => {}
            }
        }
        main.push(' ');
        line = false;
    }
    ensure!(
        !comment && variation == 0,
        "Незакрытый комментарий или вариант PGN"
    );
    let mut result = None;
    let mut moves = Vec::new();
    for token in main.split_whitespace() {
        let token = MOVE_NUMBER.replace(token, "");
        let token = token
            .split('$')
            .next()
            .unwrap_or_default()
            .trim_end_matches(['!', '?']);
        match token {
            "1-0" => {
                result = Some("1-0");
                break;
            }
            "0-1" => {
                result = Some("0-1");
                break;
            }
            "1/2-1/2" => {
                result = Some("1/2-1/2");
                break;
            }
            "*" => break,
            "" | "..." | "e.p." => (),
            _ => moves.push(token.replace('0', "O")),
        }
    }
    Ok(Pgn {
        fen: headers,
        moves,
        result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn castling_en_passant_promotions_and_seventy_five_moves() {
        let castle = Chess::new("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        assert!(castle.legal().iter().any(|m| m.notation == "e1g1"));
        let ep = Chess::new("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 2").unwrap();
        assert!(ep.legal().iter().any(|m| m.notation == "e5d6"));
        let promotion = Chess::new("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        assert_eq!(
            promotion
                .legal()
                .iter()
                .filter(|m| m.notation.starts_with("a7a8"))
                .count(),
            4
        );
        let mut near = Chess::new("4k3/8/8/8/8/8/8/R3K3 w - - 149 80").unwrap();
        assert!(near.can_claim(None));
        assert!(near.outcome().is_none());
        near.play("a1a2").unwrap();
        assert_eq!(near.outcome().unwrap().reason, "Правило 75 ходов");
    }

    #[test]
    fn mate_stalemate_and_insufficient_material_are_distinct() {
        let mate = Chess::new("7k/6Q1/6K1/8/8/8/8/8 b - - 0 1").unwrap();
        assert_eq!(mate.outcome().unwrap().winner, Some(Side::First));
        let stalemate = Chess::new("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap();
        assert_eq!(stalemate.outcome().unwrap().reason, "Пат");
        let dead = Chess::new("7k/8/8/8/8/8/8/1B4K1 w - - 0 1").unwrap();
        assert_eq!(dead.outcome().unwrap().reason, "Недостаточный материал");
        let near = Chess::new("4k3/8/8/8/8/8/8/R3K3 w - - 99 50").unwrap();
        assert!(!near.can_claim(None));
        assert!(near.can_claim(Some("a1a2")));
    }

    #[test]
    fn pgn_discards_side_variations() {
        assert_eq!(
            pgn_moves("1.e4! {hello} e5 (1... c5 (1... e6)) 2.Nf3 $1 1-0")
                .unwrap()
                .moves,
            ["e4", "e5", "Nf3"]
        );
    }
}
