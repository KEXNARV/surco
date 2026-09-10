//! Vista de letras en la terminal: sigue la cancion y resalta el verso actual.
//!
//! Mantiene una sola conexion al daemon y le pregunta la posicion varias veces
//! por segundo. Sondear el socket sale casi gratis (el estado ya esta en
//! memoria del daemon); lo caro seria arrancar un proceso por consulta.

use crate::ipc::{self, Request, Response};
use crate::lyrics::Lyrics;
use anyhow::{Context, Result};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use std::io::{stdout, Write};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Conexion persistente que habla el protocolo linea a linea.
struct Conn {
    write: tokio::net::unix::OwnedWriteHalf,
    read: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
}

impl Conn {
    async fn open() -> Result<Self> {
        let path = ipc::socket_path();
        let stream = UnixStream::connect(&path)
            .await
            .with_context(|| format!("no hay daemon en {}. Arrancalo: surco daemon", path.display()))?;
        let (r, w) = stream.into_split();
        Ok(Self { write: w, read: BufReader::new(r).lines() })
    }

    async fn ask(&mut self, req: &Request) -> Result<Response> {
        let mut line = serde_json::to_string(req)?;
        line.push('\n');
        self.write.write_all(line.as_bytes()).await?;
        let raw = self
            .read
            .next_line()
            .await?
            .context("el daemon cerro la conexion")?;
        Ok(serde_json::from_str(&raw)?)
    }
}

/// Lo que hace falta pintar un frame.
struct Frame {
    title: String,
    position: f64,
    paused: bool,
}

async fn poll_frame(conn: &mut Conn) -> Result<Option<Frame>> {
    let Response::Data { payload } = conn.ask(&Request::Status).await? else {
        return Ok(None);
    };
    let pb = payload.get("playback");
    let position = pb
        .and_then(|p| p.get("position"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let paused = pb
        .and_then(|p| p.get("paused"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let title = payload
        .get("current")
        .filter(|c| !c.is_null())
        .map(|c| {
            let t = c.get("title").and_then(serde_json::Value::as_str).unwrap_or("?");
            let who = c
                .get("artist")
                .and_then(serde_json::Value::as_str)
                .or_else(|| c.get("channel").and_then(serde_json::Value::as_str));
            crate::resolver::one_line(who, t)
        })
        .unwrap_or_else(|| "nada cargado".into());
    Ok(Some(Frame { title, position, paused }))
}

async fn load_lyrics(conn: &mut Conn) -> Result<Option<Lyrics>> {
    match conn.ask(&Request::Lyrics).await? {
        Response::Data { payload } if !payload.is_null() => Ok(serde_json::from_value(payload)?),
        Response::Error { message } => anyhow::bail!("{message}"),
        _ => Ok(None),
    }
}

pub async fn run_lyrics_view() -> Result<()> {
    let mut conn = Conn::open().await?;

    // Antes de tomar la terminal: si no hay letra, mejor decirlo en texto plano.
    let frame = poll_frame(&mut conn).await?.context("el daemon no respondio")?;
    let Some(mut lyrics) = load_lyrics(&mut conn).await? else {
        println!("sin letra para: {}", frame.title);
        println!("(lrclib no la tiene, o la pista es instrumental)");
        return Ok(());
    };

    let mut out = stdout();
    terminal::enable_raw_mode()?;
    execute!(out, terminal::EnterAlternateScreen, cursor::Hide)?;

    // Cualquier salida, incluso por error, tiene que devolver la terminal.
    let result = event_loop(&mut conn, &mut lyrics, &frame.title).await;

    execute!(stdout(), cursor::Show, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    result
}

async fn event_loop(conn: &mut Conn, lyrics: &mut Lyrics, initial_title: &str) -> Result<()> {
    let mut title = initial_title.to_string();
    let mut position: f64 = 0.0;
    let mut paused = false;
    let mut notice = String::new();

    loop {
        // --- entrada del usuario, sin bloquear el refresco ---
        while event::poll(Duration::from_millis(0))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(())
                    }
                    KeyCode::Char(' ') => {
                        conn.ask(&Request::Toggle).await?;
                    }
                    KeyCode::Left => {
                        conn.ask(&Request::Seek { seconds: (position - 5.0).max(0.0) }).await?;
                    }
                    KeyCode::Right => {
                        conn.ask(&Request::Seek { seconds: position + 5.0 }).await?;
                    }
                    // La letra va adelantada: '+' la retrasa.
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        lyrics.offset += 0.5;
                        conn.ask(&Request::LyricsOffset { delta: 0.5 }).await?;
                        notice = format!("desfase {:+.1}s", lyrics.offset);
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') => {
                        lyrics.offset -= 0.5;
                        conn.ask(&Request::LyricsOffset { delta: -0.5 }).await?;
                        notice = format!("desfase {:+.1}s", lyrics.offset);
                    }
                    KeyCode::Char('n') => {
                        conn.ask(&Request::Next).await?;
                    }
                    KeyCode::Char('p') => {
                        conn.ask(&Request::Prev).await?;
                    }
                    _ => {}
                }
            }
        }

        // --- estado del reproductor ---
        if let Some(f) = poll_frame(conn).await? {
            // Cambio de cancion: hay que traer otra letra.
            if f.title != title {
                title = f.title.clone();
                notice.clear();
                match load_lyrics(conn).await {
                    Ok(Some(next)) => *lyrics = next,
                    _ => {
                        *lyrics = Lyrics {
                            matched: "sin letra para esta pista".into(),
                            ..Default::default()
                        }
                    }
                }
            }
            position = f.position;
            paused = f.paused;
        }

        draw(lyrics, &title, position, paused, &notice)?;
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
}

fn draw(lyrics: &Lyrics, title: &str, position: f64, paused: bool, notice: &str) -> Result<()> {
    let (cols, rows) = terminal::size()?;
    let cols = cols as usize;
    let mut out = stdout();
    queue!(out, Clear(ClearType::All), cursor::MoveTo(0, 0))?;

    // --- cabecera ---
    let icon = if paused { "" } else { "" };
    queue!(
        out,
        SetAttribute(Attribute::Bold),
        Print(fit(&format!("{icon}  {title}"), cols)),
        SetAttribute(Attribute::Reset),
        cursor::MoveTo(0, 1),
        SetForegroundColor(Color::DarkGrey),
        Print(fit(
            &format!(
                "   {}   {}   {}",
                fmt_time(position),
                lyrics.matched,
                if notice.is_empty() { lyrics.source.clone() } else { notice.to_string() }
            ),
            cols
        )),
        ResetColor
    )?;

    let body_top = 3u16;
    let body_rows = rows.saturating_sub(body_top + 2) as usize;

    if !lyrics.is_synced() {
        // Sin tiempos solo se puede volcar el texto.
        let text = lyrics.plain.clone().unwrap_or_else(|| "sin letra".into());
        for (i, line) in text.lines().take(body_rows).enumerate() {
            queue!(out, cursor::MoveTo(2, body_top + i as u16), Print(fit(line, cols - 2)))?;
        }
        queue!(out, cursor::MoveTo(0, rows - 1), SetForegroundColor(Color::DarkGrey),
               Print(fit("  q salir   espacio pausa   (esta letra no tiene tiempos)", cols)), ResetColor)?;
        out.flush()?;
        return Ok(());
    }

    // --- versos, con el actual en el centro ---
    let cur = lyrics.index_at(position);
    let center = body_rows / 2;
    // Indice del verso que va en la primera fila visible. Se permite negativo
    // para que la intro no arranque pegada al borde.
    let first = cur.map_or(0i64, |c| c as i64) - center as i64;

    for row in 0..body_rows {
        let idx = first + row as i64;
        if idx < 0 || idx as usize >= lyrics.lines.len() {
            continue;
        }
        let idx = idx as usize;
        let line = &lyrics.lines[idx];
        if line.text.trim().is_empty() {
            continue;
        }
        let is_current = Some(idx) == cur;
        // Cuanto mas lejos del verso actual, mas apagado.
        let dist = cur.map_or(99, |c| c.abs_diff(idx));
        let color = if is_current {
            Color::White
        } else if dist <= 2 {
            Color::Grey
        } else {
            Color::DarkGrey
        };
        // El verso actual se centra en pantalla; los demas van alineados.
        let indent = if is_current { 2 } else { 4 };
        queue!(out, cursor::MoveTo(indent, body_top + row as u16))?;
        if is_current {
            queue!(out, SetAttribute(Attribute::Bold), SetForegroundColor(color),
                   Print(fit(&format!("▸ {}", line.text), cols - indent as usize)),
                   SetAttribute(Attribute::Reset))?;
        } else {
            queue!(out, SetForegroundColor(color),
                   Print(fit(&line.text, cols - indent as usize)))?;
        }
        queue!(out, ResetColor)?;
    }

    queue!(
        out,
        cursor::MoveTo(0, rows - 1),
        SetForegroundColor(Color::DarkGrey),
        Print(fit(
            "  q salir   espacio pausa   ←/→ 5s   +/- cuadrar letra   n/p pista",
            cols
        )),
        ResetColor
    )?;
    out.flush()?;
    Ok(())
}

/// Recorta a lo ancho contando caracteres, no bytes: los acentos ocupan dos.
fn fit(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    s.chars().take(width.saturating_sub(1)).collect::<String>() + "…"
}

fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}
