//! URLs de stream ya resueltas, por pista. yt-dlp tarda ~1.7 s en sacar una (casi todo es
//! ir a YouTube) y mpv solo ~0.3 s en abrirla, así que lo que hace sentir lento a surco es
//! resolver. Aquí se guarda cada URL hasta que caduca (`expire=` en la propia URL, unas
//! 6 h), y si dos pedidos llegan a la vez por la misma pista (pasar el mouse y luego hacer
//! clic, o el prefetch y un salto) esperan a la misma llamada de yt-dlp en vez de lanzar dos.

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

#[derive(Default)]
pub struct Urls {
    cells: Mutex<HashMap<String, Cell>>,
}

impl Urls {
    /// La URL de `track`: la guardada si sigue viva, la que ya se está resolviendo, o una nueva.
    pub async fn get(&self, resolver: &dyn Resolver, track: &Track) -> Result<String> {
        let cell = {
            let mut cells = self.cells.lock().unwrap();
            let now = now();
            cells.retain(|_, c| c.get().is_none_or(|(_, exp)| *exp > now));
            cells.entry(track.id.clone()).or_default().clone()
        };
        let got = cell
            .get_or_try_init(|| async {
                let url = resolver.stream_url(track).await?;
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
