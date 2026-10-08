//! URLs de stream ya resueltas, por pista. yt-dlp tarda ~1.7 s en sacar una (casi todo es
//! ir a YouTube) y mpv solo ~0.3 s en abrirla, así que lo que hace sentir lento a surco es
//! resolver. Aquí se guarda cada URL hasta que caduca (`expire=` en la propia URL, unas
//! 6 h), y si dos pedidos llegan a la vez por la misma pista (pasar el mouse y luego hacer
//! clic, o el prefetch y un salto) esperan a la misma llamada de yt-dlp en vez de lanzar dos.
//!
//! Ojo: a veces yt-dlp entrega una URL que YouTube solo deja leer en trozos chicos (hasta
//! ~0.5 MB) y que a mpv le da 403. Guardada, esa canción no cargaba nunca. Un HEAD (~20 ms)
//! la delata: 403 en la mala, 200 en la buena. Se revisa al resolver y al sacar de la caché.

use crate::resolver::{Resolver, Track};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::OnceCell;

/// Se da por caducada un poco antes: una canción larga empezada al filo se cortaría.
const MARGIN: u64 = 15 * 60;
/// Si la URL no dice cuándo caduca, se le cree una hora.
const DEFAULT_TTL: u64 = 60 * 60;

type Cell = Arc<OnceCell<(String, u64)>>;

pub struct Urls {
    cells: Mutex<HashMap<String, Cell>>,
    http: reqwest::Client,
    /// Las de audio (para mpv) o las de video (para verlo en la app).
    video: bool,
}

impl Default for Urls {
    fn default() -> Self {
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(4)).build().unwrap_or_default();
        Self { cells: Default::default(), http, video: false }
    }
}

impl Urls {
    pub fn video() -> Self {
        Self { video: true, ..Default::default() }
    }

    /// La URL de `track`, ya comprobada: la guardada si sigue viva y YouTube la acepta, la
    /// que ya se está resolviendo, o una nueva.
    pub async fn get(&self, resolver: &dyn Resolver, track: &Track) -> Result<String> {
        let url = self.cached(resolver, track).await?;
        if self.usable(&url).await {
            return Ok(url);
        }
        // Mala (recién resuelta o ya guardada): fuera, y una vez más desde cero.
        self.forget(&track.id);
        let url = self.cached(resolver, track).await?;
        if self.usable(&url).await {
            return Ok(url);
        }
        self.forget(&track.id);
        let what = if self.video { "el video de " } else { "" };
        anyhow::bail!("YouTube no deja abrir {what}«{}» ahora mismo; prueba en un rato", track.title)
    }

    /// ¿YouTube deja leer la URL entera? Si el HEAD no llega (sin red, lento), se le da el
    /// beneficio de la duda: que lo intente mpv.
    async fn usable(&self, url: &str) -> bool {
        match self.http.head(url).send().await {
            Ok(r) => r.status() != reqwest::StatusCode::FORBIDDEN,
            Err(_) => true,
        }
    }

    async fn cached(&self, resolver: &dyn Resolver, track: &Track) -> Result<String> {
        let cell = {
            let mut cells = self.cells.lock().unwrap();
            let now = now();
            cells.retain(|_, c| c.get().is_none_or(|(_, exp)| *exp > now));
            cells.entry(track.id.clone()).or_default().clone()
        };
        let got = cell
            .get_or_try_init(|| async {
                let url = if self.video { resolver.video_url(track).await? } else { resolver.stream_url(track).await? };
                let exp = expiry(&url).unwrap_or(now() + DEFAULT_TTL).saturating_sub(MARGIN);
                Ok::<_, anyhow::Error>((url, exp))
            })
            .await;
        match got {
            Ok((url, _)) => Ok(url.clone()),
            Err(e) => {
                // Que el próximo intento vuelva a preguntar en vez de quedarse con la celda vacía.
                self.drop_cell(&track.id, &cell);
                Err(e)
            }
        }
    }

    /// Olvida la URL de una pista (mpv no pudo abrirla: caducó antes o cambió la IP).
    pub fn forget(&self, id: &str) {
        self.cells.lock().unwrap().remove(id);
    }

    fn drop_cell(&self, id: &str, cell: &Cell) {
        let mut cells = self.cells.lock().unwrap();
        if cells.get(id).is_some_and(|c| Arc::ptr_eq(c, cell)) {
            cells.remove(id);
        }
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// El `expire=` (segundos Unix) de una URL de googlevideo.
fn expiry(url: &str) -> Option<u64> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| kv.strip_prefix("expire=")?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lee_expire() {
        assert_eq!(expiry("https://r1.googlevideo.com/videoplayback?expire=1791400000&ei=x"), Some(1791400000));
        assert_eq!(expiry("https://r1.googlevideo.com/videoplayback?ei=x&expire=17"), Some(17));
        assert_eq!(expiry("https://example.com/a"), None);
    }
}
