//! Cola, estado y orquestacion. Es lo que convierte "un motor" en "un
//! reproductor": decide que suena, que sigue y cuando adelantarse a resolverlo.

use crate::backend::{Backend, Playback};
use crate::library::{self, Library};
use crate::lyrics::{Lyrics, LyricsProvider};
use crate::resolver::{Resolver, Track};
use anyhow::{bail, Result};
use std::sync::Arc;
use tokio::sync::Mutex;

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
    /// La última pista que mpv no pudo abrir y cuándo se reintentó: una vez, no en bucle.
    retried: Option<(String, std::time::Instant)>,
    volume: f64,
    /// Cuándo empezó `now_playing`, para el historial.
    started_at: u64,
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
    lyrics: Arc<dyn LyricsProvider>,
    pub library: Arc<Library>,
    pub catalog: crate::resolver::ytmusic::YtMusic,
    state: Mutex<State>,
    urls: crate::urls::Urls,
    video_urls: crate::urls::Urls,
    /// La última lista "para ti" y cuándo se armó: se rehace pasado un rato o si se pide.
    for_you: Mutex<Option<(std::time::Instant, ForYou)>>,
}

#[derive(Clone, serde::Serialize)]
pub struct ForYou {
    /// De qué canciones salió, para decirlo en la página.
    pub seeds: Vec<Track>,
    pub tracks: Vec<Track>,
}

/// Cuánto vale una lista "para ti" antes de rehacerla sola.
const FOR_YOU_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

impl Player {
    pub fn new(
        backend: Arc<dyn Backend>,
        resolver: Arc<dyn Resolver>,
        lyrics: Arc<dyn LyricsProvider>,
        library: Arc<Library>,
    ) -> Arc<Self> {
        Arc::new(Self {
            backend,
            resolver,
            lyrics,
            library,
            catalog: crate::resolver::ytmusic::YtMusic::new().expect("cliente HTTP"),
            urls: Default::default(),
            video_urls: crate::urls::Urls::video(),
            for_you: Mutex::new(None),
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
        }
        self.play_index(0).await
    }

    /// Reemplaza la cola por `tracks` y reproduce desde `index`: lo que hace un clic en
    /// una fila de una búsqueda, una playlist o los favoritos.
    pub async fn play_tracks(self: &Arc<Self>, tracks: Vec<Track>, index: usize) -> Result<Track> {
        if index >= tracks.len() {
            bail!("la lista no tiene indice {index}");
        }
        {
            let mut st = self.state.lock().await;
            st.queue = tracks;
            st.current = None;
        }
        self.play_index(index).await
    }

    /// Añade una pista ya conocida al final de la cola, sin buscar.
    pub async fn enqueue_track(self: &Arc<Self>, track: Track) -> Result<()> {
        let next_idx = {
            let mut st = self.state.lock().await;
            let becomes_next = st.current.map_or(false, |c| st.queue.len() == c + 1);
            st.queue.push(track);
            becomes_next.then(|| st.queue.len() - 1)
        };
        if let Some(idx) = next_idx {
            self.spawn_prefetch(idx);
        }
        Ok(())
    }

    /// Deja en el historial lo que sonaba, con cuánto se escuchó. `end`: "eof", "skip"
    /// o "stop". Lo que falle aquí no debe impedir cambiar de pista.
    async fn finish(&self, end: &str) {
        let Some((track, started)) = ({
            let mut st = self.state.lock().await;
            st.now_playing.take().map(|t| (t, st.started_at))
        }) else {
            return;
        };
        let pb = self.backend.playback().await.ok();
        let duration = pb.as_ref().and_then(|p| p.duration).or(track.duration);
        // Al terminar sola mpv ya reinició la posición: se escuchó entera.
        let listened = if end == "eof" { duration.unwrap_or(0.0) } else { pb.map(|p| p.position).unwrap_or(0.0) };
        if let Err(e) = self.library.record(track, started, listened, duration, end).await {
            eprintln!("surco: no se pudo guardar el historial: {e}");
        }
    }

    pub async fn enqueue(self: &Arc<Self>, query: &str) -> Result<Track> {
        let mut found = self.resolver.search(query, 1).await?;
        let track = match found.drain(..).next() {
            Some(t) => t,
            None => bail!("sin resultados para \"{query}\""),
        };
        let next_idx = {
            let mut st = self.state.lock().await;
            // Solo importa si esta pista pasa a ser la siguiente, que es cuando
            // conviene adelantarse a resolverla.
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
        let track = match self.state.lock().await.queue.get(idx) {
            Some(t) => t.clone(),
            None => bail!("la cola no tiene indice {idx}"),
        };
        let url = self.urls.get(self.resolver.as_ref(), &track).await?;

        self.finish("skip").await;
        self.backend.load(&url).await?;

        let volume = {
            let mut st = self.state.lock().await;
            st.current = Some(idx);
            st.now_playing = Some(track.clone());
            st.started_at = library::timestamp();
            st.volume
        };
        // mpv olvida el volumen entre archivos con --no-config.
        self.backend.set_volume(volume).await?;

        self.spawn_prefetch(idx + 1);
        Ok(track)
    }

    /// Resuelve en segundo plano la URL de `idx` para que el cambio de pista no
    /// se coma lo que tarda yt-dlp. Si la cola cambia mientras tanto no pasa nada:
    /// la URL queda guardada por pista, no por posición.
    fn spawn_prefetch(self: &Arc<Self>, idx: usize) {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let Some(track) = me.state.lock().await.queue.get(idx).cloned() else {
                return; // fin de la cola, nada que adelantar
            };
            let _ = me.urls.get(me.resolver.as_ref(), &track).await;
        });
    }

    /// Adelanta la URL de una pista que quizá suene pronto (la app la pide al dejar el
    /// mouse encima), sin tocar la cola. Vuelve enseguida; la resolución sigue sola.
    pub fn warm(self: &Arc<Self>, track: Track) {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let _ = me.urls.get(me.resolver.as_ref(), &track).await;
        });
    }

    /// La siguiente de la cola, saltando las que no te gustan.
    pub async fn next(self: &Arc<Self>) -> Result<Track> {
        let (start, queue) = {
            let st = self.state.lock().await;
            (st.current.map_or(0, |c| c + 1), st.queue.clone())
        };
        let disliked = self.library.data().await.disliked;
        let idx = (start..queue.len()).find(|&i| !disliked.iter().any(|d| d.id == queue[i].id));
        match idx {
            Some(i) => self.play_index(i).await,
            None => bail!("no queda nada en la cola"),
        }
    }

    pub async fn dislike(self: &Arc<Self>, track: Track, on: bool) -> Result<()> {
        let id = track.id.clone();
        self.library.set_disliked(track, on).await?;
        let playing = self.state.lock().await.now_playing.as_ref().is_some_and(|t| t.id == id);
        if on && playing {
            // Sin siguiente, que al menos deje de sonar.
            if self.next().await.is_err() {
                self.stop().await?;
            }
        }
        // "Para ti" se rehace sin ella la próxima vez que se pida.
        *self.for_you.lock().await = None;
        Ok(())
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
        self.finish("stop").await;
        let mut st = self.state.lock().await;
        st.current = None;
        st.now_playing = None;
        drop(st);
        self.backend.stop().await
    }

    /// URL de solo video de una pista, comprobada y guardada como las de audio.
    pub async fn video_url(&self, track: &Track) -> Result<String> {
        self.video_urls.get(self.resolver.as_ref(), track).await
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

    /// "Para ti": radios de YouTube Music de unas semillas, ordenadas por el historial
    /// (para_ti.rs). `refresh` arma otra aunque la guardada siga vigente.
    pub async fn for_you(self: &Arc<Self>, refresh: bool) -> Result<ForYou> {
        let mut cached = self.for_you.lock().await;
        if let Some((at, fy)) = cached.as_ref() {
            if !refresh && at.elapsed() < FOR_YOU_TTL {
                return Ok(fy.clone());
            }
        }
        let history = self.library.history().await?;
        let data = self.library.data().await;
        let taste = crate::para_ti::Taste::new(&history, &data.favorites, &data.artists, &data.disliked);
        let mut seed = library::timestamp() ^ 0x9e37_79b9_7f4a_7c15;
        let seeds = taste.seeds(&history, &data.favorites, || {
            // xorshift: no hace falta más azar que este para elegir semillas.
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        });
        let mut jobs = tokio::task::JoinSet::new();
        for (i, s) in seeds.iter().enumerate() {
            let me = Arc::clone(self);
            let id = s.id.clone();
            jobs.spawn(async move { (i, me.catalog.radio(&id).await) });
        }
        let mut radios = vec![vec![]; seeds.len()];
        let mut failed = None;
        while let Some(Ok((i, r))) = jobs.join_next().await {
            match r {
                Ok(list) => radios[i] = list,
                Err(e) => failed = Some(e),
            }
        }
        if radios.iter().all(Vec::is_empty) {
            if let Some(e) = failed {
                return Err(e);
            }
        }
        let fy = ForYou { tracks: taste.rank(&seeds, &radios, library::timestamp()), seeds };
        *cached = Some((std::time::Instant::now(), fy.clone()));
        Ok(fy)
    }

    /// La página del artista: primero por el canal de la canción; si ese canal no es de
    /// un artista (subidas de terceros), por el nombre.
    pub async fn artist(&self, channel_id: Option<&str>, name: Option<&str>) -> Result<crate::resolver::ytmusic::Artist> {
        if let Some(id) = channel_id {
            if let Some(a) = self.catalog.artist(id).await? {
                // El canal puede ser de un sello que YouTube Music trata como artista
                // (88rising sube a Joji): solo vale si se llama como el artista buscado.
                let same = |x: &str, y: &str| x.to_lowercase().contains(&y.to_lowercase()) || y.to_lowercase().contains(&x.to_lowercase());
                // Un canal de YouTube que no es el del artista en Music (el de League of
                // Legends) sale como "artista" pero solo con videos: si no trae canciones,
                // se prueba la búsqueda por nombre y se queda con la que sí las tenga.
                if name.is_none_or(|n| same(&a.name, n)) {
                    if !a.top.is_empty() || name.is_none() {
                        return Ok(a);
                    }
                    if let Ok(id) = self.catalog.find_artist(name.unwrap_or_default()).await {
                        if let Ok(Some(b)) = self.catalog.artist(&id).await {
                            if !b.top.is_empty() {
                                return Ok(b);
                            }
                        }
                    }
                    return Ok(a);
                }
            }
        }
        let Some(name) = name.filter(|n| !n.trim().is_empty()) else {
            bail!("no sé de qué artista es esta canción");
        };
        let id = self.catalog.find_artist(name).await?;
        match self.catalog.artist(&id).await? {
            Some(a) => Ok(a),
            None => bail!("no encontré la página de «{name}»"),
        }
    }

    /// Solo busca: la cola queda como estaba.
    pub async fn find(&self, query: &str, limit: usize) -> Result<Vec<Track>> {
        self.resolver.search(query, limit).await
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
        }
        // Adelanta el primero: es el que se elige la mayoria de las veces.
        self.spawn_prefetch(0);
        Ok(found)
    }

    /// Letra de lo que suena ahora. `None` si no hay pista o no se encontro.
    pub async fn lyrics(&self) -> Result<Option<Lyrics>> {
        let Some(track) = self.state.lock().await.now_playing.clone() else {
            return Ok(None);
        };
        self.lyrics.fetch(&track).await
    }

    /// Mueve el desfase de la letra actual y lo deja guardado.
    pub async fn nudge_lyrics(&self, delta: f64) -> Result<f64> {
        let Some(track) = self.state.lock().await.now_playing.clone() else {
            anyhow::bail!("no hay nada sonando");
        };
        let current = self
            .lyrics
            .fetch(&track)
            .await?
            .map(|l| l.offset)
            .unwrap_or(0.0);
        let next = current + delta;
        self.lyrics.save_offset(&track, next).await?;
        Ok(next)
    }

    pub async fn queue(&self) -> Vec<Track> {
        self.state.lock().await.queue.clone()
    }

    pub async fn clear_queue(&self) -> Result<()> {
        self.finish("stop").await;
        let mut st = self.state.lock().await;
        st.queue.clear();
        st.current = None;
        st.now_playing = None;
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
                // mpv no pudo abrir la URL: lo normal es que caducara o que cambiara la IP
                // (las URLs van atadas a ella). Se olvida y se vuelve a resolver, una vez.
                if msg.get("reason").and_then(|v| v.as_str()) == Some("error") {
                    let retry = {
                        let mut st = me.state.lock().await;
                        // Otro intento solo si no se reintentó esta misma hace nada: antes
                        // quedaba bloqueada para siempre tras el primer reintento.
                        let fresh = |id: &String| st.retried.as_ref().is_some_and(|(r, at)| r == id && at.elapsed().as_secs() < 30);
                        match (st.current, st.now_playing.as_ref().map(|t| t.id.clone())) {
                            (Some(idx), Some(id)) if !fresh(&id) => {
                                st.retried = Some((id.clone(), std::time::Instant::now()));
                                // Sin `now_playing` el reintento no deja en el historial un salto falso.
                                st.now_playing = None;
                                Some((idx, id))
                            }
                            _ => None,
                        }
                    };
                    // La URL que falló no se vuelve a usar, haya reintento o no.
                    if let Some(id) = me.state.lock().await.now_playing.as_ref().map(|t| t.id.clone()) {
                        me.urls.forget(&id);
                    }
                    if let Some((idx, id)) = retry {
                        me.urls.forget(&id);
                        if let Err(e) = me.play_index(idx).await {
                            eprintln!("surco: no se pudo reabrir la pista: {e}");
                        }
                    }
                    continue;
                }
                if msg.get("reason").and_then(|v| v.as_str()) != Some("eof") {
                    continue;
                }
                me.finish("eof").await;
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
