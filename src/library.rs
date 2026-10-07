//! Lo que es tuyo y no de la fuente: favoritos, playlists e historial de escucha.
//!
//! Favoritos y playlists viven en un JSON que se reescribe entero en cada cambio
//! (son pocos cientos de pistas como mucho). El historial es append-only, una línea
//! por reproducción, porque crece sin parar y de él saldrán las recomendaciones.

use crate::resolver::Track;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
}

/// Un artista que sigues: lo justo para listarlo y abrir su página.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistRef {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Data {
    pub favorites: Vec<Track>,
    pub playlists: Vec<Playlist>,
    #[serde(default)]
    pub artists: Vec<ArtistRef>,
}

/// Una reproducción terminada: cuánto se escuchó de cuánto. Un salto antes de los
/// 30 s o de la mitad cuenta como rechazo para las recomendaciones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Play {
    pub track: Track,
    /// Segundos desde la época al empezar.
    pub at: u64,
    pub listened: f64,
    pub duration: Option<f64>,
    /// "eof" si terminó sola, "skip" si se cambió, "stop" si se detuvo.
    pub end: String,
}

pub struct Library {
    dir: PathBuf,
    data: Mutex<Data>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Library {
    /// `SURCO_DATA` la reubica: la instancia de pruebas no toca tu biblioteca.
    pub async fn open() -> Result<Self> {
        let dir = match std::env::var_os("SURCO_DATA") {
            Some(d) => PathBuf::from(d),
            None => dirs::data_dir().context("sin directorio de datos")?.join("surco"),
        };
        tokio::fs::create_dir_all(&dir).await?;
        let data = match tokio::fs::read(dir.join("library.json")).await {
            Ok(bytes) => serde_json::from_slice(&bytes).context("library.json corrupto")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Data::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { dir, data: Mutex::new(data) })
    }

    pub async fn data(&self) -> Data {
        self.data.lock().await.clone()
    }

    /// Escribe a un temporal y renombra: un corte a mitad no deja el archivo a medias.
    async fn save(&self, data: &Data) -> Result<()> {
        let tmp = self.dir.join("library.json.tmp");
        tokio::fs::write(&tmp, serde_json::to_vec_pretty(data)?).await?;
        tokio::fs::rename(&tmp, self.dir.join("library.json")).await?;
        Ok(())
    }

    async fn edit<T>(&self, f: impl FnOnce(&mut Data) -> Result<T>) -> Result<T> {
        let mut data = self.data.lock().await;
        let out = f(&mut data)?;
        self.save(&data).await?;
        Ok(out)
    }

    pub async fn set_favorite(&self, track: Track, on: bool) -> Result<()> {
        self.edit(|d| {
            d.favorites.retain(|t| t.id != track.id);
            if on {
                // Lo último que marcaste va primero, como en Spotify.
                d.favorites.insert(0, track);
            }
            Ok(())
        })
        .await
    }

    pub async fn set_following(&self, artist: ArtistRef, on: bool) -> Result<()> {
        self.edit(|d| {
            d.artists.retain(|a| a.id != artist.id);
            if on {
                d.artists.insert(0, artist);
            }
            Ok(())
        })
        .await
    }

    pub async fn create_playlist(&self, name: &str) -> Result<Playlist> {
        let name = name.trim();
        if name.is_empty() {
            bail!("la playlist necesita un nombre");
        }
        self.edit(|d| {
            let p = Playlist { id: format!("p{}", now() * 1000 + d.playlists.len() as u64), name: name.into(), tracks: vec![] };
            d.playlists.push(p.clone());
            Ok(p)
        })
        .await
    }

    fn find<'a>(d: &'a mut Data, id: &str) -> Result<&'a mut Playlist> {
        d.playlists.iter_mut().find(|p| p.id == id).with_context(|| format!("no existe la playlist {id}"))
    }

    pub async fn rename_playlist(&self, id: &str, name: &str) -> Result<()> {
        let name = name.trim().to_string();
        if name.is_empty() {
            bail!("la playlist necesita un nombre");
        }
        self.edit(|d| {
            Self::find(d, id)?.name = name;
            Ok(())
        })
        .await
    }

    pub async fn delete_playlist(&self, id: &str) -> Result<()> {
        self.edit(|d| {
            let before = d.playlists.len();
            d.playlists.retain(|p| p.id != id);
            if d.playlists.len() == before {
                bail!("no existe la playlist {id}");
            }
            Ok(())
        })
        .await
    }

    pub async fn playlist_add(&self, id: &str, track: Track) -> Result<()> {
        self.edit(|d| {
            let p = Self::find(d, id)?;
            if p.tracks.iter().any(|t| t.id == track.id) {
                bail!("ya está en «{}»", p.name);
            }
            p.tracks.push(track);
            Ok(())
        })
        .await
    }

    pub async fn playlist_remove(&self, id: &str, track_id: &str) -> Result<()> {
        self.edit(|d| {
            Self::find(d, id)?.tracks.retain(|t| t.id != track_id);
            Ok(())
        })
        .await
    }

    pub async fn record(&self, track: Track, started: u64, listened: f64, duration: Option<f64>, end: &str) -> Result<()> {
        let play = Play { track, at: started, listened, duration, end: end.into() };
        let mut line = serde_json::to_vec(&play)?;
        line.push(b'\n');
        let mut f = tokio::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("history.jsonl")).await?;
        f.write_all(&line).await?;
        Ok(())
    }

    /// Lo escuchado hace poco, sin repetir pistas, lo más reciente primero.
    pub async fn recent(&self, limit: usize) -> Result<Vec<Play>> {
        let text = match tokio::fs::read_to_string(self.dir.join("history.jsonl")).await {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        let mut seen = std::collections::HashSet::new();
        Ok(text
            .lines()
            .rev()
            .filter_map(|l| serde_json::from_str::<Play>(l).ok())
            .filter(|p| seen.insert(p.track.id.clone()))
            .take(limit)
            .collect())
    }
}

pub fn timestamp() -> u64 {
    now()
}
