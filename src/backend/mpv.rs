//! Motor sobre mpv, hablando su protocolo JSON-IPC por un socket unix.
//!
//! mpv ya resuelve lo dificil: decodifica opus, hace buffering de HTTP, seek
//! sobre streams remotos y saca audio por pipewire. Reimplementar eso con
//! symphonia+rodio serian miles de lineas para llegar al mismo sitio.
//!
//! El socket es una sola conexion full-duplex donde se cruzan respuestas y
//! eventos asincronos, asi que un task dedicado lo posee y despacha: las
//! respuestas se casan por `request_id`, los eventos se difunden aparte.

use super::{Backend, Playback};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

/// Una peticion en vuelo, esperando la respuesta de mpv.
struct Pending {
    id: u64,
    payload: Value,
    reply: oneshot::Sender<Result<Value>>,
}

pub struct MpvBackend {
    tx: mpsc::Sender<Pending>,
    next_id: AtomicU64,
    /// Eventos de mpv completos, con su payload. Se difunde el JSON entero
    /// porque el nombre solo no basta: `end-file` necesita su `reason` para
    /// saber si la pista acabo sola o si fuimos nosotros al cargar otra.
    events: broadcast::Sender<Value>,
    child: Mutex<Option<tokio::process::Child>>,
    socket_path: PathBuf,
}

impl MpvBackend {
    /// Arranca un mpv propio en modo idle y se conecta a su socket.
    pub async fn spawn(socket_path: PathBuf) -> Result<Arc<Self>> {
        // Un socket huerfano de una corrida anterior haria que connect() apunte
        // a un mpv que ya no existe.
        let _ = tokio::fs::remove_file(&socket_path).await;
        if let Some(dir) = socket_path.parent() {
            tokio::fs::create_dir_all(dir).await.ok();
        }

        let child = tokio::process::Command::new("mpv")
            .arg("--idle=yes")
            .arg("--no-video")
            .arg("--no-terminal")
            // Sin cache el seek sobre HTTP se vuelve inutilizable.
            .arg("--cache=yes")
            .arg("--cache-secs=60")
            // Evita que mpv relea ~/.config/mpv y nos cambie el comportamiento.
            .arg("--no-config")
            .arg(format!("--input-ipc-server={}", socket_path.display()))
            .kill_on_drop(true)
            .spawn()
            .context("no se pudo arrancar mpv; esta instalado?")?;

        let stream = Self::connect_with_retry(&socket_path).await?;
        let (tx, rx) = mpsc::channel(64);
        let (events, _) = broadcast::channel(256);

        let backend = Arc::new(Self {
            tx,
            next_id: AtomicU64::new(1),
            events: events.clone(),
            child: Mutex::new(Some(child)),
            socket_path,
        });

        tokio::spawn(Self::run(stream, rx, events));
        Ok(backend)
    }

    /// mpv crea el socket un instante despues de arrancar, no al instante.
    async fn connect_with_retry(path: &PathBuf) -> Result<UnixStream> {
        for _ in 0..100 {
            if let Ok(s) = UnixStream::connect(path).await {
                return Ok(s);
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        bail!("mpv nunca abrio el socket en {}", path.display())
    }

    /// El unico dueño del socket: escribe peticiones, lee todo lo que llega y
    /// lo reparte entre respuestas pendientes y eventos.
    async fn run(
        stream: UnixStream,
        mut rx: mpsc::Receiver<Pending>,
        events: broadcast::Sender<Value>,
    ) {
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = BufReader::new(read_half).lines();
        let mut pending: HashMap<u64, oneshot::Sender<Result<Value>>> = HashMap::new();

        loop {
            tokio::select! {
                // Nueva peticion desde cualquier parte del programa.
                Some(req) = rx.recv() => {
                    let mut line = req.payload.to_string();
                    line.push('\n');
                    if let Err(e) = write_half.write_all(line.as_bytes()).await {
                        let _ = req.reply.send(Err(anyhow!("mpv no acepta comandos: {e}")));
                        break;
                    }
                    pending.insert(req.id, req.reply);
                }

                // Algo llego por el socket.
                line = lines.next_line() => {
                    let line = match line {
                        Ok(Some(l)) => l,
                        // EOF o error de lectura: mpv murio.
                        _ => break,
                    };
                    let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };

                    if let Some(id) = msg.get("request_id").and_then(Value::as_u64) {
                        if let Some(reply) = pending.remove(&id) {
                            let err = msg.get("error").and_then(Value::as_str).unwrap_or("");
                            let out = if err == "success" {
                                Ok(msg.get("data").cloned().unwrap_or(Value::Null))
                            } else {
                                Err(anyhow!("mpv rechazo el comando: {err}"))
                            };
                            let _ = reply.send(out);
                        }
                    } else if msg.get("event").is_some() {
                        // Nadie escuchando todavia no es un error.
                        let _ = events.send(msg);
                    }
                }

                else => break,
            }
        }

        // mpv se fue: nadie debe quedarse esperando para siempre.
        for (_, reply) in pending.drain() {
            let _ = reply.send(Err(anyhow!("mpv termino")));
        }
        let _ = events.send(json!({ "event": "shutdown" }));
    }

    /// Envia un comando y espera su respuesta.
    async fn command(&self, args: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, wait) = oneshot::channel();
        self.tx
            .send(Pending {
                id,
                payload: json!({ "command": args, "request_id": id }),
                reply,
            })
            .await
            .map_err(|_| anyhow!("el motor ya no esta corriendo"))?;

        // Un comando que no vuelve no debe congelar al reproductor entero.
        match tokio::time::timeout(std::time::Duration::from_secs(15), wait).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => bail!("el motor descarto la peticion"),
            Err(_) => bail!("mpv no respondio en 15s"),
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, prop: &str) -> Result<T> {
        let v = self.command(json!(["get_property", prop])).await?;
        serde_json::from_value(v).with_context(|| format!("propiedad {prop} con tipo inesperado"))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }
}

#[async_trait::async_trait]
impl Backend for MpvBackend {
    async fn load(&self, url: &str) -> Result<()> {
        self.command(json!(["loadfile", url, "replace"])).await?;
        // loadfile responde en cuanto acepta la orden, no cuando ya suena.
        self.command(json!(["set_property", "pause", false])).await?;
        Ok(())
    }

    async fn set_paused(&self, paused: bool) -> Result<()> {
        self.command(json!(["set_property", "pause", paused])).await?;
        Ok(())
    }

    async fn seek(&self, seconds: f64) -> Result<()> {
        self.command(json!(["seek", seconds, "absolute"])).await?;
        Ok(())
    }

    async fn set_volume(&self, volume: f64) -> Result<()> {
        self.command(json!(["set_property", "volume", volume.clamp(0.0, 100.0)]))
            .await?;
        Ok(())
    }

    async fn playback(&self) -> Result<Playback> {
        // Sin pista cargada mpv devuelve error en time-pos/duration en vez de
        // null, asi que un fallo aqui se lee como "no hay nada sonando".
        Ok(Playback {
            position: self.get("time-pos").await.unwrap_or(0.0),
            duration: self.get("duration").await.ok(),
            paused: self.get("pause").await.unwrap_or(false),
            idle: self.get("core-idle").await.unwrap_or(true),
        })
    }

    async fn stop(&self) -> Result<()> {
        self.command(json!(["stop"])).await?;
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        // Si mpv ya murio el quit falla, y da igual: igual hay que limpiar.
        let _ = self.command(json!(["quit"])).await;
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await;
            let _ = child.kill().await;
        }
        let _ = tokio::fs::remove_file(&self.socket_path).await;
        Ok(())
    }
}
