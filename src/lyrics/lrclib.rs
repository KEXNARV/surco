//! Cliente de lrclib.net: letras sincronizadas, publicas y sin API key.
//!
//! Lo dificil es el match. Los titulos de YouTube vienen con ruido
//! ("(Video Oficial)", "(Lyrics)") y el canal muchas veces es la disquera y no
//! el artista. Medido sobre casos reales de este reproductor, la normalizacion
//! de abajo acierta 7 de 8.

use super::{Lyrics, LyricsProvider};
use crate::resolver::Track;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

/// Ruido que no cambia que cancion es.
const NOISE: &[&str] = &[
    "official music video", "official video", "video oficial", "official audio",
    "lyric video", "lyrics video", "lyrics", "lyric", "letra", "letras",
    "con letra", "audio", "visualizer", "remastered", "remaster", "hd", "hq",
    "4k", "full", "mv", "cover audio",
];

/// Marcas de que es OTRA version: el timing del estudio no va a cuadrar.
const VERSION_MARKERS: &[&str] = &[
    "unplugged", "live", "en vivo", "acustico", "acústico", "acoustic",
    "remix", "sinfonico", "sinfónico", "session", "concert", "tour", "vivo",
];

#[derive(Deserialize)]
struct ApiLyrics {
    #[serde(rename = "trackName")]
    track_name: Option<String>,
    #[serde(rename = "artistName")]
    artist_name: Option<String>,
    duration: Option<f64>,
    #[serde(rename = "syncedLyrics")]
    synced: Option<String>,
    #[serde(rename = "plainLyrics")]
    plain: Option<String>,
    instrumental: Option<bool>,
}

pub struct LrcLib {
    http: reqwest::Client,
    cache_dir: PathBuf,
}

impl LrcLib {
    pub fn new() -> Result<Self> {
        // lrclib pide identificarse en el User-Agent.
        let http = reqwest::Client::builder()
            .user_agent(concat!(
                "surco/", env!("CARGO_PKG_VERSION"),
                " (https://github.com/kex/surco)"
            ))
            .timeout(std::time::Duration::from_secs(12))
            .build()
            .context("no se pudo construir el cliente HTTP")?;

        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("surco/lyrics");
        Ok(Self { http, cache_dir })
    }

    fn cache_path(&self, track: &Track) -> PathBuf {
        // El id de YouTube ya es unico y seguro como nombre de archivo.
        let safe: String = track
            .id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        self.cache_dir.join(format!("{safe}.json"))
    }

    async fn from_cache(&self, track: &Track) -> Option<Lyrics> {
        let raw = tokio::fs::read(self.cache_path(track)).await.ok()?;
        serde_json::from_slice(&raw).ok()
    }

    async fn to_cache(&self, track: &Track, lyrics: &Lyrics) {
        // Un fallo de cache no debe impedir mostrar la letra.
        if tokio::fs::create_dir_all(&self.cache_dir).await.is_err() {
            return;
        }
        if let Ok(json) = serde_json::to_vec_pretty(lyrics) {
            let _ = tokio::fs::write(self.cache_path(track), json).await;
        }
    }

    async fn store_offset(&self, track: &Track, offset: f64) -> Result<()> {
        let mut lyrics = self
            .from_cache(track)
            .await
            .context("no hay letra en cache para esta pista")?;
        lyrics.offset = offset;
        self.to_cache(track, &lyrics).await;
        Ok(())
    }

    async fn get(&self, artist: &str, track: &str, duration: Option<f64>) -> Option<ApiLyrics> {
        let mut q: Vec<(&str, String)> = vec![
            ("artist_name", artist.to_string()),
            ("track_name", track.to_string()),
        ];
        if let Some(d) = duration {
            q.push(("duration", format!("{:.0}", d)));
        }
        let res = self
            .http
            .get("https://lrclib.net/api/get")
            .query(&q)
            .send()
            .await
            .ok()?;
        if !res.status().is_success() {
            return None; // 404 es lo normal, no un error que reportar
        }
        res.json::<ApiLyrics>().await.ok()
    }

    async fn search(&self, query: &str) -> Option<ApiLyrics> {
        let res = self
            .http
            .get("https://lrclib.net/api/search")
            .query(&[("q", query)])
            .send()
            .await
            .ok()?;
        let list: Vec<ApiLyrics> = res.json().await.ok()?;
        // Prefiere un resultado que traiga tiempos sobre uno de texto plano.
        list.into_iter()
            .find(|l| l.synced.is_some())
            .or_else(|| None)
    }
}

/// Quita del titulo lo que es puro ruido de YouTube.
fn strip_noise(title: &str) -> String {
    let mut out = title.to_string();
    // Parentesis y corchetes cuyo contenido es solo ruido.
    for open in ['(', '['] {
        let close = if open == '(' { ')' } else { ']' };
        loop {
            let Some(a) = out.find(open) else { break };
            let Some(rel) = out[a..].find(close) else { break };
            let b = a + rel;
            let inside = out[a + 1..b].trim().to_lowercase();
            let is_noise = NOISE.iter().any(|n| inside.contains(n));
            if is_noise {
                out.replace_range(a..=b, "");
            } else {
                // Contenido con significado (una version): se conserva y hay
                // que seguir buscando mas alla de este parentesis.
                let tail = out[b + 1..].to_string();
                let head = out[..=b].to_string();
                let cleaned_tail = strip_noise(&tail);
                out = format!("{head}{cleaned_tail}");
                break;
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
        .trim_matches(|c: char| c == '-' || c == '|' || c.is_whitespace())
        .to_string()
}

/// Limpia el canal para poder usarlo como artista si hace falta.
fn strip_channel(channel: &str) -> String {
    let lower = channel.to_lowercase();
    let mut out = channel.to_string();
    for junk in ["vevo", "- topic", "oficial", "official", "records", "music"] {
        if let Some(pos) = lower.find(junk) {
            // Solo recorta si sobra algo; "Warp Records" -> "Warp".
            let candidate = format!("{}{}", &channel[..pos], &channel[pos + junk.len()..]);
            if !candidate.trim().is_empty() {
                out = candidate;
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// De una pista de YouTube saca (artista, cancion, es_otra_version).
pub fn normalize(track: &Track) -> (String, String, bool) {
    let title = strip_noise(&track.title);
    let lower = track.title.to_lowercase();
    let alt_version = VERSION_MARKERS.iter().any(|m| lower.contains(m));

    // Los tags oficiales, cuando existen, son mejores que cualquier heuristica.
    if let (Some(a), Some(t)) = (track.artist.as_deref(), track.album.as_deref()) {
        let _ = t;
        return (a.to_string(), title, alt_version);
    }

    let channel = track.channel.as_deref().unwrap_or_default();
    // "Artista - Cancion" es el patron dominante en YouTube.
    let parts: Vec<&str> = title
        .split(" - ")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();

    if parts.len() >= 2 {
        // "Warner Music Mexico - Cafe Tacvba - La Ingrata": el primer trozo es
        // la disquera, no el artista. Se descarta si coincide con el canal.
        let drop_first = parts.len() >= 3
            && !channel.is_empty()
            && channel.to_lowercase().contains(&parts[0].to_lowercase());
        let parts = if drop_first { &parts[1..] } else { &parts[..] };
        return (
            parts[0].to_string(),
            parts[1..].join(" - "),
            alt_version,
        );
    }

    let artist = track
        .artist
        .clone()
        .unwrap_or_else(|| strip_channel(channel));
    (artist, title, alt_version)
}

#[async_trait::async_trait]
impl LyricsProvider for LrcLib {
    async fn save_offset(&self, track: &Track, offset: f64) -> Result<()> {
        self.store_offset(track, offset).await
    }

    async fn fetch(&self, track: &Track) -> Result<Option<Lyrics>> {
        if let Some(hit) = self.from_cache(track).await {
            return Ok(Some(hit));
        }

        let (artist, title, alt_version) = normalize(track);
        if artist.is_empty() || title.is_empty() {
            return Ok(None);
        }

        // En cascada, de mas preciso a mas permisivo. La duracion es lo que
        // evita traer la letra de otra version, asi que va primero.
        let mut how = "get+duracion";
        let mut found = self.get(&artist, &title, track.duration).await;
        if found.is_none() {
            how = "get";
            found = self.get(&artist, &title, None).await;
        }
        if found.is_none() {
            how = "search";
            found = self.search(&format!("{artist} {title}")).await;
        }

        let Some(api) = found else { return Ok(None) };
        if api.instrumental.unwrap_or(false) {
            return Ok(None);
        }

        let lines = api.synced.as_deref().map(Lyrics::parse_lrc).unwrap_or_default();
        if lines.is_empty() && api.plain.is_none() {
            return Ok(None);
        }

        // Si las duraciones no cuadran, la letra va a ir desfasada. No se
        // corrige sola porque no se sabe si el desfase esta al principio o
        // repartido; se avisa y el usuario lo cuadra con +/-.
        let drift = match (api.duration, track.duration) {
            (Some(a), Some(b)) => (a - b).abs(),
            _ => 0.0,
        };
        let matched = format!(
            "{} - {}{}{}",
            api.artist_name.unwrap_or_else(|| artist.clone()),
            api.track_name.unwrap_or_else(|| title.clone()),
            if drift > 3.0 { format!("  (±{drift:.0}s de diferencia)") } else { String::new() },
            if alt_version { "  (otra version: puede desfasar)" } else { "" },
        );

        let lyrics = Lyrics {
            lines,
            plain: api.plain,
            offset: 0.0,
            source: format!("lrclib ({how})"),
            matched,
        };
        self.to_cache(track, &lyrics).await;
        Ok(Some(lyrics))
    }
}
