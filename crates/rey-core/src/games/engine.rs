use anyhow::{Context, Result, bail, ensure};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
    time::timeout,
};

use super::Kind;

#[derive(Debug)]
pub struct Analysis {
    pub engine: Option<String>,
    pub best: String,
    pub score: Option<Score>,
    pub depth: Option<u8>,
    pub pv: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum Score {
    Centipawns(i32),
    Mate(i32),
}

pub async fn analyze(executable: &Path, kind: Kind, position: &str, depth: u8) -> Result<Analysis> {
    ensure!(
        position.len() <= 32 * 1024 && position.bytes().all(|b| b.is_ascii_graphic() || b == b' '),
        "Неподдерживаемая строка позиции"
    );
    let executable = check_executable(executable)?;
    timeout(
        Duration::from_secs(25),
        run(&executable, kind, position, depth.clamp(1, 22)),
    )
    .await
    .context("Движок не ответил за 25 секунд")?
}

pub fn check_executable(path: &Path) -> Result<PathBuf> {
    let executable = locate(path).with_context(|| {
        format!(
            "Движок {} не найден. Проверь путь в настройках",
            path.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            executable.metadata()?.permissions().mode() & 0o111 != 0,
            "У файла движка {} нет разрешения на запуск",
            executable.display()
        );
    }
    Ok(executable)
}

fn locate(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return path.canonicalize().ok();
    }
    if path.components().count() != 1 {
        return None;
    }
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|directory| {
        let file = directory.join(path);
        if file.is_file() {
            return file.canonicalize().ok();
        }
        if cfg!(windows) && file.extension().is_none() {
            let file = file.with_extension("exe");
            if file.is_file() {
                return file.canonicalize().ok();
            }
        }
        None
    })
}

async fn run(executable: &Path, kind: Kind, position: &str, depth: u8) -> Result<Analysis> {
    let mut command = Command::new(executable);
    command
        .current_dir(executable.parent().context("Нет каталога движка")?)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for key in ["SYSTEMROOT", "WINDIR", "TEMP", "TMP", "LD_LIBRARY_PATH"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let mut child = command
        .spawn()
        .context("Не удалось запустить игровой движок")?;
    let mut output = BufReader::new(child.stdout.take().context("Нет stdout движка")?);
    let mut input = child.stdin.take().context("Нет stdin движка")?;
    let protocol = if kind == Kind::Chess { "uci" } else { "usi" };
    send(&mut input, protocol).await?;
    let ok = format!("{protocol}ok");
    let mut options = Vec::new();
    let mut analysis = Analysis {
        engine: None,
        best: String::new(),
        score: None,
        depth: None,
        pv: Vec::new(),
    };
    let mut buffer = Vec::new();
    loop {
        let line = receive(&mut output, &mut buffer).await?;
        if line == ok {
            break;
        }
        if let Some(name) = line.strip_prefix("id name ") {
            analysis.engine = Some(
                name.chars()
                    .filter(|character| !character.is_control())
                    .take(160)
                    .collect(),
            );
        }
        if let Some(option) = line
            .strip_prefix("option name ")
            .and_then(|line| line.split_once(" type "))
            && options.len() < 512
        {
            options.push(option.0.to_owned());
        }
    }
    for (name, value) in [
        ("Threads", "1"),
        ("Hash", "64"),
        ("USI_Hash", "64"),
        ("MultiPV", "1"),
        ("USI_Ponder", "false"),
    ] {
        if options.iter().any(|option| option == name) {
            send(&mut input, &format!("setoption name {name} value {value}")).await?;
        }
    }
    send(&mut input, "isready").await?;
    while receive(&mut output, &mut buffer).await? != "readyok" {}
    if kind == Kind::Shogi {
        send(&mut input, "usinewgame").await?;
    } else {
        send(&mut input, "ucinewgame").await?;
    }
    send(&mut input, &format!("position {position}")).await?;
    send(&mut input, &format!("go depth {depth}")).await?;
    loop {
        let line = receive(&mut output, &mut buffer).await?;
        if let Some(rest) = line.strip_prefix("bestmove ") {
            let best = rest
                .split_whitespace()
                .next()
                .context("Пустой ход движка")?;
            ensure!(
                valid_move(kind, best) || matches!(best, "(none)" | "resign" | "win" | "0000"),
                "Некорректный формат хода движка"
            );
            analysis.best = best.to_owned();
            break;
        }
        if let Some(info) = line.strip_prefix("info ") {
            parse_info(kind, info, &mut analysis);
        }
    }
    let _ = send(&mut input, "quit").await;
    if timeout(Duration::from_millis(500), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
    Ok(analysis)
}

async fn receive<'a>(
    reader: &mut BufReader<ChildStdout>,
    buffer: &'a mut Vec<u8>,
) -> Result<&'a str> {
    buffer.clear();
    let n = reader.take(8193).read_until(b'\n', buffer).await?;
    if n == 0 {
        bail!("Движок завершился до ответа");
    }
    ensure!(
        n <= 8192 && buffer.last() == Some(&b'\n'),
        "Слишком длинный или оборванный ответ движка"
    );
    Ok(std::str::from_utf8(buffer).unwrap_or("").trim())
}

async fn send(input: &mut ChildStdin, line: &str) -> Result<()> {
    input.write_all(line.as_bytes()).await?;
    input.write_all(b"\n").await?;
    input.flush().await?;
    Ok(())
}

fn valid_move(kind: Kind, movement: &str) -> bool {
    match kind {
        Kind::Chess => movement.parse::<cozy_chess::Move>().is_ok(),
        Kind::Shogi => {
            <shogi_core::Move as shogi_usi_parser::FromUsi>::from_usi_lite(movement).is_some()
        }
    }
}

fn parse_info(kind: Kind, text: &str, analysis: &mut Analysis) {
    let words: Vec<_> = text.split_whitespace().collect();
    if words.first() == Some(&"string") {
        return;
    }
    let field = |name| words.iter().position(|entry| *entry == name);
    let Some(pv) = field("pv") else { return };
    let depth = field("depth")
        .and_then(|i| words.get(i + 1))
        .and_then(|word| word.parse::<u8>().ok());
    if depth < analysis.depth {
        return;
    }
    analysis.depth = depth;
    analysis.pv = words[pv + 1..]
        .iter()
        .take(12)
        .take_while(|word| valid_move(kind, word))
        .map(|word| (*word).to_string())
        .collect();
    analysis.score = field("score").and_then(|i| {
        match (*words.get(i + 1)?, words.get(i + 2)?.parse::<i32>().ok()?) {
            ("cp", value) => Some(Score::Centipawns(value)),
            ("mate", value) => Some(Score::Mate(value)),
            _ => None,
        }
    });
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[tokio::test]
    async fn uci_and_usi_handshakes_use_real_processes() {
        let folder = tempfile::tempdir().unwrap();
        let executable = folder.path().join("fixture");
        std::fs::write(&executable,"#!/bin/sh\nwhile read -r command; do\n case \"$command\" in\n uci) move=e2e4; printf 'id name Test UCI\\nuciok\\n';;\n usi) move=7g7f; printf 'id name Test USI\\nusiok\\n';;\n isready) echo readyok;;\n go*) echo \"info depth 5 score cp 12 pv $move\"; echo \"bestmove $move\";;\n quit) exit 0;;\n esac\ndone\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        for (kind, movement) in [(Kind::Chess, "e2e4"), (Kind::Shogi, "7g7f")] {
            let result = analyze(&executable, kind, "startpos", 5).await.unwrap();
            assert_eq!(result.best, movement);
            assert_eq!(result.depth, Some(5));
            assert!(matches!(result.score, Some(Score::Centipawns(12))));
        }
        assert!(
            analyze(&executable, Kind::Chess, "startpos\nquit", 4)
                .await
                .is_err()
        );
    }
}
