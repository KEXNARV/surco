//! Fuente de pistas via yt-dlp.
//!
//! Dos llamadas distintas a proposito:
//!   - buscar usa `--flat-playlist`, que no toca cada video y tarda ~1.4s
//!   - resolver el stream si extrae el video y tarda ~2.7s
//! Por eso la busqueda es barata y la reproduccion necesita prefetch.

use super::{Resolver, Track};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use tokio::process::Command;

pub struct YtDlpResolver {
    /// Formato en notacion de yt-dlp. Opus es lo que YouTube sirve nativo, asi
    /// que pedirlo evita una transcodificacion.
    format: String,
}

impl Default for YtDlpResolver {
    fn default() -> Self {
        Self {
            format: "bestaudio[acodec=opus]/bestaudio".to_string(),
        }
    }
}

impl YtDlpResolver {
    async fn run(&self, args: &[&str]) -> Result<Vec<u8>> {
        let out = Command::new("yt-dlp")
            .args(args)
            .output()
            .await
            .context("no se pudo ejecutar yt-dlp; esta instalado?")?;

        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            // El primer sintoma de que YouTube cambio algo aparece aqui.
            bail!(
                "yt-dlp fallo ({}): {}",
                out.status,
                err.lines().last().unwrap_or("sin detalle").trim()
            );
        }
        Ok(out.stdout)
    }
}

/// Saca un f64 de un campo que yt-dlp a veces manda como numero y a veces nulo.
fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
    // filter() importa: yt-dlp devuelve "" en vez de null en varios campos.
}

#[async_trait::async_trait]
impl Resolver for YtDlpResolver {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Track>> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }
        // ytsearchN: busca en YouTube. El prefijo va dentro del argumento, no
        // como flag, y hay que armarlo con el limite ya interpolado.
        let spec = format!("ytsearch{}:{}", limit.clamp(1, 50), query);
        let stdout = self
            .run(&[&spec, "--flat-playlist", "-J", "--no-warnings"])
            .await?;

        let root: Value = serde_json::from_slice(&stdout)
            .context("yt-dlp devolvio algo que no es JSON de busqueda")?;

        let entries = root
            .get("entries")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);

        Ok(entries
            .iter()
            // Sin id no hay nada que reproducir: videos privados o borrados.
            .filter_map(|e| {
                let id = text(e, "id")?;
                Some(Track {
                    id,
                    title: text(e, "title").unwrap_or_else(|| "(sin titulo)".into()),
                    artist: text(e, "artist"),
                    album: text(e, "album"),
                    duration: num(e, "duration"),
                    channel: text(e, "channel").or_else(|| text(e, "uploader")),
                })
            })
            .collect())
    }

    async fn stream_url(&self, track: &Track) -> Result<String> {
        let page = format!("https://www.youtube.com/watch?v={}", track.id);
        let stdout = self
            .run(&["-f", &self.format, "-g", "--no-warnings", &page])
            .await?;

        // Con -f de audio solo debe venir una URL; si vinieran dos seria
        // porque el selector cayo a un formato con video.
        String::from_utf8_lossy(&stdout)
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with("http"))
            .map(str::to_owned)
            .context("yt-dlp no devolvio ninguna URL de stream")
    }
}
