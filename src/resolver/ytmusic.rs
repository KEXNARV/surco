//! Páginas de artista y de álbum desde YouTube Music.
//!
//! yt-dlp no tiene "artista": solo canales y videos. La API interna de YouTube Music
//! (la que usa music.youtube.com, cliente WEB_REMIX, sin sesión) sí, y en ~1 s por
//! página trae canciones más escuchadas, álbumes, singles, artistas parecidos y foto.
//! Es interna y puede cambiar sin aviso: todo se lee a la defensiva y lo que no se
//! entiende se salta en vez de fallar.

use super::Track;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

const API: &str = "https://music.youtube.com/youtubei/v1";
/// La de la web a finales de 2026. Si YouTube deja de aceptarla, sube este número.
const CLIENT_VERSION: &str = "1.20260930.01.00";
/// Filtro "Artistas" de la búsqueda de YouTube Music.
const ARTISTS_FILTER: &str = "EgWKAQIgAWoKEAkQAxAEEAoQBQ%3D%3D";

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    /// "album", "artist", "video" o "playlist".
    pub kind: String,
    /// browseId para álbumes, artistas y playlists; videoId para videos.
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub thumb: Option<String>,
}

/// A dónde lleva el "ver todo" de una sección: la lista entera, no el adelanto.
#[derive(Debug, Clone, Serialize)]
pub struct More {
    pub id: String,
    pub params: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Section {
    pub title: String,
    pub items: Vec<Item>,
    pub more: Option<More>,
}

/// Una lista entera: o canciones (una playlist, "todas las populares") o tarjetas
/// (la discografía).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Listing {
    pub title: String,
    pub items: Vec<Item>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub image: Option<String>,
    pub listeners: Option<String>,
    pub top: Vec<Track>,
    pub top_more: Option<More>,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub artist_id: Option<String>,
    pub subtitle: String,
    pub thumb: Option<String>,
    pub tracks: Vec<Track>,
}

pub struct YtMusic {
    http: reqwest::Client,
}

/// Texto de un objeto `{runs: [{text}]}` o `{simpleText}`.
fn runs(v: &Value) -> String {
    if let Some(s) = v.get("simpleText").and_then(Value::as_str) {
        return s.to_string();
    }
    v.get("runs")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(|x| x.get("text").and_then(Value::as_str)).collect())
        .unwrap_or_default()
}

/// La imagen más grande de un `thumbnails`, pedida a un tamaño útil: las URLs de
/// googleusercontent llevan el tamaño al final (`=w60-h60-...`).
fn thumb(v: &Value) -> Option<String> {
    let list = v.pointer("/musicThumbnailRenderer/thumbnail/thumbnails").or_else(|| v.pointer("/thumbnails"))?.as_array()?;
    let url = list.last()?.get("url")?.as_str()?;
    Some(match url.find("=w") {
        Some(i) if url.contains("googleusercontent") => format!("{}=w544-h544-l90-rj", &url[..i]),
        _ => url.to_string(),
    })
}

fn flex(item: &Value, i: usize) -> &Value {
    item.pointer(&format!("/flexColumns/{i}/musicResponsiveListItemFlexColumnRenderer/text")).unwrap_or(&Value::Null)
}

/// "4:11" o "1:02:03" a segundos.
fn duration(s: &str) -> Option<f64> {
    s.split(':').try_fold(0.0, |acc, p| p.trim().parse::<f64>().ok().map(|n| acc * 60.0 + n))
}

/// Una fila de canción (`musicResponsiveListItemRenderer`). `artist` cuando la fila no
/// lo trae, como en las de un álbum.
fn track_row(r: &Value, artist: Option<&str>, album: Option<&str>) -> Option<Track> {
    let id = r.pointer("/playlistItemData/videoId")?.as_str()?.to_string();
    let title = runs(flex(r, 0));
    let col1 = runs(flex(r, 1));
    let dur = r
        .pointer("/fixedColumns/0/musicResponsiveListItemFixedColumnRenderer/text")
        .map(runs)
        .and_then(|s| duration(&s));
    let who = if col1.is_empty() || col1.contains("reproducciones") || col1.contains("plays") { artist.map(str::to_string) } else { Some(col1) };
    Some(Track {
        id,
        title,
        artist: who,
        album: album.map(str::to_string).or_else(|| Some(runs(flex(r, 3))).filter(|s| !s.is_empty())),
        duration: dur,
        channel: None,
        channel_id: None,
    })
}

/// Una fila de la radio (`playlistPanelVideoRenderer`). El byline es "Artista y Otro •
/// Álbum • 2017"; el canal es el del primer artista.
fn radio_row(r: &Value) -> Option<Track> {
    let id = r.get("videoId")?.as_str()?.to_string();
    let byline = r.get("longBylineText").map(runs).unwrap_or_default();
    let mut parts = byline.split(" • ");
    let artist = parts.next().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let album = parts.next().map(str::trim).filter(|s| !s.is_empty() && s.parse::<u32>().is_err()).map(str::to_string);
    let channel_id = r
        .pointer("/longBylineText/runs")
        .and_then(Value::as_array)
        .and_then(|rs| rs.iter().find_map(|x| x.pointer("/navigationEndpoint/browseEndpoint/browseId")?.as_str().filter(|b| b.starts_with("UC"))))
        .map(str::to_string);
    Some(Track {
        id,
        title: r.get("title").map(runs).unwrap_or_default(),
        artist,
        album,
        duration: r.get("lengthText").map(runs).and_then(|s| duration(&s)),
        channel: None,
        channel_id,
    })
}

fn two_row(r: &Value) -> Option<Item> {
    let title = runs(r.get("title")?);
    let subtitle = r.get("subtitle").map(runs).unwrap_or_default();
    let thumb = r.get("thumbnailRenderer").and_then(thumb);
    let nav = r.get("navigationEndpoint")?;
    if let Some(v) = nav.pointer("/watchEndpoint/videoId").and_then(Value::as_str) {
        return Some(Item { kind: "video".into(), id: v.into(), title, subtitle, thumb });
    }
    let id = nav.pointer("/browseEndpoint/browseId")?.as_str()?.to_string();
    let kind = if id.starts_with("MPRE") {
        "album"
    } else if id.starts_with("UC") {
        "artist"
    } else {
        "playlist"
    };
    Some(Item { kind: kind.into(), id, title, subtitle, thumb })
}

impl YtMusic {
    pub fn new() -> Result<Self> {
        // Holgado: con la red lenta una página de ~400 KB tarda más de 15 s.
        Ok(Self { http: reqwest::Client::builder().timeout(std::time::Duration::from_secs(45)).build()? })
    }

    async fn call(&self, endpoint: &str, body: Value) -> Result<Value> {
        let mut body = body;
        body["context"] = json!({ "client": { "clientName": "WEB_REMIX", "clientVersion": CLIENT_VERSION, "hl": "es" } });
        let res = self
            .http
            .post(format!("{API}/{endpoint}?prettyPrint=false"))
            .header("Origin", "https://music.youtube.com")
            .json(&body)
            .send()
            .await
            .context("YouTube Music no responde")?;
        if !res.status().is_success() {
            bail!("YouTube Music respondió {}", res.status());
        }
        Ok(res.json().await?)
    }

    /// La radio de YouTube Music de una canción (`RDAMVM<id>`): ~50 parecidas, la propia
    /// primero. Es lo que suena en music.youtube.com al darle a "Iniciar radio".
    pub async fn radio(&self, video_id: &str) -> Result<Vec<Track>> {
        let v = self
            .call("next", json!({ "videoId": video_id, "playlistId": format!("RDAMVM{video_id}"), "isAudioOnly": true }))
            .await?;
        let mut out = vec![];
        walk(&v, &mut |o| {
            if let Some(t) = o.get("playlistPanelVideoRenderer").and_then(radio_row) {
                out.push(t);
            }
        });
        Ok(out)
    }

    /// El canal de un artista a partir de su nombre: el primero de la búsqueda de artistas.
    pub async fn find_artist(&self, name: &str) -> Result<String> {
        let v = self.call("search", json!({ "query": name, "params": urlencoding_decode(ARTISTS_FILTER) })).await?;
        let mut found = None;
        walk(&v, &mut |o| {
            if found.is_none() {
                if let Some(id) = o.pointer("/browseEndpoint/browseId").and_then(Value::as_str) {
                    if id.starts_with("UC") {
                        found = Some(id.to_string());
                    }
                }
            }
        });
        found.with_context(|| format!("no encontré al artista «{name}»"))
    }

    /// La página de un artista. `None` si ese canal no es de un artista (un canal de
    /// subidas cualquiera, como "7clouds").
    pub async fn artist(&self, channel_id: &str) -> Result<Option<Artist>> {
        let v = self.call("browse", json!({ "browseId": channel_id })).await?;
        let Some(h) = v.pointer("/header/musicImmersiveHeaderRenderer").or_else(|| v.pointer("/header/musicVisualHeaderRenderer")) else {
            return Ok(None);
        };
        let name = runs(h.get("title").unwrap_or(&Value::Null));
        let mut artist = Artist {
            id: channel_id.to_string(),
            name: name.clone(),
            description: h.get("description").map(runs).filter(|s| !s.is_empty()),
            image: h.get("thumbnail").and_then(thumb).or_else(|| h.get("foregroundThumbnail").and_then(thumb)),
            listeners: h.get("monthlyListenerCount").map(runs).filter(|s| !s.is_empty()),
            top: vec![],
            top_more: None,
            sections: vec![],
        };
        let secs = v
            .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for s in &secs {
            if let Some(shelf) = s.get("musicShelfRenderer") {
                let rows = shelf.get("contents").and_then(Value::as_array).cloned().unwrap_or_default();
                artist.top = rows
                    .iter()
                    .filter_map(|r| track_row(r.get("musicResponsiveListItemRenderer")?, Some(&name), None))
                    .map(|t| Track { channel_id: Some(channel_id.to_string()), ..t })
                    .collect();
                artist.top_more = shelf.pointer("/title/runs/0/navigationEndpoint/browseEndpoint").and_then(more);
            } else if let Some(c) = s.get("musicCarouselShelfRenderer") {
                let title = c.pointer("/header/musicCarouselShelfBasicHeaderRenderer/title").map(runs).unwrap_or_default();
                let items: Vec<Item> = c
                    .get("contents")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|x| two_row(x.get("musicTwoRowItemRenderer")?)).collect())
                    .unwrap_or_default();
                let h = c.pointer("/header/musicCarouselShelfBasicHeaderRenderer");
                let link = h
                    .and_then(|h| h.pointer("/moreContentButton/buttonRenderer/navigationEndpoint/browseEndpoint"))
                    .or_else(|| h.and_then(|h| h.pointer("/title/runs/0/navigationEndpoint/browseEndpoint")))
                    .and_then(more);
                if !items.is_empty() {
                    artist.sections.push(Section { title, items, more: link });
                }
            } else if let Some(d) = s.get("musicDescriptionShelfRenderer") {
                if artist.description.is_none() {
                    artist.description = d.get("description").map(runs);
                }
            }
        }
        Ok(Some(artist))
    }

    /// El "ver todo": una discografía (rejilla de álbumes) o una playlist de canciones,
    /// esta última pidiendo páginas de 100 hasta `MAX_TRACKS`.
    pub async fn listing(&self, id: &str, params: Option<&str>) -> Result<Listing> {
        const MAX_TRACKS: usize = 500;
        let mut body = json!({ "browseId": id });
        if let Some(p) = params {
            body["params"] = json!(urlencoding_decode(p));
        }
        let v = self.call("browse", body).await?;
        let mut out = Listing::default();
        out.title = v
            .pointer("/header/musicHeaderRenderer/title")
            .or_else(|| v.pointer("/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer/title"))
            .map(runs)
            .unwrap_or_default();
        let mut cont = None;
        walk(&v, &mut |o| {
            if let Some(g) = o.get("gridRenderer") {
                if let Some(items) = g.get("items").and_then(Value::as_array) {
                    out.items.extend(items.iter().filter_map(|x| two_row(x.get("musicTwoRowItemRenderer")?)));
                }
            }
            if let Some(shelf) = o.get("musicPlaylistShelfRenderer") {
                take_rows(shelf.get("contents"), &mut out.tracks, &mut cont);
            }
        });
        while let Some(token) = cont.take() {
            if out.tracks.len() >= MAX_TRACKS {
                break;
            }
            let v = self.call("browse", json!({ "continuation": token })).await?;
            let items = v.pointer("/onResponseReceivedActions/0/appendContinuationItemsAction/continuationItems");
            take_rows(items, &mut out.tracks, &mut cont);
        }
        Ok(out)
    }

    pub async fn album(&self, browse_id: &str) -> Result<Album> {
        let v = self.call("browse", json!({ "browseId": browse_id })).await?;
        let two = v.pointer("/contents/twoColumnBrowseResultsRenderer").context("YouTube Music devolvió un álbum sin el formato esperado")?;
        let h = two
            .pointer("/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer")
            .context("álbum sin cabecera")?;
        let title = runs(h.get("title").unwrap_or(&Value::Null));
        let strap = h.get("straplineTextOne").unwrap_or(&Value::Null);
        let artist = runs(strap);
        let artist_id = strap.pointer("/runs/0/navigationEndpoint/browseEndpoint/browseId").and_then(Value::as_str).map(str::to_string);
        let rows = two
            .pointer("/secondaryContents/sectionListRenderer/contents/0/musicShelfRenderer/contents")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let tracks = rows
            .iter()
            .filter_map(|r| track_row(r.get("musicResponsiveListItemRenderer")?, Some(&artist), Some(&title)))
            .map(|t| Track { channel_id: artist_id.clone(), ..t })
            .collect();
        Ok(Album {
            id: browse_id.to_string(),
            title,
            artist,
            artist_id,
            subtitle: h.get("subtitle").map(runs).unwrap_or_default(),
            thumb: h.get("thumbnail").and_then(thumb),
            tracks,
        })
    }
}

fn more(be: &Value) -> Option<More> {
    Some(More { id: be.get("browseId")?.as_str()?.to_string(), params: be.get("params").and_then(Value::as_str).map(str::to_string) })
}

/// Las filas de canción de una página de playlist, y el token de la siguiente si la hay.
fn take_rows(rows: Option<&Value>, out: &mut Vec<Track>, cont: &mut Option<String>) {
    for r in rows.and_then(Value::as_array).into_iter().flatten() {
        if let Some(t) = r.get("musicResponsiveListItemRenderer").and_then(|x| track_row(x, None, None)) {
            out.push(t);
        } else if let Some(tok) = r.pointer("/continuationItemRenderer/continuationEndpoint/continuationCommand/token").and_then(Value::as_str) {
            *cont = Some(tok.to_string());
        }
    }
}

fn walk(v: &Value, f: &mut impl FnMut(&Value)) {
    f(v);
    match v {
        Value::Object(m) => m.values().for_each(|x| walk(x, f)),
        Value::Array(a) => a.iter().for_each(|x| walk(x, f)),
        _ => {}
    }
}

/// Los `params` van en el JSON tal cual, sin el escape de URL con que se copian.
fn urlencoding_decode(s: &str) -> String {
    s.replace("%3D", "=")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duraciones() {
        assert_eq!(duration("4:11"), Some(251.0));
        assert_eq!(duration("1:02:03"), Some(3723.0));
        assert_eq!(duration("x"), None);
    }

    #[tokio::test]
    #[ignore] // red
    async fn pagina_real() {
        let y = YtMusic::new().unwrap();
        let id = y.find_artist("gustavo cerati").await.unwrap();
        let a = y.artist(&id).await.unwrap().unwrap();
        println!("{} {:?} top={} secciones={:?}", a.name, a.listeners, a.top.len(), a.sections.iter().map(|s| (&s.title, s.items.len())).collect::<Vec<_>>());
        let alb = a.sections.iter().flat_map(|s| &s.items).find(|i| i.kind == "album").unwrap();
        let al = y.album(&alb.id).await.unwrap();
        println!("{} — {} {} pistas: {:?}", al.title, al.artist, al.tracks.len(), al.tracks.first());
        assert!(!a.top.is_empty() && !al.tracks.is_empty());
    }

    #[tokio::test]
    #[ignore] // red
    async fn radio_real() {
        let r = YtMusic::new().unwrap().radio("r7zTKRonHXM").await.unwrap();
        println!("{} pistas; {:?}", r.len(), r.get(1));
        assert!(r.len() > 20 && r[0].id == "r7zTKRonHXM" && r[1].artist.is_some() && r[1].duration.is_some());
    }
}
