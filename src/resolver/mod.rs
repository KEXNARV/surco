//! De donde salen las pistas: busqueda, metadata y la URL final del stream.
//!
//! Separado del motor a proposito. yt-dlp resuelve *que* suena; mpv se encarga
//! de *como*. Un backend de biblioteca local implementaria esto leyendo tags.

pub mod ytdlp;

use anyhow::Result;

/// Una pista tal como la conocemos antes de tener el stream.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Track {
    /// Identificador estable dentro de la fuente (para YouTube, el video id).
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<f64>,
    /// Quien subio el contenido. Es lo unico que hay cuando no viene metadata.
    pub channel: Option<String>,
}

impl Track {
    /// Como mostrarla en una linea. Cae al canal cuando no hay artista, que es
    /// el caso comun en subidas de usuarios.
    pub fn label(&self) -> String {
        one_line(self.artist.as_deref().or(self.channel.as_deref()), &self.title)
    }
}

/// Junta interprete y titulo sin repetirse. La mitad de los titulos de YouTube
/// ya vienen como "Artista - Cancion", y prefijar a ciegas produce
/// "Radiohead - Radiohead - Weird Fishes".
pub fn one_line(who: Option<&str>, title: &str) -> String {
    match who {
        Some(w) if title.to_lowercase().starts_with(&w.to_lowercase()) => title.to_string(),
        Some(w) => format!("{w} - {title}"),
        None => title.to_string(),
    }
}

#[async_trait::async_trait]
pub trait Resolver: Send + Sync {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Track>>;
    /// Consigue la URL reproducible. Se llama justo antes de sonar porque
    /// las URLs de googlevideo caducan y estan atadas a la IP que las pidio.
    async fn stream_url(&self, track: &Track) -> Result<String>;
}
