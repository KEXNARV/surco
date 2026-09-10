//! surco -- reproductor con motor intercambiable.
//!
//! Un proceso hace de daemon (posee mpv y la cola) y el mismo binario, con
//! cualquier otro subcomando, actua de cliente contra su socket.

mod backend;
mod ipc;
mod player;
mod resolver;

use anyhow::{Context, Result};
use backend::mpv::MpvBackend;
use backend::Backend as _;
use clap::{Parser, Subcommand};
use ipc::{Request, Response};
use player::Player;
use resolver::ytdlp::YtDlpResolver;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

#[derive(Parser)]
#[command(name = "surco", about = "Reproductor de musica con motor intercambiable")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Arranca el daemon (posee mpv y la cola).
    Daemon,
    /// Busca y reproduce; el resto de resultados queda en cola.
    Play { query: Vec<String> },
    /// Añade a la cola sin cortar lo que suena.
    Add { query: Vec<String> },
    /// Busca sin reproducir.
    Search { query: Vec<String> },
    /// Alterna play/pausa.
    Toggle,
    Pause,
    Resume,
    Next,
    Prev,
    Stop,
    /// Que suena ahora.
    Status,
    /// Lista la cola.
    Queue,
    /// Vacia la cola.
    Clear,
    /// Volumen 0-100.
    Vol { level: f64 },
    /// Salta a un segundo absoluto.
    Seek { seconds: f64 },
    /// Reproduce el indice N de la cola.
    Jump { index: usize },
    /// Apaga el daemon.
    Kill,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Daemon => run_daemon().await,
        other => run_client(to_request(other)).await,
    }
}

/// Los subcomandos toman la consulta como palabras sueltas para no obligar a
/// entrecomillar: `surco play weird fishes` funciona igual que con comillas.
fn to_request(cmd: Cmd) -> Request {
    match cmd {
        Cmd::Play { query } => Request::Play { query: query.join(" ") },
        Cmd::Add { query } => Request::Add { query: query.join(" ") },
        Cmd::Search { query } => Request::Search {
            query: query.join(" "),
            limit: Some(10),
        },
        Cmd::Toggle => Request::Toggle,
        Cmd::Pause => Request::Pause,
        Cmd::Resume => Request::Resume,
        Cmd::Next => Request::Next,
        Cmd::Prev => Request::Prev,
        Cmd::Stop => Request::Stop,
        Cmd::Status => Request::Status,
        Cmd::Queue => Request::Queue,
        Cmd::Clear => Request::ClearQueue,
        Cmd::Vol { level } => Request::Volume { level },
        Cmd::Seek { seconds } => Request::Seek { seconds },
        Cmd::Jump { index } => Request::Jump { index },
        Cmd::Kill => Request::Quit,
        Cmd::Daemon => unreachable!("el daemon no pasa por aqui"),
    }
}

async fn run_daemon() -> Result<()> {
    let sock = ipc::socket_path();

    // Si ya hay un daemon vivo escuchando ahi, no debemos pisarlo.
    if UnixStream::connect(&sock).await.is_ok() {
        anyhow::bail!("ya hay un daemon corriendo en {}", sock.display());
    }
    let _ = tokio::fs::remove_file(&sock).await;

    let mpv = MpvBackend::spawn(ipc::mpv_socket_path()).await?;
    let events = mpv.subscribe();
    let player = Player::new(mpv.clone(), Arc::new(YtDlpResolver::default()));
    player.watch_end_of_track(events);

    let listener = UnixListener::bind(&sock)
        .with_context(|| format!("no se pudo abrir {}", sock.display()))?;
    eprintln!("surco: escuchando en {}", sock.display());

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(a) => a,
                    Err(e) => { eprintln!("surco: accept fallo: {e}"); continue }
                };
                // Cada cliente en su task: una consulta lenta no bloquea a otro.
                tokio::spawn(handle_client(stream, Arc::clone(&player), shutdown_tx.clone()));
            }
            _ = shutdown_rx.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    eprintln!("surco: cerrando");
    mpv.shutdown().await.ok();
    let _ = tokio::fs::remove_file(&sock).await;
    Ok(())
}

async fn handle_client(
    stream: UnixStream,
    player: Arc<Player>,
    shutdown: tokio::sync::mpsc::Sender<()>,
) {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(req) => {
                let quitting = matches!(req, Request::Quit);
                let res = dispatch(&player, req).await;
                if quitting {
                    let _ = shutdown.send(()).await;
                }
                res
            }
            Err(e) => Response::error(format!("peticion invalida: {e}")),
        };

        let mut out = serde_json::to_string(&response).unwrap_or_else(|_| {
            r#"{"status":"error","message":"fallo al serializar"}"#.to_string()
        });
        out.push('\n');
        if write_half.write_all(out.as_bytes()).await.is_err() {
            break; // el cliente se fue
        }
    }
}

async fn dispatch(player: &Arc<Player>, req: Request) -> Response {
    // Cada brazo traduce el Result del player a Response; el `?` no sirve aqui
    // porque un fallo de red debe volver al cliente, no tumbar el daemon.
    match req {
        Request::Play { query } => match player.play_query(&query, 10).await {
            Ok(t) => Response::ok(format!("suena: {}", t.label())),
            Err(e) => Response::error(e.to_string()),
        },
        Request::Add { query } => match player.enqueue(&query).await {
            Ok(t) => Response::ok(format!("en cola: {}", t.label())),
            Err(e) => Response::error(e.to_string()),
        },
        Request::Search { query, limit } => {
            match player.search(&query, limit.unwrap_or(10)).await {
                Ok(tracks) => Response::data(tracks),
                Err(e) => Response::error(e.to_string()),
            }
        }
        Request::Toggle => match player.toggle_pause().await {
            Ok(paused) => Response::ok(if paused { "pausado" } else { "reproduciendo" }),
            Err(e) => Response::error(e.to_string()),
        },
        Request::Pause => reply(player.set_paused(true).await, "pausado"),
        Request::Resume => reply(player.set_paused(false).await, "reproduciendo"),
        Request::Next => match player.next().await {
            Ok(t) => Response::ok(format!("suena: {}", t.label())),
            Err(e) => Response::error(e.to_string()),
        },
        Request::Prev => match player.prev().await {
            Ok(t) => Response::ok(format!("suena: {}", t.label())),
            Err(e) => Response::error(e.to_string()),
        },
        Request::Jump { index } => match player.play_index(index).await {
            Ok(t) => Response::ok(format!("suena: {}", t.label())),
            Err(e) => Response::error(e.to_string()),
        },
        Request::Stop => reply(player.stop().await, "detenido"),
        Request::Status => match player.status().await {
            Ok(s) => Response::data(s),
            Err(e) => Response::error(e.to_string()),
        },
        Request::Queue => Response::data(player.queue().await),
        Request::ClearQueue => reply(player.clear_queue().await, "cola vacia"),
        Request::Volume { level } => reply(player.set_volume(level).await, format!("volumen {level}")),
        Request::Seek { seconds } => reply(player.seek(seconds).await, format!("posicion {seconds}s")),
        Request::Quit => Response::ok("apagando"),
    }
}

fn reply(res: Result<()>, msg: impl Into<String>) -> Response {
    match res {
        Ok(()) => Response::ok(msg),
        Err(e) => Response::error(e.to_string()),
    }
}

async fn run_client(req: Request) -> Result<()> {
    // Tras una busqueda conviene recordar que los resultados ya son accionables.
    let hint_jump = matches!(req, Request::Search { .. });
    let sock = ipc::socket_path();
    let stream = UnixStream::connect(&sock).await.with_context(|| {
        format!(
            "no hay daemon en {}. Arrancalo con: surco daemon",
            sock.display()
        )
    })?;

    let (read_half, mut write_half) = stream.into_split();
    let mut payload = serde_json::to_string(&req)?;
    payload.push('\n');
    write_half.write_all(payload.as_bytes()).await?;

    let mut lines = BufReader::new(read_half).lines();
    let line = lines
        .next_line()
        .await?
        .context("el daemon cerro sin responder")?;

    match serde_json::from_str::<Response>(&line)? {
        Response::Ok { message } => println!("{message}"),
        Response::Data { payload } => {
            println!("{}", render(&payload));
            if hint_jump && payload.as_array().is_some_and(|a| !a.is_empty()) {
                println!("\n     elige con: surco jump N");
            }
        }
        Response::Error { message } => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// Formatea las respuestas de datos para leerlas en la terminal. El JSON crudo
/// sigue disponible para cualquier interfaz que hable el socket directamente.
fn render(payload: &serde_json::Value) -> String {
    use serde_json::Value;

    // Una lista de pistas (search / queue).
    if let Some(items) = payload.as_array() {
        if items.is_empty() {
            return "(vacio)".to_string();
        }
        return items
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let title = t.get("title").and_then(Value::as_str).unwrap_or("?");
                let who = t
                    .get("artist")
                    .and_then(Value::as_str)
                    .or_else(|| t.get("channel").and_then(Value::as_str))
                    .unwrap_or("?");
                let dur = t
                    .get("duration")
                    .and_then(Value::as_f64)
                    .map(fmt_time)
                    .unwrap_or_else(|| "--:--".into());
                format!("{i:>3}  {dur:>6}  {}", resolver::one_line(Some(who), title))
            })
            .collect::<Vec<_>>()
            .join("\n");
    }

    // El status.
    if let Some(pb) = payload.get("playback") {
        let pos = pb.get("position").and_then(Value::as_f64).unwrap_or(0.0);
        let dur = pb.get("duration").and_then(Value::as_f64);
        let paused = pb.get("paused").and_then(Value::as_bool).unwrap_or(false);
        let idle = pb.get("idle").and_then(Value::as_bool).unwrap_or(true);

        let what = payload
            .get("current")
            .filter(|c| !c.is_null())
            .map(|c| {
                let title = c.get("title").and_then(Value::as_str).unwrap_or("?");
                let who = c
                    .get("artist")
                    .and_then(Value::as_str)
                    .or_else(|| c.get("channel").and_then(Value::as_str))
                    .unwrap_or("?");
                resolver::one_line(Some(who), title)
            })
            .unwrap_or_else(|| "nada cargado".to_string());

        let icon = if idle && !paused {
            "" // ni sonando ni pausado: cola terminada
        } else if paused {
            ""
        } else {
            ""
        };

        let time = match dur {
            Some(d) => format!("{} / {}", fmt_time(pos), fmt_time(d)),
            None => fmt_time(pos),
        };
        let vol = payload.get("volume").and_then(Value::as_f64).unwrap_or(0.0);
        let n = payload.get("queue_len").and_then(Value::as_u64).unwrap_or(0);
        let at = payload
            .get("position_in_queue")
            .and_then(Value::as_u64)
            .map(|i| format!("{}/{}", i + 1, n))
            .unwrap_or_else(|| format!("-/{n}"));

        return format!("{icon}  {what}\n   {time}   vol {vol:.0}   cola {at}");
    }

    payload.to_string()
}

fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}
