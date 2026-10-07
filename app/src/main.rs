//! surco como app: biblioteca y reproductor al estilo Spotify, con la estética de Iris.
//! Al tocar la canción se abre la vista completa con el núcleo bailando y la letra.
//! Habla con el daemon por el mismo socket que el CLI.

mod api;
mod bench;
mod cargando;
mod musica;
mod nucleo;
mod puntos;
mod theme;
mod ui;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use api::{Album, Artist, ArtistRef, ForYou, Library, Listing, Lyrics, More, Track};
use iced::keyboard::{self, Key, key::Named};
use iced::widget::image;
use iced::{Color, Font, Size, Subscription, Task, event, time, window};
use serde_json::{Value, json};

/// Separación entre puntos del núcleo, en píxeles lógicos.
pub const SPACING: f32 = 5.0;

fn main() -> iced::Result {
    if std::env::args().any(|a| a == "--bench") {
        bench::run();
        return Ok(());
    }
    iced::application(App::new, App::update, App::view)
        .title("surco")
        .subscription(App::subscription)
        .theme(App::theme)
        .style(|_, _| iced::theme::Style { background_color: Color::TRANSPARENT, text_color: ui::rgb(theme::text_rgb(), 1.0) })
        .default_font(Font::MONOSPACE)
        .window(window::Settings {
            size: Size::new(1280.0, 800.0),
            min_size: Some(Size::new(820.0, 520.0)),
            transparent: std::env::var_os("SURCO_OPACO").is_none(),
            platform_specific: window::settings::PlatformSpecific { application_id: "surco".into(), ..Default::default() },
            ..Default::default()
        })
        .run()
}

#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    Home,
    ForYou,
    Search,
    Favorites,
    Playlist(String),
    /// Canal del artista si se sabe, y su nombre para buscarlo si no.
    Artist(Option<String>, String),
    Album(String),
    /// El "ver todo" de una sección: su título (con el artista) y a dónde lleva.
    Listing(String, More),
}

/// Lo que se pidió reproducir y el daemon aún no confirma (yt-dlp tarda ~3 s).
pub struct Pending {
    /// La pista pedida; `None` en siguiente/anterior, donde la app no sabe cuál es.
    pub track: Option<Track>,
    /// Para no confundir la respuesta de un clic viejo con la del último.
    seq: u64,
    /// Cuándo contestó el daemon; se suelta con el primer status que ya trae la nueva.
    answered: Option<Instant>,
}

pub enum Thumb {
    Loading,
    Ready(image::Handle),
    Missing,
}

pub struct App {
    pub core: nucleo::Core,
    musica: musica::Musica,
    last: Instant,
    pub size: Size,
    theme_check: Instant,
    /// `SURCO_FPS=1`: cuadros por segundo y ms de la vista, cada segundo por stderr.
    fps: Option<(Instant, u32)>,
    pub view_cpu: std::cell::Cell<Duration>,

    /// El último `status` del daemon; `None` si no responde.
    pub status: Option<Value>,
    pub library: Library,
    pub recent: Vec<Track>,
    pub page: Page,
    /// Páginas anteriores, para el botón de atrás.
    pub back: Vec<Page>,
    /// La página de artista o álbum abierta: `None` mientras carga.
    pub artist: Option<Result<Artist, String>>,
    pub album: Option<Result<Album, String>>,
    pub listing: Option<Result<Listing, String>>,
    /// "Para ti": `None` hasta la primera respuesta.
    pub for_you: Option<Result<ForYou, String>>,
    /// Se está armando una mezcla (la primera o "otra mezcla").
    pub mixing: bool,
    pub query: String,
    pub results: Vec<Track>,
    pub searching: bool,
    /// Nombre de la playlist que se está creando, si el campo está abierto.
    pub new_playlist: Option<String>,
    /// Pista cuyo menú "añadir a playlist" está abierto.
    pub menu: Option<String>,
    pub thumbs: HashMap<String, Thumb>,
    /// Vista completa: el núcleo grande y la letra.
    pub full: bool,
    pub lyrics: Option<Lyrics>,
    lyrics_for: Option<String>,
    /// Mientras se arrastra la barra de progreso o el volumen, el valor que se ve.
    pub seeking: Option<f32>,
    pub volume: Option<f32>,
    /// Un aviso corto abajo (errores del daemon, "añadida a…").
    pub toast: Option<(String, Instant)>,
    pub pending: Option<Pending>,
    seq: u64,
    /// La pista bajo el mouse; si se queda ahí un rato, se adelanta su URL.
    hover: Option<String>,
    /// Lo ya adelantado, para no pedirlo otra vez en cada pasada del mouse.
    warmed: HashMap<String, Instant>,
    /// Origen del reloj de las animaciones de carga.
    pub born: Instant,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Frame(Instant),
    Resized(Size),
    Poll,
    Status(Option<Value>),
    Library(Result<Library, String>),
    Recent(Result<Vec<api::Play>, String>),
    Nav(Page),
    Back,
    ArtistLoaded(Page, Result<Artist, String>),
    AlbumLoaded(Page, Result<Album, String>),
    ListingLoaded(Page, Result<Listing, String>),
    ForYouLoaded(Result<ForYou, String>),
    /// "Otra mezcla": semillas nuevas.
    Remix,
    Query(String),
    Search,
    Results(Result<Vec<Track>, String>),
    PlayList(Vec<Track>, usize),
    Enqueue(Track),
    Favorite(Track, bool),
    Follow(ArtistRef, bool),
    Menu(Option<String>),
    AddTo(String, Track),
    RemoveFrom(String, String),
    NewPlaylist(Option<String>),
    CreatePlaylist,
    DeletePlaylist(String),
    Control(&'static str),
    Seek(f32),
    SeekDone,
    Volume(f32),
    VolumeDone,
    Full(bool),
    Lyrics(String, Option<Lyrics>),
    Thumb(String, Option<Vec<u8>>),
    /// Respuesta de una orden: si cambió la biblioteca, se recarga.
    Done(Result<Value, String>, bool),
    /// Respuesta a reproducir algo (play_tracks, next, prev), con su número de pedido.
    Started(u64, Result<Value, String>),
    /// El mouse entró en una pista o salió de ella (por id: al pasar de una fila a otra la
    /// entrada a la nueva puede llegar antes que la salida de la vieja).
    Hover(Track),
    Unhover(String),
    /// Pasó el rato de espera: si el mouse sigue en esa pista, se adelanta.
    Dwell(Track),
    Key(Key),
}

const SEARCH_INPUT: &str = "buscar";
/// Cuánto tiene que quedarse el mouse sobre una pista para adelantar su URL.
const HOVER_DWELL: Duration = Duration::from_millis(150);
/// El daemon guarda las URLs unas horas; pasada una, se le vuelve a avisar por si se reinició.
const WARM_TTL: Duration = Duration::from_secs(3600);

impl App {
    fn new() -> (Self, Task<Msg>) {
        theme::poll();
        let app = App {
            core: nucleo::Core::new(),
            musica: musica::Musica::spawn(),
            last: Instant::now(),
            size: Size::new(1280.0, 800.0),
            theme_check: Instant::now(),
            fps: std::env::var_os("SURCO_FPS").map(|_| (Instant::now(), 0)),
            view_cpu: Default::default(),
            status: None,
            library: Library::default(),
            recent: vec![],
            page: Page::Home,
            back: vec![],
            artist: None,
            album: None,
            listing: None,
            for_you: None,
            mixing: true,
            query: String::new(),
            results: vec![],
            searching: false,
            new_playlist: None,
            menu: None,
            thumbs: HashMap::new(),
            full: false,
            lyrics: None,
            lyrics_for: None,
            seeking: None,
            volume: None,
            toast: None,
            pending: None,
            seq: 0,
            hover: None,
            warmed: HashMap::new(),
            born: Instant::now(),
        };
        let mut boot = vec![Task::done(Msg::Poll), reload_library(), reload_recent(), load_for_you(false)];
        // `SURCO_INICIO`, para capturas de desarrollo: `completa`, `favoritos`, `buscar=<texto>`,
        // `artista=<nombre_con_guiones_bajos>`.
        match std::env::var("SURCO_INICIO").ok().as_deref() {
            Some("completa") => boot.push(Task::done(Msg::Full(true))),
            Some("favoritos") => boot.push(Task::done(Msg::Nav(Page::Favorites))),
            Some("parati") => boot.push(Task::done(Msg::Nav(Page::ForYou))),
            Some(q) if q.starts_with("artista=") => boot.push(Task::done(Msg::Nav(Page::Artist(None, q["artista=".len()..].replace('_', " "))))),
            Some(q) if q.starts_with("album=") => boot.push(Task::done(Msg::Nav(Page::Album(q["album=".len()..].to_string())))),
            Some(q) if q.starts_with("buscar=") => {
                boot.push(Task::done(Msg::Nav(Page::Search)));
                boot.push(Task::done(Msg::Query(q["buscar=".len()..].to_string())));
                boot.push(Task::done(Msg::Search));
            }
            _ => {}
        }
        (app, Task::batch(boot))
    }

    pub fn current(&self) -> Option<Track> {
        self.status.as_ref()?.get("current").and_then(|c| serde_json::from_value(c.clone()).ok())
    }

    pub fn playback(&self, key: &str) -> Option<&Value> {
        self.status.as_ref()?.pointer(&format!("/playback/{key}"))
    }

    /// mpv cargó la pista pero aún no suena: llenando el búfer.
    pub fn buffering(&self) -> bool {
        self.current().is_some()
            && !self.playback("paused").and_then(Value::as_bool).unwrap_or(true)
            && self.playback("idle").and_then(Value::as_bool).unwrap_or(false)
    }

    /// Hay algo en camino de sonar: pedido al daemon o en el búfer de mpv.
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.buffering()
    }

    /// La pista que está por sonar, si se sabe cuál es.
    pub fn loading_id(&self) -> Option<String> {
        match &self.pending {
            Some(p) => p.track.as_ref().map(|t| t.id.clone()),
            None if self.buffering() => self.current().map(|t| t.id),
            None => None,
        }
    }

    /// Segundos para las animaciones de carga.
    pub fn anim(&self) -> f32 {
        self.born.elapsed().as_secs_f32()
    }

    /// Algo se está esperando y hay que dibujarlo moviéndose.
    fn waiting(&self) -> bool {
        self.busy()
            || self.searching
            || self.mixing && matches!(self.page, Page::Home | Page::ForYou)
            || matches!(self.page, Page::Artist(..)) && self.artist.is_none()
            || matches!(self.page, Page::Album(_)) && self.album.is_none()
            || matches!(self.page, Page::Listing(..)) && self.listing.is_none()
            || self.thumbs.values().any(|t| matches!(t, Thumb::Loading))
    }

    /// Pide reproducir y deja la marca de "en camino" hasta que el daemon confirme.
    fn start(&mut self, track: Option<Track>, req: Value) -> Task<Msg> {
        self.seq += 1;
        let seq = self.seq;
        self.pending = Some(Pending { track, seq, answered: None });
        Task::perform(api::ask(req), move |r| Msg::Started(seq, r))
    }

    pub fn is_favorite(&self, id: &str) -> bool {
        self.library.favorites.iter().any(|t| t.id == id)
    }

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Frame(now) => {
                let dt = now.saturating_duration_since(self.last).as_secs_f64().min(0.1);
                self.last = now;
                if self.theme_check.elapsed() >= Duration::from_secs(1) {
                    self.theme_check = Instant::now();
                    theme::poll();
                }
                if let Some((_, t)) = &self.toast {
                    if t.elapsed() > Duration::from_secs(4) {
                        self.toast = None;
                    }
                }
                if self.full {
                    let sig = nucleo::Signals { music: self.musica.level(), ..Default::default() };
                    self.core.step(dt, self.want(), &sig);
                }
                if let Some((since, n)) = &mut self.fps {
                    *n += 1;
                    if since.elapsed() >= Duration::from_secs(1) {
                        let v = self.view_cpu.take();
                        eprintln!("{} fps, vista {:.2} ms", n, v.as_secs_f64() * 1000.0 / *n as f64);
                        (*since, *n) = (Instant::now(), 0);
                    }
                }
            }
            Msg::Resized(s) => self.size = s,
            Msg::Poll => {
                return Task::perform(api::ask(api::cmd("status")), |r| Msg::Status(r.ok().and_then(|v| v.get("payload").cloned())));
            }
            Msg::Status(v) => {
                let before = self.current().map(|t| t.id);
                self.status = v;
                // Un status pedido antes de la respuesta aún trae la pista vieja: esperar al
                // que traiga la nueva (o un segundo, si el daemon sonó otra cosa).
                let now_id = self.current().map(|t| t.id);
                if self.pending.as_ref().is_some_and(|p| {
                    p.answered.is_some_and(|at| {
                        p.track.as_ref().is_none_or(|t| Some(&t.id) == now_id.as_ref()) || at.elapsed() > Duration::from_secs(1)
                    })
                }) {
                    self.pending = None;
                }
                let now = self.current();
                let mut tasks = vec![];
                if let Some(t) = &now {
                    tasks.push(self.want_thumbs(std::slice::from_ref(t)));
                    if self.lyrics_for.as_ref() != Some(&t.id) {
                        self.lyrics_for = Some(t.id.clone());
                        self.lyrics = None;
                        let id = t.id.clone();
                        tasks.push(Task::perform(api::get::<Option<Lyrics>>(api::cmd("lyrics")), move |r| Msg::Lyrics(id.clone(), r.ok().flatten())));
                    }
                }
                // Cambió la canción: la anterior ya está en el historial.
                if before != now.map(|t| t.id) {
                    tasks.push(reload_recent());
                }
                return Task::batch(tasks);
            }
            Msg::Library(Ok(lib)) => {
                self.library = lib;
                let all: Vec<Track> =
                    self.library.favorites.iter().chain(self.library.playlists.iter().flat_map(|p| &p.tracks)).cloned().collect();
                if let Page::Playlist(id) = &self.page {
                    if !self.library.playlists.iter().any(|p| &p.id == id) {
                        self.page = Page::Home;
                    }
                }
                let photos = self.library.artists.iter().filter_map(|a| Some((format!("foto:{}", a.id), a.image.clone()?))).collect();
                return Task::batch([self.want_thumbs(&all), self.want_urls(photos)]);
            }
            Msg::Recent(Ok(plays)) => {
                self.recent = plays.into_iter().map(|p| p.track).collect();
                return self.want_thumbs(&self.recent.clone());
            }
            Msg::Library(Err(e)) | Msg::Recent(Err(e)) => self.notify(e),
            Msg::Nav(page) => {
                if page != self.page {
                    self.back.push(std::mem::replace(&mut self.page, page.clone()));
                    if self.back.len() > 50 {
                        self.back.remove(0);
                    }
                }
                return self.open(page);
            }
            Msg::Back => {
                if let Some(p) = self.back.pop() {
                    self.page = p.clone();
                    return self.open(p);
                }
            }
            Msg::ArtistLoaded(page, r) => {
                if page == self.page {
                    if let Ok(a) = &r {
                        let mut urls: Vec<(String, String)> =
                            a.sections.iter().flat_map(|s| &s.items).filter_map(|i| Some((i.id.clone(), i.thumb.clone()?))).collect();
                        if let Some(img) = &a.image {
                            urls.push((format!("foto:{}", a.id), img.clone()));
                        }
                        let top = a.top.clone();
                        self.artist = Some(r);
                        return Task::batch([self.want_urls(urls), self.want_thumbs(&top)]);
                    }
                    self.artist = Some(r);
                }
            }
            Msg::ListingLoaded(page, r) => {
                if page == self.page {
                    let mut t = vec![];
                    if let Ok(l) = &r {
                        t.push(self.want_urls(l.items.iter().filter_map(|i| Some((i.id.clone(), i.thumb.clone()?))).collect()));
                        t.push(self.want_thumbs(&l.tracks));
                    }
                    self.listing = Some(r);
                    return Task::batch(t);
                }
            }
            Msg::AlbumLoaded(page, r) => {
                if page == self.page {
                    let t = match &r {
                        Ok(a) => a.thumb.clone().map(|u| self.want_urls(vec![(a.id.clone(), u)])),
                        Err(_) => None,
                    };
                    self.album = Some(r);
                    return t.unwrap_or_else(Task::none);
                }
            }
            Msg::ForYouLoaded(r) => {
                self.mixing = false;
                let t = match &r {
                    Ok(f) => self.want_thumbs(&f.tracks.clone()),
                    Err(_) => Task::none(),
                };
                // Si falla una mezcla nueva, queda la anterior y se avisa.
                match (r, &self.for_you) {
                    (Err(e), Some(Ok(_))) => self.notify(e),
                    (r, _) => self.for_you = Some(r),
                }
                return t;
            }
            Msg::Remix => {
                if !self.mixing {
                    self.mixing = true;
                    return load_for_you(true);
                }
            }
            Msg::Query(q) => self.query = q,
            Msg::Search => {
                if self.query.trim().is_empty() {
                    return Task::none();
                }
                self.searching = true;
                return Task::perform(api::get(json!({ "cmd": "find", "query": self.query, "limit": 25 })), Msg::Results);
            }
            Msg::Results(r) => {
                self.searching = false;
                match r {
                    Ok(t) => {
                        self.results = t;
                        return self.want_thumbs(&self.results.clone());
                    }
                    Err(e) => self.notify(e),
                }
            }
            Msg::PlayList(tracks, index) => {
                self.menu = None;
                let t = tracks.get(index).cloned();
                return self.start(t, json!({ "cmd": "play_tracks", "tracks": tracks, "index": index }));
            }
            Msg::Enqueue(t) => {
                self.notify(format!("en cola: {}", t.title));
                return send(json!({ "cmd": "enqueue_track", "track": t }), false);
            }
            Msg::Favorite(t, on) => {
                // Se ve al instante; el daemon confirma después.
                self.library.favorites.retain(|f| f.id != t.id);
                if on {
                    self.library.favorites.insert(0, t.clone());
                }
                return send(json!({ "cmd": "favorite", "track": t, "on": on }), true);
            }
            Msg::Follow(a, on) => {
                self.library.artists.retain(|x| x.id != a.id);
                if on {
                    self.library.artists.insert(0, a.clone());
                }
                return send(json!({ "cmd": "follow", "artist": a, "on": on }), true);
            }
            Msg::Menu(id) => self.menu = id,
            Msg::AddTo(id, t) => {
                self.menu = None;
                let name = self.library.playlists.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default();
                self.notify(format!("añadida a «{name}»"));
                return send(json!({ "cmd": "playlist_add", "id": id, "track": t }), true);
            }
            Msg::RemoveFrom(id, track_id) => return send(json!({ "cmd": "playlist_remove", "id": id, "track_id": track_id }), true),
            Msg::NewPlaylist(name) => self.new_playlist = name,
            Msg::CreatePlaylist => {
                let Some(name) = self.new_playlist.take() else { return Task::none() };
                return Task::perform(api::ask(json!({ "cmd": "playlist_create", "name": name })), |r| match r {
                    Ok(v) => match v.pointer("/payload/id").and_then(Value::as_str) {
                        Some(id) => Msg::Nav(Page::Playlist(id.to_string())),
                        None => Msg::Done(Ok(v), true),
                    },
                    Err(e) => Msg::Done(Err(e), false),
                })
                .chain(reload_library());
            }
            Msg::DeletePlaylist(id) => {
                self.page = Page::Home;
                return send(json!({ "cmd": "playlist_delete", "id": id }), true);
            }
            Msg::Control(c @ ("next" | "prev")) => return self.start(None, api::cmd(c)),
            Msg::Control(c) => return send(api::cmd(c), false),
            Msg::Unhover(id) => {
                if self.hover.as_ref() == Some(&id) {
                    self.hover = None;
                }
            }
            Msg::Hover(t) => {
                self.hover = Some(t.id.clone());
                // Barrer una lista con el mouse no debe lanzar un yt-dlp por fila: solo
                // cuenta si se queda encima.
                return Task::perform(tokio::time::sleep(HOVER_DWELL), move |_| Msg::Dwell(t.clone()));
            }
            Msg::Dwell(t) => {
                let fresh = self.warmed.get(&t.id).is_some_and(|at| at.elapsed() < WARM_TTL);
                if self.hover.as_ref() == Some(&t.id) && !fresh && self.current().is_none_or(|c| c.id != t.id) {
                    self.warmed.insert(t.id.clone(), Instant::now());
                    return Task::future(api::ask(json!({ "cmd": "warm", "track": t }))).discard();
                }
            }
            Msg::Started(seq, r) => {
                if self.pending.as_ref().is_some_and(|p| p.seq == seq) {
                    match r {
                        Ok(_) => self.pending.as_mut().unwrap().answered = Some(Instant::now()),
                        Err(e) => {
                            self.pending = None;
                            self.notify(e);
                        }
                    }
                }
                return Task::done(Msg::Poll);
            }
            Msg::Seek(v) => self.seeking = Some(v),
            Msg::SeekDone => {
                if let Some(v) = self.seeking.take() {
                    return send(json!({ "cmd": "seek", "seconds": v }), false);
                }
            }
            Msg::Volume(v) => self.volume = Some(v),
            Msg::VolumeDone => {
                if let Some(v) = self.volume.take() {
                    return send(json!({ "cmd": "volume", "level": v }), false);
                }
            }
            Msg::Full(on) => {
                self.full = on;
                self.menu = None;
                self.last = Instant::now();
            }
            Msg::Lyrics(id, l) => {
                if self.lyrics_for.as_ref() == Some(&id) {
                    self.lyrics = l;
                }
            }
            Msg::Thumb(id, bytes) => {
                self.thumbs.insert(id, bytes.map_or(Thumb::Missing, |b| Thumb::Ready(image::Handle::from_bytes(b))));
            }
            Msg::Done(r, library_changed) => {
                if let Err(e) = r {
                    self.notify(e);
                }
                let mut t = vec![Task::done(Msg::Poll)];
                if library_changed {
                    t.push(reload_library());
                }
                return Task::batch(t);
            }
            Msg::Key(key) => match key.as_ref() {
                Key::Named(Named::Space) => return send(api::cmd("toggle"), false),
                Key::Named(Named::Escape) if self.full => self.full = false,
                Key::Named(Named::Escape) if self.menu.is_some() => self.menu = None,
                Key::Named(Named::Escape) | Key::Named(Named::Backspace) => return self.update(Msg::Back),
                Key::Character("/") => return self.update(Msg::Nav(Page::Search)),
                _ => {}
            },
        }
        Task::none()
    }

    /// Entrar a una página: cierra menús y la vista completa, y pide lo que haga falta.
    fn open(&mut self, page: Page) -> Task<Msg> {
        self.menu = None;
        self.full = false;
        match page {
            Page::Search => iced::widget::operation::focus(SEARCH_INPUT),
            Page::Artist(id, name) => {
                self.artist = None;
                let p = Page::Artist(id.clone(), name.clone());
                Task::perform(api::get(json!({ "cmd": "artist", "channel_id": id, "name": name })), move |r| Msg::ArtistLoaded(p.clone(), r))
            }
            Page::Listing(title, more) => {
                self.listing = None;
                let p = Page::Listing(title, more.clone());
                Task::perform(api::get(json!({ "cmd": "listing", "id": more.id, "params": more.params })), move |r| Msg::ListingLoaded(p.clone(), r))
            }
            Page::Album(id) => {
                self.album = None;
                let p = Page::Album(id.clone());
                Task::perform(api::get(json!({ "cmd": "album", "id": id })), move |r| Msg::AlbumLoaded(p.clone(), r))
            }
            _ => Task::none(),
        }
    }

    /// Imágenes que no son portadas de video (fotos de artista, álbumes), por URL.
    fn want_urls(&mut self, items: Vec<(String, String)>) -> Task<Msg> {
        let mut tasks = vec![];
        for (key, url) in items {
            if !self.thumbs.contains_key(&key) {
                self.thumbs.insert(key.clone(), Thumb::Loading);
                tasks.push(Task::perform(api::thumb(key, Some(url)), |(id, b)| Msg::Thumb(id, b)));
            }
        }
        Task::batch(tasks)
    }

    fn notify(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), Instant::now()));
    }

    /// Pide las portadas que falten; cada una llega sola como `Msg::Thumb`.
    fn want_thumbs(&mut self, tracks: &[Track]) -> Task<Msg> {
        let mut tasks = vec![];
        for t in tracks {
            if !self.thumbs.contains_key(&t.id) {
                self.thumbs.insert(t.id.clone(), Thumb::Loading);
                tasks.push(Task::perform(api::thumb(t.id.clone(), None), |(id, b)| Msg::Thumb(id, b)));
            }
        }
        Task::batch(tasks)
    }

    /// Qué hace el núcleo según el reproductor: baila sonando, duerme en pausa.
    fn want(&self) -> nucleo::State {
        if self.status.is_none() {
            return nucleo::State::Offline;
        }
        match self.playback("paused").and_then(Value::as_bool) {
            Some(true) if self.current().is_some() => nucleo::State::Sleeping,
            _ => nucleo::State::Idle,
        }
    }

    fn theme(&self) -> iced::Theme {
        iced::Theme::custom(
            "surco",
            iced::theme::Palette {
                background: ui::rgb(theme::bg_rgb(), 1.0),
                text: ui::rgb(theme::text_rgb(), 1.0),
                primary: ui::rgb(theme::accent_rgb(), 1.0),
                success: ui::rgb(theme::accent_rgb(), 1.0),
                warning: Color::from_rgb8(0xe8, 0xa0, 0x30),
                danger: Color::from_rgb8(0xe0, 0x50, 0x50),
            },
        )
    }

    fn subscription(&self) -> Subscription<Msg> {
        let mut subs = vec![
            window::resize_events().map(|(_, s)| Msg::Resized(s)),
            time::every(Duration::from_millis(500)).map(|_| Msg::Poll),
            // Solo las teclas que nadie usó: escribir un espacio en la búsqueda no pausa.
            event::listen_with(|e, status, _| match (e, status) {
                (iced::Event::Keyboard(keyboard::Event::KeyPressed { key, .. }), event::Status::Ignored) => Some(Msg::Key(key)),
                _ => None,
            }),
        ];
        // El núcleo solo se anima con la vista completa abierta; el resto de la app no
        // necesita cuadros continuos.
        if self.full || self.toast.is_some() || self.waiting() {
            subs.push(window::frames().map(Msg::Frame));
        }
        Subscription::batch(subs)
    }
}

fn reload_library() -> Task<Msg> {
    Task::perform(api::get(api::cmd("library")), Msg::Library)
}

fn load_for_you(refresh: bool) -> Task<Msg> {
    Task::perform(api::get(json!({ "cmd": "for_you", "refresh": refresh })), Msg::ForYouLoaded)
}

fn reload_recent() -> Task<Msg> {
    Task::perform(api::get(json!({ "cmd": "history", "limit": 24 })), Msg::Recent)
}

fn send(req: Value, library_changed: bool) -> Task<Msg> {
    Task::perform(api::ask(req), move |r| Msg::Done(r, library_changed))
}
