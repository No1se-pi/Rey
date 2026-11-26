use anyhow::{Context, Result, ensure};
use shogi_core::{Color, Hand, Move, PartialPosition, Piece, PieceKind, Square, ToUsi};
use shogi_legality_lite::{
    all_legal_moves_partial, is_in_check_partial_lite, is_legal_partial, status_partial,
};
use shogi_usi_parser::FromUsi;

use super::{LegalMove, Origin, Outcome, PieceView, Side};

pub(super) struct Shogi {
    pub board: PartialPosition,
    history: Vec<(String, Option<(Side, bool)>)>,
}

pub(super) fn side(color: Color) -> Side {
    if color == Color::Black {
        Side::First
    } else {
        Side::Second
    }
}

fn color(side: Side) -> Color {
    if side == Side::First {
        Color::Black
    } else {
        Color::White
    }
}

fn ui_square(sq: Square) -> usize {
    usize::from(sq.rank() - 1) * 9 + usize::from(9 - sq.file())
}

fn square(ui: usize) -> Option<Square> {
    if ui >= 81 {
        return None;
    }
    Square::new(9 - (ui % 9) as u8, (ui / 9 + 1) as u8)
}

fn piece_view(piece: Piece) -> PieceView {
    use PieceKind::*;
    let (kind, color) = piece.to_parts();
    let (symbol, simple) = match kind {
        Pawn => ("歩", "P"),
        Lance => ("香", "L"),
        Knight => ("桂", "N"),
        Silver => ("銀", "S"),
        Gold => ("金", "G"),
        Bishop => ("角", "B"),
        Rook => ("飛", "R"),
        King => ("玉", "K"),
        ProPawn => ("と", "+P"),
        ProLance => ("杏", "+L"),
        ProKnight => ("圭", "+N"),
        ProSilver => ("全", "+S"),
        ProBishop => ("馬", "+B"),
        ProRook => ("龍", "+R"),
    };
    PieceView {
        side: side(color),
        symbol,
        simple,
        promoted: kind.unpromote().is_some(),
    }
}

fn position_key(board: &PartialPosition) -> String {
    let sfen = board.to_sfen_owned();
    sfen.rsplit_once(' ')
        .map(|(position, _)| position.to_owned())
        .unwrap_or(sfen)
}

fn parse_position(sfen: &str) -> Result<PartialPosition> {
    match sfen.trim() {
        "" | "startpos" => Ok(PartialPosition::startpos()),
        value => {
            let prefixed = if value.starts_with("sfen ") {
                value.to_owned()
            } else {
                format!("sfen {value}")
            };
            PartialPosition::from_usi_lite(&prefixed).context("Некорректный SFEN")
        }
    }
}

fn parse_move(board: &PartialPosition, notation: &str) -> Result<Move> {
    let movement = Move::from_usi_lite(notation).context("Некорректный ход USI")?;
    Ok(match movement {
        Move::Drop { piece, to } => Move::Drop {
            piece: Piece::new(piece.piece_kind(), board.side_to_move()),
            to,
        },
        movement => movement,
    })
}

impl Shogi {
    pub fn new(sfen: &str) -> Result<Self> {
        let board = parse_position(sfen)?;
        ensure!(
            Square::all()
                .filter(|sq| board.piece_at(*sq) == Some(Piece::new(PieceKind::King, Color::Black)))
                .count()
                == 1
                && Square::all()
                    .filter(
                        |sq| board.piece_at(*sq) == Some(Piece::new(PieceKind::King, Color::White))
                    )
                    .count()
                    == 1,
            "В SFEN должен быть один король каждой стороны"
        );
        ensure!(
            !matches!(status_partial(&board), shogi_core::PositionStatus::Invalid),
            "В SFEN превышено допустимое количество фигур"
        );
        let mut opponent = board.clone();
        opponent.side_to_move_set(board.side_to_move().flip());
        ensure!(
            !is_in_check_partial_lite(&opponent),
            "Сторона без хода находится под шахом"
        );
        let history = vec![(position_key(&board), None)];
        Ok(Self { board, history })
    }

    pub fn piece(&self, ui: usize) -> Option<PieceView> {
        self.board.piece_at(square(ui)?).map(piece_view)
    }

    pub fn hand(&self, side: Side) -> Vec<(char, PieceView, u8)> {
        let hand = self.board.hand_of_a_player(color(side));
        Hand::all_hand_pieces()
            .filter_map(|kind| {
                let count = hand.count(kind)?;
                let view = piece_view(Piece::new(kind, color(side)));
                (count > 0).then(|| (view.simple.chars().next().expect("piece code"), view, count))
            })
            .collect()
    }

    pub fn checked_king(&self) -> Option<usize> {
        if !is_in_check_partial_lite(&self.board) {
            return None;
        }
        self.board
            .king_position(self.board.side_to_move())
            .map(ui_square)
    }

    pub fn legal(&self) -> Vec<LegalMove> {
        all_legal_moves_partial(&self.board)
            .into_iter()
            .map(|movement| {
                let notation = movement.to_usi_owned();
                let (origin, option) = match movement {
                    Move::Normal { from, promote, .. } => (
                        Origin::Square(ui_square(from)),
                        if promote {
                            "Превратить"
                        } else {
                            "Сохранить"
                        },
                    ),
                    Move::Drop { .. } => (Origin::Hand(notation.as_bytes()[0] as char), "Сбросить"),
                };
                LegalMove {
                    origin,
                    target: ui_square(movement.to()),
                    notation,
                    option,
                }
            })
            .collect()
    }

    pub fn play(&mut self, notation: &str) -> Result<String> {
        let movement = parse_move(&self.board, notation)?;
        is_legal_partial(&self.board, movement)
            .map_err(|err| anyhow::anyhow!("Нелегальный ход: {err:?}"))?;
        let player = side(self.board.side_to_move());
        self.board
            .make_move(movement)
            .context("Не удалось применить ход")?;
        self.history.push((
            position_key(&self.board),
            Some((player, is_in_check_partial_lite(&self.board))),
        ));
        Ok(notation.to_string())
    }

    pub fn outcome(&self) -> Option<Outcome> {
        if all_legal_moves_partial(&self.board).is_empty() {
            return Some(Outcome {
                winner: Some(side(self.board.side_to_move().flip())),
                reason: if self.checked_king().is_some() {
                    "Мат"
                } else {
                    "Нет легальных ходов"
                },
            });
        }
        let current = &self.history.last()?.0;
        let occurrences: Vec<_> = self
            .history
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.0 == *current)
            .map(|(index, _)| index)
            .collect();
        if occurrences.len() < 4 {
            return None;
        }
        let cycle = &self.history[occurrences[occurrences.len() - 4] + 1..];
        let perpetual = [Side::First, Side::Second].into_iter().find(|player| {
            cycle
                .iter()
                .filter_map(|(_, movement)| *movement)
                .filter(|(mover, _)| mover == player)
                .all(|(_, check)| check)
        });
        Some(Outcome {
            winner: perpetual.map(Side::other),
            reason: if perpetual.is_some() {
                "Непрерывный шах"
            } else {
                "Сэннититэ"
            },
        })
    }

    fn camp(sq: Square, owner: Color) -> bool {
        if owner == Color::Black {
            sq.rank() <= 3
        } else {
            sq.rank() >= 7
        }
    }

    fn points(&self, owner: Color, camp_only: bool) -> (usize, u16) {
        let board = Square::all()
            .filter(|&sq| !camp_only || Self::camp(sq, owner))
            .filter_map(|sq| self.board.piece_at(sq))
            .filter(|piece| piece.color() == owner && piece.piece_kind() != PieceKind::King);
        let (count, board_points) = board.fold((0, 0), |(count, points), piece| {
            (count + 1, points + value(piece.piece_kind()))
        });
        let hand = self.board.hand_of_a_player(owner);
        let hand_points: u16 = Hand::all_hand_pieces()
            .map(|kind| u16::from(hand.count(kind).unwrap_or(0)) * value(kind))
            .sum();
        (count, board_points + hand_points)
    }

    pub fn can_declare(&self) -> bool {
        let owner = self.board.side_to_move();
        let (count, points) = self.points(owner, true);
        self.board
            .king_position(owner)
            .is_some_and(|square| Self::camp(square, owner))
            && self.checked_king().is_none()
            && count >= 10
            && points >= if owner == Color::Black { 28 } else { 27 }
    }

    pub fn impasse(&self) -> Option<Option<Side>> {
        if self.checked_king().is_some()
            || ![Color::Black, Color::White].into_iter().all(|owner| {
                self.board
                    .king_position(owner)
                    .is_some_and(|square| Self::camp(square, owner))
            })
        {
            return None;
        }
        let first = self.points(Color::Black, false).1 >= 24;
        let second = self.points(Color::White, false).1 >= 24;
        Some(match (first, second) {
            (true, false) => Some(Side::First),
            (false, true) => Some(Side::Second),
            _ => None,
        })
    }
}

fn value(kind: PieceKind) -> u16 {
    match kind.unpromote().unwrap_or(kind) {
        PieceKind::Bishop | PieceKind::Rook => 5,
        PieceKind::King => 0,
        _ => 1,
    }
}

fn csa_piece(kind: PieceKind) -> &'static str {
    use PieceKind::*;
    match kind {
        Pawn => "FU",
        Lance => "KY",
        Knight => "KE",
        Silver => "GI",
        Gold => "KI",
        Bishop => "KA",
        Rook => "HI",
        King => "OU",
        ProPawn => "TO",
        ProLance => "NY",
        ProKnight => "NK",
        ProSilver => "NG",
        ProBishop => "UM",
        ProRook => "RY",
    }
}

pub(super) fn export_csa(
    initial: &str,
    moves: &[String],
    user: Side,
    outcome: Option<&Outcome>,
) -> String {
    let mut board = parse_position(initial).expect("validated SFEN");
    let mut csa = format!(
        "V2.2\nN+{}\nN-{}\n",
        if user == Side::First { "Player" } else { "Rey" },
        if user == Side::Second {
            "Player"
        } else {
            "Rey"
        }
    );
    let sign = |c| if c == Color::Black { '+' } else { '-' };
    for rank in 1..=9 {
        csa.push_str(&format!("P{rank}"));
        for file in (1..=9).rev() {
            if let Some(piece) = board.piece_at(Square::new(file, rank).unwrap()) {
                csa.push(sign(piece.color()));
                csa.push_str(csa_piece(piece.piece_kind()));
            } else {
                csa.push_str(" * ");
            }
        }
        csa.push('\n');
    }
    for c in [Color::Black, Color::White] {
        let mut hand = format!("P{}", sign(c));
        for kind in Hand::all_hand_pieces() {
            for _ in 0..board.hand_of_a_player(c).count(kind).unwrap_or(0) {
                hand.push_str("00");
                hand.push_str(csa_piece(kind));
            }
        }
        if hand.len() > 2 {
            csa.push_str(&hand);
            csa.push('\n');
        }
    }
    csa.push(sign(board.side_to_move()));
    csa.push('\n');
    for notation in moves {
        let movement = parse_move(&board, notation).expect("validated USI");
        csa.push(sign(board.side_to_move()));
        if let Some(from) = movement.from() {
            csa.push_str(&format!("{}{}", from.file(), from.rank()));
        } else {
            csa.push_str("00");
        }
        let to = movement.to();
        board.make_move(movement).expect("validated movement");
        csa.push_str(&format!(
            "{}{}{}\n",
            to.file(),
            to.rank(),
            csa_piece(board.piece_at(to).unwrap().piece_kind())
        ));
    }
    if let Some(outcome) = outcome {
        csa.push_str(match outcome.reason {
            "Сдача" => "%TORYO\n",
            "Мат" => "%TSUMI\n",
            "Сэннититэ" => "%SENNICHITE\n",
            "Непрерывный шах" => "%ILLEGAL_MOVE\n",
            "Входящий король" => "%KACHI\n",
            "Дзисёги" => "%JISHOGI\n",
            "Время истекло" => "%TIME_UP\n",
            _ => "%CHUDAN\n",
        });
    }
    csa
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shogi_drop_and_promotion_constraints() {
        let game = Shogi::new("4k4/9/4P4/9/9/9/9/9/4K4 b PN 1").unwrap();
        let legal = game.legal();
        assert!(
            !legal
                .iter()
                .any(|m| m.notation == "P*5d" || m.notation == "N*4b")
        );
        assert!(legal.iter().any(|m| m.notation == "P*4d"));
        assert!(
            legal.iter().any(|m| m.notation == "5c5b+")
                && legal.iter().any(|m| m.notation == "5c5b")
        );
        let game = Shogi::new("k8/4P4/9/9/9/9/9/9/4K4 b - 1").unwrap();
        assert!(game.legal().iter().any(|m| m.notation == "5b5a+"));
        assert!(!game.legal().iter().any(|m| m.notation == "5b5a"));
    }

    #[test]
    fn pawn_drop_mate_is_illegal_and_sennichite_ends_game() {
        let mate = Shogi::new("3lkl3/3p1p3/4G4/9/9/9/9/9/4K4 b P 1").unwrap();
        assert!(!mate.legal().iter().any(|m| m.notation == "P*5b"));
        let mut repeated = Shogi::new("4k4/9/9/9/9/9/9/9/4K4 b - 1").unwrap();
        for movement in ["5i4i", "5a4a", "4i5i", "4a5a"]
            .into_iter()
            .cycle()
            .take(12)
        {
            repeated.play(movement).unwrap();
        }
        assert_eq!(repeated.outcome().unwrap().reason, "Сэннититэ");
        let mut checked = Shogi::new("4k4/4R4/9/9/9/9/9/9/5K3 w - 1").unwrap();
        for movement in ["5a4a", "5b4b", "4a5a", "4b5b"]
            .into_iter()
            .cycle()
            .take(12)
        {
            checked.play(movement).unwrap();
        }
        assert_eq!(checked.outcome().unwrap().winner, Some(Side::Second));
    }
}
