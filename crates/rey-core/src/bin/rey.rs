use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rey_core::{
    agent::{Agent, Turn},
    config::Config,
    ghost::Ghost,
    llm::{self, Provider},
    memory::copy_database,
    voice::Voice,
};
use std::{
    io::{self, Write},
    path::PathBuf,
};
use uuid::Uuid;

#[derive(Parser)]
#[command(version, about = "Rey • Ghost & Shell")]
struct Arguments {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[command(subcommand)]
    action: Option<Action>,
}

#[derive(Subcommand)]
enum Action {
    Chat {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        voice: bool,
    },
    Migrate {
        #[arg(long)]
        from: Option<PathBuf>,
        #[arg(long)]
        to: Option<PathBuf>,
    },
    Backup {
        destination: PathBuf,
    },
    Models,
    Ghost,
    Transcribe {
        audio: PathBuf,
    },
    Speak {
        text: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    rey_core::init_logging();
    let arguments = Arguments::parse();
    let config = Config::load(arguments.root)?;
    match arguments.action.unwrap_or(Action::Chat {
        session: None,
        model: None,
        voice: false,
    }) {
        Action::Migrate { from, to } => {
            let destination = to.unwrap_or_else(|| config.database.clone());
            copy_database(&from.unwrap_or(config.legacy_database), &destination, true)?;
            println!("{}", destination.display());
        }
        Action::Backup { destination } => {
            copy_database(&config.database, &destination, false)?;
            println!("{}", destination.display());
        }
        Action::Ghost => {
            let ghost = Ghost::load(&config.root)?;
            println!(
                "{} ({}) · character v{}\nSHA-256 {}",
                ghost.soul.name, ghost.soul.id, ghost.character.version, ghost.digest
            );
        }
        Action::Models => {
            let provider = llm::from_config(&config)?;
            for model in provider.models().await? {
                println!("{model}");
            }
        }
        Action::Transcribe { audio } => {
            println!("{}", Voice::new(config).transcribe(&audio).await?)
        }
        Action::Speak { text, output } => {
            let output = output.unwrap_or_else(|| config.voice_output.clone());
            let audio = Voice::new(config).synthesize(&text).await?;
            println!("{}", audio.archive(&output)?.display());
        }
        Action::Chat {
            session,
            model,
            voice,
        } => {
            chat(
                Agent::open(config)?,
                session.unwrap_or_else(|| Uuid::new_v4().to_string()),
                model,
                voice,
            )
            .await?;
        }
    }
    Ok(())
}

async fn chat(agent: Agent, session: String, model: Option<String>, spoken: bool) -> Result<()> {
    let provider = agent.provider()?;
    let configured = model.as_deref().unwrap_or_else(|| agent.config.model());
    let detected;
    let model = if configured.trim().is_empty() {
        detected = provider
            .models()
            .await?
            .into_iter()
            .next()
            .context("У провайдера нет доступных моделей")?;
        &detected
    } else {
        configured
    };
    let voice = Voice::new(agent.config.clone());
    println!(
        "Рей на связи · {model}\nДиалог {session}\n/exit · /history · /memory · /forget ID · /state"
    );
    let mut lines = io::stdin().lines();
    loop {
        print!("Ты> ");
        io::stdout().flush()?;
        let Some(line) = lines.next() else { break };
        let line = line?;
        let text = line.trim();
        match text {
            "" => continue,
            "/exit" => break,
            "/history" => {
                for message in agent.store.history(&session, agent.config.history)? {
                    println!("{}> {}", message.role, message.content);
                }
            }
            "/memory" => {
                for note in agent.store.notes("", 20)? {
                    println!("#{} {} · {}", note.id, note.title, note.markdown);
                }
            }
            "/state" => println!("idle"),
            other if other.starts_with("/forget ") => match other[8..].trim().parse() {
                Ok(id) => agent.store.delete_note(id)?,
                Err(_) => println!("Укажи числовой ID заметки"),
            },
            _ => {
                let turn = Turn {
                    session: &session,
                    model,
                    text,
                    spoken,
                    game_context: None,
                };
                let result = tokio::select! {
                    answer = agent.reply(provider.as_ref(),turn) => answer,
                    signal = tokio::signal::ctrl_c() => { signal.context("Сигнал отмены недоступен")?; println!("Отменено"); continue; }
                };
                match result {
                    Ok(reply) => {
                        agent.save_reply(
                            &Turn {
                                session: &session,
                                model,
                                text,
                                spoken,
                                game_context: None,
                            },
                            &reply,
                        )?;
                        println!("Рей> {}", reply.text);
                        if spoken
                            && let Err(error) = synthesize(
                                &agent.config,
                                &voice,
                                provider.as_ref(),
                                model,
                                &reply.text,
                            )
                            .await
                        {
                            eprintln!("Аудио: {error:#}");
                        }
                    }
                    Err(error) => eprintln!("Рей: {error:#}"),
                }
            }
        }
    }
    Ok(())
}

async fn synthesize(
    config: &Config,
    voice: &Voice,
    provider: &dyn Provider,
    model: &str,
    text: &str,
) -> Result<()> {
    if config.unload_ollama
        && let Err(error) = provider.unload(model).await
    {
        tracing::warn!(%error,"Ollama model unload failed");
    }
    let audio = voice.synthesize(text).await?;
    let output = audio.archive(&config.voice_output)?;
    println!("Аудио · {}", output.display());
    Ok(())
}
