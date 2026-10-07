//! El daemon de surco visto desde la app: una petición JSON por conexión, una línea
//! de vuelta. Igual que el CLI; `SURCO_SOCKET` lo reubica.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<f64>,
    pub channel: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
}

impl Track {
    /// Quién la canta, o quién la subió si YouTube no dice más.
    pub fn who(&self) -> &str {
        self.artist.as_deref().or(self.channel.as_deref()).unwrap_or("")
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtistRef {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Library {
    pub favorites: Vec<Track>,
    pub playlists: Vec<Playlist>,
    #[serde(default)]
    pub artists: Vec<ArtistRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Play {
    pub track: Track,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Item {
    /// "album", "artist", "video" o "playlist".
    pub kind: String,
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub thumb: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct More {
    pub id: String,
    pub params: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Section {
    pub title: String,
    pub items: Vec<Item>,
    pub more: Option<More>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Listing {
    pub title: String,
    pub items: Vec<Item>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub artist_id: Option<String>,
    pub subtitle: String,
    pub thumb: Option<String>,
    pub tracks: Vec<Track>,
}

/// "Para ti": de qué canciones salió y lo que trae.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ForYou {
    pub seeds: Vec<Track>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Line {
    pub at: f64,
    pub text: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Lyrics {
    pub lines: Vec<Line>,
    pub plain: Option<String>,
    pub offset: f64,
}

/// Una petición. `Err` con el mensaje del daemon si contestó con error, o si no hay daemon.
pub async fn ask(req: Value) -> Result<Value, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let path = std::env::var_os("SURCO_SOCKET").map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join("surco.sock")
    });
    let fut = async {
        let mut s = tokio::net::UnixStream::connect(path).await.map_err(|_| "surco no responde".to_string())?;
        s.write_all(format!("{req}\n").as_bytes()).await.map_err(|e| e.to_string())?;
        let mut line = String::new();
        BufReader::new(s).read_line(&mut line).await.map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if v.get("status").and_then(Value::as_str) == Some("error") {
            return Err(v.get("message").and_then(Value::as_str).unwrap_or("error").to_string());
        }
        Ok(v)
    };
    // Reproducir espera a yt-dlp (~3 s); una lista larga de YouTube Music con la red
    // lenta, bastante más.
    tokio::time::timeout(Duration::from_secs(90), fut).await.map_err(|_| "surco tardó demasiado".to_string())?
}

pub async fn get<T: for<'de> Deserialize<'de>>(req: Value) -> Result<T, String> {
    let v = ask(req).await?;
    serde_json::from_value(v.get("payload").cloned().unwrap_or(Value::Null)).map_err(|e| e.to_string())
}

pub fn cmd(name: &str) -> Value {
    json!({ "cmd": name })
}

/// Una imagen, guardada en ~/.cache/surco/portadas con `key` de nombre. Sin `url`, la
/// portada del video de YouTube con ese id.
pub async fn thumb(key: String, url: Option<String>) -> (String, Option<Vec<u8>>) {
    let dir = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("surco/portadas");
    let file = dir.join(format!("{}.jpg", key.replace('/', "_")));
    if let Ok(b) = tokio::fs::read(&file).await {
        return (key, Some(b));
    }
    let url = url.unwrap_or_else(|| format!("https://i.ytimg.com/vi/{key}/mqdefault.jpg"));
    let bytes = async { reqwest::get(&url).await.ok()?.error_for_status().ok()?.bytes().await.ok() }.await;
    if let Some(b) = &bytes {
        let _ = tokio::fs::create_dir_all(&dir).await;
        let _ = tokio::fs::write(&file, b).await;
    }
    (key, bytes.map(|b| b.to_vec()))
}
