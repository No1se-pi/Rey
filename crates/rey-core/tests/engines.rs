use rey_core::games::{Game, Kind, Side, engine};
use std::path::PathBuf;

#[tokio::test]
#[ignore = "requires installed Stockfish and YaneuraOu"]
async fn real_engines_return_playable_moves() {
    for (variable, kind) in [
        ("REY_CHESS_ENGINE", Kind::Chess),
        ("REY_SHOGI_ENGINE", Kind::Shogi),
    ] {
        let executable = PathBuf::from(std::env::var_os(variable).expect(variable));
        let mut game = Game::new(kind, "", Side::First, 10, 4, true).unwrap();
        for _ in 0..3 {
            let analysis = engine::analyze(&executable, kind, &game.engine_position(), 4)
                .await
                .unwrap();
            println!(
                "{} · {}",
                analysis.engine.as_deref().unwrap_or("engine"),
                analysis.best
            );
            assert!(analysis.engine.is_some());
            game.play(&analysis.best).unwrap();
        }
        assert_eq!(game.moves.len(), 3);
    }
}
