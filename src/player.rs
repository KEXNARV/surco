//! Cola, estado y orquestacion. Es lo que convierte "un motor" en "un
//! reproductor": decide que suena, que sigue y cuando adelantarse a resolverlo.

use crate::backend::{Backend, Playback};
use crate::resolver::{Resolver, Track};
use anyhow::{bail, Result};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Una URL de stream ya resuelta, lista para cargar sin esperar a yt-dlp.
#[derive(Clone)]
struct Resolved {
    track_id: String,
    url: String,
}

#[derive(Default)]
struct State {
    queue: Vec<Track>,
    /// Indice en `queue` de lo que suena, cuando lo que suena viene de ella.
    /// Una busqueda reemplaza la cola sin cortar el audio, y entonces esto
    /// queda en `None` aunque siga habiendo musica.
    current: Option<usize>,
    /// Lo que de verdad esta cargado en el motor. Sobrevive a que la cola
    /// cambie debajo, para que el status nunca mienta.
    now_playing: Option<Track>,
    /// La siguiente pista ya resuelta, para que el salto sea instantaneo.
    next_up: Option<Resolved>,
    volume: f64,
}

#[derive(serde::Serialize)]
pub struct Status {
    pub playback: Playback,
    pub current: Option<Track>,
    pub position_in_queue: Option<usize>,
    pub queue_len: usize,
    pub volume: f64,
}

pub struct Player {
    backend: Arc<dyn Backend>,
    resolver: Arc<dyn Resolver>,
    state: Mutex<State>,
}

impl Player {
    pub fn new(backend: Arc<dyn Backend>, resolver: Arc<dyn Resolver>) -> Arc<Self> {
        Arc::new(Self {
            backend,
            resolver,
            state: Mutex::new(State {
                volume: 70.0,
                ..Default::default()
            }),
        })
    }

    /// Busca y reproduce el primer resultado, dejando el resto en la cola.
    pub async fn play_query(self: &Arc<Self>, query: &str, limit: usize) -> Result<Track> {
        let found = self.resolver.search(query, limit).await?;
        if found.is_empty() {
            bail!("sin resultados para \"{query}\"");
        }
        {
            let mut st = self.state.lock().await;
            st.queue = found;
            st.current = None;
            st.next_up = None;
        }
        self.play_index(0).await
    }

    pub async fn enqueue(self: &Arc<Self>, query: &str) -> Result<Track> {
        let mut found = self.resolver.search(query, 1).await?;
        let track = match found.drain(..).next() {
            Some(t) => t,
            None => bail!("sin resultados para \"{query}\""),
        };
        let next_idx = {
            let mut st = self.state.lock().await;
            // Añadir al final no reordena nada, asi que un prefetch en vuelo
            // sigue siendo valido: solo importa si esta pista pasa a ser la
            // siguiente, que es cuando conviene adelantarse a resolverla.
            let becomes_next = st.current.map_or(false, |c| st.queue.len() == c + 1);
            st.queue.push(track.clone());
            becomes_next.then(|| st.queue.len() - 1)
        };
        if let Some(idx) = next_idx {
            self.spawn_prefetch(idx);
        }
        Ok(track)
    }

    /// Carga la pista `idx` de la cola y dispara el prefetch de la siguiente.
    pub async fn play_index(self: &Arc<Self>, idx: usize) -> Result<Track> {
        // El lock se suelta antes de la resolucion: yt-dlp tarda segundos y
        // mantenerlo bloquearia cualquier consulta de estado mientras tanto.
        let (track, prefetched) = {
            let st = self.state.lock().await;
            let track = match st.queue.get(idx) {
                Some(t) => t.clone(),
                None => bail!("la cola no tiene indice {idx}"),
            };
            let hit = st
                .next_up
                .as_ref()
                .filter(|r| r.track_id == track.id)
                .cloned();
            (track, hit)
        };

        let url = match prefetched {
            Some(r) => r.url,
            None => self.resolver.stream_url(&track).await?,
        };

        self.backend.load(&url).await?;

        let volume = {
            let mut st = self.state.lock().await;
            st.current = Some(idx);
            st.now_playing = Some(track.clone());
            st.next_up = None;
            st.volume
        };
        // mpv olvida el volumen entre archivos con --no-config.
        self.backend.set_volume(volume).await?;

        self.spawn_prefetch(idx + 1);
        Ok(track)
    }

    /// Resuelve en segundo plano la URL de `idx` para que el cambio de pista no
    /// se coma los ~2.7s que tarda yt-dlp.
    fn spawn_prefetch(self: &Arc<Self>, idx: usize) {
        self.prefetch(idx, true)
    }

    /// Igual, pero sin exigir que `idx` sea el siguiente de lo que suena. Lo
    /// usa la busqueda, donde no hay pista actual dentro de la cola nueva.
    fn spawn_prefetch_at(self: &Arc<Self>, idx: usize) {
        self.prefetch(idx, false)
    }

    fn prefetch(self: &Arc<Self>, idx: usize, require_adjacent: bool) {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let track = {
                let st = me.state.lock().await;
                match st.queue.get(idx) {
                    Some(t) => t.clone(),
                    None => return, // fin de la cola, nada que adelantar
                }
            };
            if let Ok(url) = me.resolver.stream_url(&track).await {
                let mut st = me.state.lock().await;
                // La cola puede haber cambiado mientras resolviamos: solo vale
                // si ese indice sigue teniendo la misma pista.
                if st.queue.get(idx).map(|t| &t.id) != Some(&track.id) {
                    return;
                }
                if require_adjacent && st.current.map(|c| c + 1) != Some(idx) {
                    return;
                }
                st.next_up = Some(Resolved {
                    track_id: track.id,
                    url,
                });
            }
        });
    }

    pub async fn next(self: &Arc<Self>) -> Result<Track> {
        let idx = match self.state.lock().await.current {
            Some(c) => c + 1,
            None => 0,
        };
        self.play_index(idx).await
    }

    pub async fn prev(self: &Arc<Self>) -> Result<Track> {
        let idx = match self.state.lock().await.current {
            Some(c) if c > 0 => c - 1,
            // Ya en la primera: reinicia en vez de fallar.
            _ => 0,
        };
        self.play_index(idx).await
    }

    pub async fn toggle_pause(&self) -> Result<bool> {
        let now = self.backend.playback().await?.paused;
        self.backend.set_paused(!now).await?;
        Ok(!now)
    }

    pub async fn set_paused(&self, paused: bool) -> Result<()> {
        self.backend.set_paused(paused).await
    }

    pub async fn seek(&self, seconds: f64) -> Result<()> {
        self.backend.seek(seconds.max(0.0)).await
    }

    pub async fn set_volume(&self, volume: f64) -> Result<()> {
        let v = volume.clamp(0.0, 100.0);
        self.state.lock().await.volume = v;
        self.backend.set_volume(v).await
    }

    pub async fn stop(&self) -> Result<()> {
        let mut st = self.state.lock().await;
        st.current = None;
        st.now_playing = None;
        st.next_up = None;
        drop(st);
        self.backend.stop().await
    }

    pub async fn status(&self) -> Result<Status> {
        let playback = self.backend.playback().await?;
        let st = self.state.lock().await;
        Ok(Status {
            playback,
            current: st.now_playing.clone(),
            position_in_queue: st.current,
            queue_len: st.queue.len(),
            volume: st.volume,
        })
    }

    /// Busca y deja los resultados en la cola, listos para `jump`, sin cortar
    /// lo que suena. Antes esto devolvia una lista que no se podia accionar.
    pub async fn search(self: &Arc<Self>, query: &str, limit: usize) -> Result<Vec<Track>> {
        let found = self.resolver.search(query, limit).await?;
        if found.is_empty() {
            return Ok(found);
        }
        {
            let mut st = self.state.lock().await;
            st.queue = found.clone();
            // Lo que suena ya no pertenece a esta cola, pero sigue sonando.
            st.current = None;
            st.next_up = None;
        }
        // Adelanta el primero: es el que se elige la mayoria de las veces.
        self.spawn_prefetch_at(0);
        Ok(found)
    }

    pub async fn queue(&self) -> Vec<Track> {
        self.state.lock().await.queue.clone()
    }

    pub async fn clear_queue(&self) -> Result<()> {
        let mut st = self.state.lock().await;
        st.queue.clear();
        st.current = None;
        st.now_playing = None;
        st.next_up = None;
        drop(st);
        self.backend.stop().await
    }

    /// Avanza sola cuando una pista termina. Se lanza una vez al arrancar.
    pub fn watch_end_of_track(
        self: &Arc<Self>,
        mut events: tokio::sync::broadcast::Receiver<serde_json::Value>,
    ) {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let msg = match events.recv().await {
                    Ok(m) => m,
                    // Lagged: se perdieron eventos por lentitud, no es fatal.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                };
                if msg.get("event").and_then(|v| v.as_str()) != Some("end-file") {
                    continue;
                }
                if std::env::var_os("SURCO_DEBUG").is_some() {
                    eprintln!("surco[dbg] {msg}");
                }
                // Aqui esta la diferencia que importa: mpv manda `end-file`
                // tanto cuando la pista acaba sola (reason "eof") como cuando
                // nosotros cargamos otra encima ("stop") o al salir ("quit").
                // Sin este filtro cada `next` avanzaba dos pistas.
                if msg.get("reason").and_then(|v| v.as_str()) != Some("eof") {
                    continue;
                }
                let has_next = {
                    let st = me.state.lock().await;
                    st.current.map_or(false, |c| c + 1 < st.queue.len())
                };
                if has_next {
                    // Un fallo aqui (video caido, red) no debe matar el watcher.
                    if let Err(e) = me.next().await {
                        eprintln!("surco: no se pudo avanzar de pista: {e}");
                    }
                }
            }
        });
    }
}
