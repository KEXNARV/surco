//! Cómo se ve: barra lateral con la biblioteca, centro con inicio, búsqueda y listas,
//! reproductor abajo. Paneles oscuros semitransparentes (Hyprland pone el blur) con el
//! acento del tema de Omarchy, como Iris.

use std::time::Instant;

use iced::widget::{
    Column, Space, button, column, container, image, mouse_area, row, scrollable, shader, slider, stack, text, text_input,
};
use iced::{Alignment, Background, Border, Color, ContentFit, Element, Fill, Length, Padding};
use serde_json::Value;

use crate::api::{Album, Artist, Item, Track};
use crate::{App, Msg, Page, SEARCH_INPUT, SPACING, Thumb, cargando, puntos, theme};

pub fn rgb(c: [f64; 3], a: f32) -> Color {
    Color::from_rgba8(c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, a)
}

fn accent() -> Color {
    rgb(theme::accent_rgb(), 1.0)
}
fn faint() -> Color {
    rgb(theme::faint_rgb(), 1.0)
}
fn dim() -> Color {
    rgb(theme::dim_rgb(), 1.0)
}
fn fg() -> Color {
    rgb(theme::text_rgb(), 1.0)
}

/// Panel: el fondo del tema, translúcido y con esquinas redondeadas.
fn panel<'a>(content: impl Into<Element<'a, Msg>>) -> container::Container<'a, Msg> {
    container(content).style(|_| container::Style {
        background: Some(rgb(theme::bg_rgb(), 0.93).into()),
        border: Border { radius: 10.0.into(), width: 1.0, color: dim().scale_alpha(0.35) },
        ..Default::default()
    })
}

/// Botón sin caja: solo texto, que se ilumina al pasar por encima.
fn flat<'a>(content: impl Into<Element<'a, Msg>>, active: bool) -> button::Button<'a, Msg> {
    button(content).padding([6, 10]).style(move |_, status| {
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: (hover || active).then(|| Background::from(accent().scale_alpha(if active { 0.16 } else { 0.08 }))),
            text_color: if active { accent() } else { fg() },
            border: Border { radius: 6.0.into(), ..Default::default() },
            ..Default::default()
        }
    })
}

/// Un icono que es botón (♥, ⏯, +…).
fn icon<'a>(glyph: &'a str, size: u32, color: Color) -> button::Button<'a, Msg> {
    button(text(glyph).size(size as f32).color(color)).padding([2, 6]).style(move |_, status| button::Style {
        background: matches!(status, button::Status::Hovered).then(|| Background::from(accent().scale_alpha(0.12))),
        text_color: color,
        border: Border { radius: 20.0.into(), ..Default::default() },
        ..Default::default()
    })
}

/// Avisa cuando el mouse entra o sale de una pista, para adelantar su URL.
fn hoverable<'a>(content: impl Into<Element<'a, Msg>>, t: &Track) -> Element<'a, Msg> {
    mouse_area(content).on_enter(Msg::Hover(t.clone())).on_exit(Msg::Unhover(t.id.clone())).into()
}

/// El nombre del artista como enlace a su página.
fn who_link<'a>(t: &Track, size: u32) -> Element<'a, Msg> {
    let who = t.who().to_string();
    if who.is_empty() {
        return Space::new().into();
    }
    let page = Page::Artist(t.channel_id.clone(), who.clone());
    button(text(short(&who, 40)).size(size as f32))
        .padding(0)
        .style(|_, status| button::Style {
            text_color: if matches!(status, button::Status::Hovered) { fg() } else { faint() },
            ..Default::default()
        })
        .on_press(Msg::Nav(page))
        .into()
}

pub fn clock(s: f64) -> String {
    let s = s.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

impl App {
    fn cover(&self, id: &str, size: f32) -> Element<'_, Msg> {
        // Mientras baja, el hueco respira; si no hay portada, queda quieto.
        let alpha = match self.thumbs.get(id) {
            Some(Thumb::Ready(h)) => {
                return image(h.clone()).width(size).height(size).content_fit(ContentFit::Cover).border_radius(size / 10.0).into();
            }
            Some(Thumb::Loading) => cargando::pulse(self.anim(), 0.12, 0.32),
            _ => 0.25,
        };
        container(Space::new().width(size).height(size))
                .style(move |_| container::Style {
                    background: Some(dim().scale_alpha(alpha).into()),
                    border: Border { radius: (size / 10.0).into(), ..Default::default() },
                    ..Default::default()
                })
                .into()
    }

    /// La portada con el giro encima si esa pista es la que está por sonar.
    fn cover_loading(&self, id: &str, size: f32) -> Element<'_, Msg> {
        let cover = self.cover(id, size);
        if self.loading_id().as_deref() != Some(id) {
            return cover;
        }
        let veil = container(cargando::spinner(self.anim(), (size * 0.36).max(14.0), accent()))
            .width(size)
            .height(size)
            .center(size)
            .style(move |_| container::Style {
                background: Some(rgb(theme::bg_rgb(), 0.55).into()),
                border: Border { radius: (size / 10.0).into(), ..Default::default() },
                ..Default::default()
            });
        stack![cover, veil].into()
    }

    /// "cargando…" con el giro delante.
    fn waiting_line<'a>(&self, label: String) -> Element<'a, Msg> {
        row![cargando::spinner(self.anim(), 18.0, accent()), text(label).color(faint())].spacing(10).align_y(Alignment::Center).into()
    }

    pub fn view(&self) -> Element<'_, Msg> {
        let t0 = Instant::now();
        let body: Element<Msg> = if self.full {
            self.full_view()
        } else {
            row![self.sidebar(), self.main()].spacing(8).height(Fill).into()
        };
        let mut layers = column![body, self.player()].spacing(8).padding(8);
        if let Some((msg, _)) = &self.toast {
            layers = layers.push(container(text(msg).size(13).color(faint())).padding([0, 8]));
        }
        let out = container(layers)
            .width(Fill)
            .height(Fill)
            .style(|_| container::Style { background: Some(rgb(theme::bg_rgb(), 0.55).into()), ..Default::default() })
            .into();
        self.view_cpu.set(self.view_cpu.get() + t0.elapsed());
        out
    }

    fn sidebar(&self) -> Element<'_, Msg> {
        let nav = |label: &'static str, page: Page| flat(text(label).size(15), self.page == page).width(Fill).on_press(Msg::Nav(page));
        let mut lib = Column::new().spacing(2);
        lib = lib.push(
            flat(row![text("♥").color(accent()), text(format!("Favoritos  {}", self.library.favorites.len())).size(14)].spacing(8), self.page == Page::Favorites)
                .width(Fill)
                .on_press(Msg::Nav(Page::Favorites)),
        );
        for a in &self.library.artists {
            let page = Page::Artist(Some(a.id.clone()), a.name.clone());
            let photo: Element<Msg> = match self.thumbs.get(&format!("foto:{}", a.id)) {
                Some(Thumb::Ready(h)) => image(h.clone()).width(26).height(26).content_fit(ContentFit::Cover).border_radius(13.0).into(),
                _ => text("◉").color(faint()).into(),
            };
            lib = lib.push(
                flat(row![photo, text(&a.name).size(14)].spacing(8).align_y(Alignment::Center), self.page == page).width(Fill).on_press(Msg::Nav(page)),
            );
        }
        for p in &self.library.playlists {
            let page = Page::Playlist(p.id.clone());
            lib = lib.push(
                flat(row![text("▤").color(faint()), text(&p.name).size(14)].spacing(8), self.page == page).width(Fill).on_press(Msg::Nav(page)),
            );
        }
        let create: Element<Msg> = match &self.new_playlist {
            Some(name) => text_input("nombre de la playlist", name)
                .on_input(|s| Msg::NewPlaylist(Some(s)))
                .on_submit(Msg::CreatePlaylist)
                .size(14)
                .padding(8)
                .into(),
            None => flat(text("+ Nueva playlist").size(14).color(faint()), false).width(Fill).on_press(Msg::NewPlaylist(Some(String::new()))).into(),
        };
        panel(
            column![
                text("s u r c o").size(18).color(accent()),
                Space::new().height(12),
                nav("◇  Inicio", Page::Home),
                nav("⌕  Buscar", Page::Search),
                Space::new().height(16),
                text("TU BIBLIOTECA").size(11).color(faint()),
                scrollable(lib).height(Fill),
                create,
            ]
            .spacing(4)
            .padding(14),
        )
        .width(240)
        .height(Fill)
        .into()
    }

    fn main(&self) -> Element<'_, Msg> {
        let content: Element<Msg> = match &self.page {
            Page::Home => self.home(),
            Page::Search => self.search(),
            Page::Favorites => self.list_page("Favoritos", &format!("{} canciones", self.library.favorites.len()), &self.library.favorites, None),
            Page::Playlist(id) => match self.library.playlists.iter().find(|p| &p.id == id) {
                Some(p) => self.list_page(&p.name, &format!("{} canciones", p.tracks.len()), &p.tracks, Some(&p.id)),
                None => text("esa playlist ya no existe").into(),
            },
            Page::Artist(_, name) => match &self.artist {
                Some(Ok(a)) => self.artist_page(a),
                Some(Err(e)) => text(format!("{name}: {e}")).color(faint()).into(),
                None => self.waiting_line(format!("cargando {name}…")),
            },
            Page::Listing(title, _) => match &self.listing {
                Some(Ok(l)) if !l.tracks.is_empty() => self.list_page(title, &format!("{} canciones", l.tracks.len()), &l.tracks, None),
                Some(Ok(l)) => column![text(title.clone()).size(30), self.item_grid(&l.items, "")].spacing(16).into(),
                Some(Err(e)) => text(e.clone()).color(faint()).into(),
                None => self.waiting_line(format!("cargando {title}…  (las listas largas tardan unos segundos)")),
            },
            Page::Album(_) => match &self.album {
                Some(Ok(a)) => self.album_page(a),
                Some(Err(e)) => text(e.clone()).color(faint()).into(),
                None => self.waiting_line("cargando álbum…".into()),
            },
        };
        let mut col = Column::new();
        if !self.back.is_empty() {
            col = col.push(icon("‹", 22, faint()).on_press(Msg::Back));
        }
        col = col.push(content);
        panel(scrollable(container(col.spacing(6)).padding(20)).height(Fill)).width(Fill).height(Fill).into()
    }

    fn home(&self) -> Element<'_, Msg> {
        let mut col = column![text(greeting()).size(26)].spacing(14);
        if self.recent.is_empty() && self.library.favorites.is_empty() {
            col = col.push(text("Busca algo para empezar: / o ⌕ Buscar.").color(faint()));
        }
        if !self.recent.is_empty() {
            col = col.push(text("Escuchado hace poco").size(18));
            col = col.push(self.cards(&self.recent));
        }
        if !self.library.favorites.is_empty() {
            col = col.push(Space::new().height(8));
            col = col.push(text("Tus favoritos").size(18));
            let top: Vec<Track> = self.library.favorites.iter().take(12).cloned().collect();
            col = col.push(self.cards(&top));
        }
        col.into()
    }

    fn artist_page<'a>(&'a self, a: &'a Artist) -> Element<'a, Msg> {
        let photo: Element<Msg> = match self.thumbs.get(&format!("foto:{}", a.id)) {
            Some(Thumb::Ready(h)) => image(h.clone()).width(180).height(180).content_fit(ContentFit::Cover).border_radius(90.0).into(),
            _ => container(Space::new().width(180).height(180))
                .style(|_| container::Style { background: Some(dim().scale_alpha(0.25).into()), border: Border { radius: 90.0.into(), ..Default::default() }, ..Default::default() })
                .into(),
        };
        let mut info = column![text("ARTISTA").size(11).color(faint()), text(&a.name).size(42)].spacing(4);
        if let Some(l) = &a.listeners {
            info = info.push(text(l).size(13).color(faint()));
        }
        let following = self.library.artists.iter().any(|x| x.id == a.id);
        let follow = button(text(if following { "Siguiendo" } else { "Seguir" }).size(13))
            .padding([7, 16])
            .style(move |_, status| button::Style {
                text_color: if following { accent() } else { fg() },
                border: Border {
                    radius: 20.0.into(),
                    width: 1.0,
                    color: if following || matches!(status, button::Status::Hovered) { accent() } else { faint() },
                },
                ..Default::default()
            })
            .on_press(Msg::Follow(crate::api::ArtistRef { id: a.id.clone(), name: a.name.clone(), image: a.image.clone() }, !following));
        let mut actions = row![].spacing(10).align_y(Alignment::Center);
        if !a.top.is_empty() {
            actions = actions.push(play_button(Msg::PlayList(a.top.clone(), 0)));
        }
        actions = actions.push(follow);
        info = info.push(Space::new().height(6));
        info = info.push(actions);
        let mut col = column![row![photo, info].spacing(24).align_y(Alignment::End)].spacing(14);
        if !a.top.is_empty() {
            col = col.push(section_title("Populares", a.top_more.as_ref().map(|m| Page::Listing(format!("{} · canciones", a.name), m.clone()))));
            col = col.push(self.track_list(&a.top, None));
        }
        for sec in &a.sections {
            // Las playlists de YouTube Music aún no se abren aquí.
            if sec.items.iter().all(|i| i.kind == "playlist") {
                continue;
            }
            col = col.push(Space::new().height(6));
            col = col.push(section_title(&sec.title, sec.more.as_ref().map(|m| Page::Listing(format!("{} · {}", a.name, sec.title), m.clone()))));
            col = col.push(self.item_cards(&sec.items, &a.name));
        }
        if let Some(d) = &a.description {
            col = col.push(Space::new().height(6));
            col = col.push(text("Acerca de").size(20));
            col = col.push(text(short(d, 600)).size(13).color(faint()));
        }
        col.into()
    }

    /// Álbumes, singles, videos y artistas parecidos, como tarjetas.
    fn item_cards<'a>(&'a self, items: &'a [Item], artist: &'a str) -> Element<'a, Msg> {
        carousel(self.item_row(items, artist))
    }

    /// Las mismas tarjetas en rejilla, para la página de "Ver todo".
    fn item_grid<'a>(&'a self, items: &'a [Item], artist: &'a str) -> Element<'a, Msg> {
        self.item_row(items, artist).wrap().vertical_spacing(14).into()
    }

    fn item_row<'a>(&'a self, items: &'a [Item], artist: &'a str) -> iced::widget::Row<'a, Msg> {
        let mut r = row![].spacing(14);
        for it in items.iter().filter(|i| i.kind != "playlist") {
            let round = it.kind == "artist";
            let pic: Element<Msg> = match self.thumbs.get(&it.id) {
                Some(Thumb::Ready(_)) if it.kind == "video" => self.cover_loading(&it.id, 150.0),
                Some(Thumb::Ready(h)) => image(h.clone()).width(150).height(150).content_fit(ContentFit::Cover).border_radius(if round { 75.0 } else { 15.0 }).into(),
                _ => self.cover(&it.id, 150.0),
            };
            let msg = match it.kind.as_str() {
                "album" => Msg::Nav(Page::Album(it.id.clone())),
                "artist" => Msg::Nav(Page::Artist(Some(it.id.clone()), it.title.clone())),
                _ => Msg::PlayList(
                    vec![Track { id: it.id.clone(), title: it.title.clone(), artist: Some(artist.to_string()), album: None, duration: None, channel: None, channel_id: None }],
                    0,
                ),
            };
            let card = column![pic, text(short(&it.title, 18)).size(13), text(short(&it.subtitle, 22)).size(11).color(faint())].spacing(6).width(150);
            let card = flat(card, false).padding(8).on_press(msg.clone());
            r = r.push(match msg {
                Msg::PlayList(ts, _) => hoverable(card, &ts[0]),
                _ => card.into(),
            });
        }
        r
    }

    fn album_page<'a>(&'a self, a: &'a Album) -> Element<'a, Msg> {
        let art: Element<Msg> = match self.thumbs.get(&a.id) {
            Some(Thumb::Ready(h)) => image(h.clone()).width(180).height(180).content_fit(ContentFit::Cover).border_radius(12.0).into(),
            _ => self.cover("", 180.0),
        };
        let artist = button(text(&a.artist).size(14))
            .padding(0)
            .style(|_, status| button::Style {
                text_color: if matches!(status, button::Status::Hovered) { accent() } else { fg() },
                ..Default::default()
            })
            .on_press(Msg::Nav(Page::Artist(a.artist_id.clone(), a.artist.clone())));
        let mut info = column![text(a.subtitle.to_uppercase()).size(11).color(faint()), text(&a.title).size(36), artist].spacing(4);
        if !a.tracks.is_empty() {
            info = info.push(Space::new().height(6));
            info = info.push(play_button(Msg::PlayList(a.tracks.clone(), 0)));
        }
        column![row![art, info].spacing(24).align_y(Alignment::End), Space::new().height(8), self.track_rows(&a.tracks, None, false)].spacing(8).into()
    }

    /// Fila de tarjetas con portada grande; al tocar una suena esa lista desde ahí.
    fn cards(&self, tracks: &[Track]) -> Element<'_, Msg> {
        let mut r = row![].spacing(14);
        for (i, t) in tracks.iter().enumerate() {
            let card = column![
                self.cover_loading(&t.id, 150.0),
                text(short(&clean(t), 18)).size(13),
                who_link(t, 11),
            ]
            .spacing(6)
            .width(150);
            r = r.push(hoverable(flat(card, false).padding(8).on_press(Msg::PlayList(tracks.to_vec(), i)), t));
        }
        carousel(r)
    }

    fn search(&self) -> Element<'_, Msg> {
        let input = text_input("¿Qué quieres escuchar?", &self.query)
            .id(SEARCH_INPUT)
            .on_input(Msg::Query)
            .on_submit(Msg::Search)
            .size(16)
            .padding(12);
        let mut col = column![input].spacing(16);
        if self.searching {
            col = col.push(self.waiting_line("buscando…".into()));
        } else if !self.results.is_empty() {
            col = col.push(self.track_list(&self.results, None));
        }
        col.into()
    }

    fn list_page<'a>(&'a self, title: &'a str, subtitle: &str, tracks: &'a [Track], playlist: Option<&'a str>) -> Element<'a, Msg> {
        let art: Element<Msg> = match tracks.first() {
            Some(t) => self.cover(&t.id, 140.0),
            None => self.cover("", 140.0),
        };
        let mut actions = row![].spacing(8).align_y(Alignment::Center);
        if !tracks.is_empty() {
            actions = actions.push(play_button(Msg::PlayList(tracks.to_vec(), 0)));
        }
        if let Some(id) = playlist {
            actions = actions.push(flat(text("Borrar playlist").size(13).color(faint()), false).on_press(Msg::DeletePlaylist(id.to_string())));
        }
        let header = row![
            art,
            column![text(if playlist.is_some() { "PLAYLIST" } else { "LISTA" }).size(11).color(faint()), text(title).size(30), text(subtitle.to_string()).size(13).color(faint()), Space::new().height(6), actions]
                .spacing(4)
        ]
        .spacing(18)
        .align_y(Alignment::End);
        let body: Element<Msg> = if tracks.is_empty() {
            text(if playlist.is_some() { "Vacía. Añade canciones con el + de cualquier fila." } else { "Marca canciones con ♥ y aparecen aquí." }).color(faint()).into()
        } else {
            self.track_list(tracks, playlist)
        };
        column![header, Space::new().height(10), body].spacing(8).into()
    }

    /// La lista de canciones: número, portada, título y artista, duración, ♥ y +.
    fn track_list<'a>(&'a self, tracks: &'a [Track], playlist: Option<&'a str>) -> Element<'a, Msg> {
        self.track_rows(tracks, playlist, true)
    }

    /// `covers`: sin portada por fila en un álbum, donde todas serían la misma.
    fn track_rows<'a>(&'a self, tracks: &'a [Track], playlist: Option<&'a str>, covers: bool) -> Element<'a, Msg> {
        let playing = self.current().map(|t| t.id);
        let loading = self.loading_id();
        let mut col = Column::new().spacing(2);
        for (i, t) in tracks.iter().enumerate() {
            let now = playing.as_deref() == Some(&t.id);
            let fav = self.is_favorite(&t.id);
            let title_color = if now { accent() } else { fg() };
            let mut tail = row![
                text(t.duration.map(clock).unwrap_or_default()).size(12).color(faint()).width(48),
                icon(if fav { "♥" } else { "♡" }, 15, if fav { accent() } else { faint() }).on_press(Msg::Favorite(t.clone(), !fav)),
                icon("+", 16, faint()).on_press(Msg::Menu(if self.menu.as_deref() == Some(&t.id) { None } else { Some(t.id.clone()) })),
            ]
            .spacing(4)
            .align_y(Alignment::Center);
            if let Some(pid) = playlist {
                tail = tail.push(icon("×", 15, faint()).on_press(Msg::RemoveFrom(pid.to_string(), t.id.clone())));
            }
            let mark: Element<Msg> = if loading.as_deref() == Some(&t.id) {
                container(cargando::spinner(self.anim(), 16.0, accent())).width(28).into()
            } else {
                text(if now { "♪".to_string() } else { (i + 1).to_string() }).size(13).color(if now { accent() } else { faint() }).width(28).into()
            };
            let line = row![
                mark,
                if covers { self.cover(&t.id, 40.0) } else { Space::new().into() },
                column![text(short(&clean(t), 70)).size(14).color(title_color), who_link(t, 12)].width(Fill),
                tail,
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            col = col.push(hoverable(flat(line, now).padding([4, 8]).width(Fill).on_press(Msg::PlayList(tracks.to_vec(), i)), t));
            if self.menu.as_deref() == Some(&t.id) {
                col = col.push(self.add_menu(t));
            }
        }
        col.into()
    }

    /// Debajo de la fila: a qué playlist añadirla, o a la cola.
    fn add_menu(&self, t: &Track) -> Element<'_, Msg> {
        let mut r = row![text("añadir a:").size(12).color(faint())].spacing(6).align_y(Alignment::Center);
        r = r.push(flat(text("la cola").size(12), false).on_press(Msg::Enqueue(t.clone())));
        for p in &self.library.playlists {
            r = r.push(flat(text(&p.name).size(12), false).on_press(Msg::AddTo(p.id.clone(), t.clone())));
        }
        if self.library.playlists.is_empty() {
            r = r.push(text("(crea una playlist en la barra lateral)").size(12).color(faint()));
        }
        container(r.wrap()).padding(Padding { left: 48.0, top: 2.0, bottom: 8.0, right: 0.0 }).into()
    }

    fn player(&self) -> Element<'_, Msg> {
        let cur = self.current();
        let paused = self.playback("paused").and_then(Value::as_bool).unwrap_or(true);
        let pos = self.playback("position").and_then(Value::as_f64).unwrap_or(0.0);
        let dur = self.playback("duration").and_then(Value::as_f64).or(cur.as_ref().and_then(|t| t.duration)).unwrap_or(0.0);
        let vol = self.status.as_ref().and_then(|s| s.get("volume")).and_then(Value::as_f64).unwrap_or(70.0);

        // Lo pedido se ve abajo al instante, aunque el daemon tarde en confirmarlo.
        let asked = self.pending.as_ref().and_then(|p| p.track.clone());
        let shown_track = asked.clone().or(cur.clone());
        let left: Element<Msg> = match &shown_track {
            Some(t) if asked.is_some() => row![
                self.cover_loading(&t.id, 52.0),
                column![text(short(&clean(t), 30)).size(14), text("cargando…").size(12).color(faint())].spacing(2),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
            Some(t) => {
                let fav = self.is_favorite(&t.id);
                let art = mouse_area(self.cover_loading(&t.id, 52.0)).on_press(Msg::Full(!self.full)).interaction(iced::mouse::Interaction::Pointer);
                let name = mouse_area(text(short(&clean(t), 30)).size(14)).on_press(Msg::Full(!self.full)).interaction(iced::mouse::Interaction::Pointer);
                let info = row![art, column![name, who_link(t, 12)].spacing(2)].spacing(12).align_y(Alignment::Center);
                row![info, icon(if fav { "♥" } else { "♡" }, 16, if fav { accent() } else { faint() }).on_press(Msg::Favorite(t.clone(), !fav))]
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .into()
            }
            None => text(if self.status.is_some() { "nada sonando" } else { "surco no responde" }).color(faint()).into(),
        };

        let shown = self.seeking.unwrap_or(pos as f32);
        let controls = column![
            row![
                icon("⏮", 18, fg()).on_press(Msg::Control("prev")),
                if self.busy() {
                    // Mismo hueco que el botón, para que nada salte.
                    container(cargando::spinner(self.anim(), 20.0, accent())).padding([4, 8]).into()
                } else {
                    Element::from(icon(if paused { "▶" } else { "⏸" }, 22, accent()).on_press(Msg::Control("toggle")))
                },
                icon("⏭", 18, fg()).on_press(Msg::Control("next")),
            ]
            .spacing(14)
            .align_y(Alignment::Center),
            row![
                text(clock(shown as f64)).size(11).color(faint()),
                slider(0.0..=dur.max(1.0) as f32, shown, Msg::Seek).on_release(Msg::SeekDone).width(Fill),
                text(clock(dur)).size(11).color(faint()),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        ]
        .spacing(4)
        .align_x(Alignment::Center)
        .width(Length::FillPortion(2));

        let right = row![
            text("🔊").size(14).color(faint()),
            slider(0.0..=100.0, self.volume.unwrap_or(vol as f32), Msg::Volume).on_release(Msg::VolumeDone).width(110),
            icon(if self.full { "⌄" } else { "⌃" }, 16, faint()).on_press(Msg::Full(!self.full)),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        panel(
            row![
                container(left).width(Length::FillPortion(1)),
                controls,
                container(right).width(Length::FillPortion(1)).align_x(Alignment::End),
            ]
            .spacing(16)
            .align_y(Alignment::Center)
            .padding([10, 16]),
        )
        .height(84)
        .into()
    }

    /// El núcleo de Iris a pantalla completa, con el título y la letra encima.
    fn full_view(&self) -> Element<'_, Msg> {
        // El núcleo ocupa la izquierda (3/5) y la letra la derecha: encima se tapaban.
        let (w, h) = ((self.size.width - 16.0) * 0.6, (self.size.height - 84.0 - 24.0).max(50.0));
        let dots = self
            .core
            .dots((w / SPACING) as usize, (h / SPACING) as usize)
            .into_iter()
            .map(|(pos, color)| puntos::Dot { pos, color })
            .collect();
        let nucleo = shader(puntos::Puntos { dots, spacing: SPACING }).width(Fill).height(Fill);

        let cur = self.current();
        let pos = self.playback("position").and_then(Value::as_f64).unwrap_or(0.0);
        let mut lyr = Column::new().spacing(12).width(Fill);
        match &self.lyrics {
            Some(l) if !l.lines.is_empty() => {
                // La línea que suena y unas cuantas alrededor; las vacías (pausas) no ocupan sitio.
                let t = pos + l.offset;
                let lines: Vec<_> = l.lines.iter().filter(|x| !x.text.trim().is_empty()).collect();
                let at = lines.iter().rposition(|x| x.at <= t).unwrap_or(0);
                for (i, line) in lines.iter().enumerate().skip(at.saturating_sub(3)).take(10) {
                    let c = if i == at { accent() } else { faint().scale_alpha(if i < at { 0.45 } else { 0.85 }) };
                    lyr = lyr.push(text(line.text.clone()).size(if i == at { 26 } else { 19 }).color(c));
                }
            }
            Some(l) if l.plain.is_some() => lyr = lyr.push(text(l.plain.clone().unwrap_or_default()).size(15).color(faint())),
            _ => lyr = lyr.push(text("sin letra").size(14).color(faint())),
        }
        let info = column![
            text(cur.as_ref().map(clean).unwrap_or_else(|| "nada sonando".into())).size(30),
            cur.as_ref().map(|t| who_link(t, 16)).unwrap_or_else(|| Space::new().into()),
        ]
        .spacing(6);
        let left = stack![nucleo, container(info).width(Fill).height(Fill).align_y(Alignment::End).padding(28)];
        panel(
            row![
                container(left).width(Length::FillPortion(3)).height(Fill),
                container(lyr).width(Length::FillPortion(2)).height(Fill).align_y(Alignment::Center).padding(28),
            ],
        )
        .width(Fill)
        .height(Fill)
        .into()
    }
}

/// El título sin el artista delante ni las coletillas de YouTube: "Daft Punk - One More
/// Time (Official Video)" → "One More Time".
pub fn clean(t: &Track) -> String {
    let mut s = t.title.trim().to_string();
    let who = t.who();
    if !who.is_empty() {
        for sep in [" - ", " – ", " — ", ": "] {
            let prefix = format!("{who}{sep}");
            if s.to_lowercase().starts_with(&prefix.to_lowercase()) {
                s = s[prefix.len()..].trim().to_string();
                break;
            }
        }
    }
    const NOISE: [&str; 9] = ["official", "video", "audio", "lyric", "letra", "hd", "4k", "remaster", "visualizer"];
    loop {
        let trimmed = s.trim_end();
        let Some(open) = trimmed.rfind(['(', '[']) else { break };
        let inner = trimmed[open..].to_lowercase();
        if !(trimmed.ends_with(')') || trimmed.ends_with(']')) || !NOISE.iter().any(|w| inner.contains(w)) {
            break;
        }
        s = trimmed[..open].trim_end().to_string();
    }
    if s.is_empty() { t.title.clone() } else { s }
}

/// Una fila de tarjetas que se desliza en horizontal (touchpad, o Shift + rueda); la
/// rueda sola sigue bajando la página.
fn carousel<'a>(r: iced::widget::Row<'a, Msg>) -> Element<'a, Msg> {
    scrollable(container(r).padding(Padding { bottom: 12.0, ..Padding::ZERO }))
        .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::new().width(4).scroller_width(4)))
        .width(Fill)
        .into()
}

/// Título de sección con su "Ver todo" a la derecha, si lleva a algún lado.
fn section_title<'a>(title: &str, more: Option<Page>) -> Element<'a, Msg> {
    let mut r = row![text(title.to_string()).size(20).width(Fill)].align_y(Alignment::Center);
    if let Some(page) = more {
        r = r.push(flat(text("Ver todo").size(12).color(faint()), false).on_press(Msg::Nav(page)));
    }
    r.into()
}

fn play_button<'a>(msg: Msg) -> Element<'a, Msg> {
    // Al apuntarle ya se adelanta la primera, que es la que va a sonar.
    let first = match &msg {
        Msg::PlayList(ts, i) => ts.get(*i).cloned(),
        _ => None,
    };
    let b = button(text("▶  Reproducir").size(14))
        .padding([8, 18])
        .style(|_, _| button::Style {
            background: Some(accent().into()),
            text_color: rgb(theme::bg_rgb(), 1.0),
            border: Border { radius: 20.0.into(), ..Default::default() },
            ..Default::default()
        })
        .on_press(msg);
    match first {
        Some(t) => hoverable(b, &t),
        None => b.into(),
    }
}

fn greeting() -> &'static str {
    use chrono::Timelike;
    match chrono::Local::now().hour() {
        5..=11 => "Buenos días",
        12..=18 => "Buenas tardes",
        _ => "Buenas noches",
    }
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n - 1).collect::<String>()) }
}
