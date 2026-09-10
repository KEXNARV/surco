//! Letras sincronizadas.
//!
//! El formato LRC marca cada verso con su tiempo: `[01:23.45] texto`. Con eso
//! y la posicion que reporta el motor se puede resaltar el verso que suena.
//!
//! El problema no es conseguir la letra, es **cuadrarla**: los videos de
//! YouTube casi nunca empiezan donde empieza el track de estudio. Medido en
//! casos reales, el desfase llega a 20s. Por eso cada pista guarda su propio
//! offset, ajustable a mano y persistente.

pub mod lrclib;

use anyhow::Result;
use crate::resolver::Track;

/// Un verso y el segundo en que se canta.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Line {
    pub at: f64,
    pub text: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Lyrics {
    /// Versos con tiempo, ordenados. Vacio si solo hay texto plano.
    pub lines: Vec<Line>,
    /// Letra sin tiempos, para cuando no existe version sincronizada.
    pub plain: Option<String>,
    /// Segundos a sumar al tiempo del reproductor antes de buscar el verso.
    /// Positivo = la letra va adelantada y hay que retrasarla.
    pub offset: f64,
    /// De donde salio, para poder decirlo en la interfaz.
    pub source: String,
    /// Que artista/cancion se acabo usando: si el match fue malo, se ve aqui.
    pub matched: String,
}

impl Lyrics {
    pub fn is_synced(&self) -> bool {
        !self.lines.is_empty()
    }

    /// Indice del verso que suena en `position`. Devuelve `None` antes del
    /// primer verso (intros instrumentales largas).
    pub fn index_at(&self, position: f64) -> Option<usize> {
        let t = position - self.offset;
        // partition_point da el primer verso que empieza despues de t.
        match self.lines.partition_point(|l| l.at <= t) {
            0 => None,
            n => Some(n - 1),
        }
    }

    /// Parsea LRC. Tolera lineas sin tiempo, tags de metadata y varios
    /// timestamps en la misma linea (`[00:10.0][01:20.0] estribillo`).
    pub fn parse_lrc(raw: &str) -> Vec<Line> {
        let mut out = Vec::new();
        for line in raw.lines() {
            let mut rest = line;
            let mut stamps = Vec::new();

            // Come todos los [..] iniciales.
            while let Some(close) = rest.strip_prefix('[').and_then(|r| r.find(']')) {
                let inside = &rest[1..close + 1];
                rest = &rest[close + 2..];
                if let Some(sec) = parse_stamp(inside) {
                    stamps.push(sec);
                }
                // Un tag como [ar:Artista] no da tiempo: se ignora y seguimos.
            }

            let text = rest.trim().to_string();
            for at in stamps {
                out.push(Line { at, text: text.clone() });
            }
        }
        // El orden importa para partition_point y las fuentes no lo garantizan.
        out.sort_by(|a, b| a.at.total_cmp(&b.at));
        out
    }
}

/// `mm:ss.cc` o `mm:ss` -> segundos. Devuelve None si no es un tiempo.
fn parse_stamp(s: &str) -> Option<f64> {
    let (m, rest) = s.split_once(':')?;
    let mins: f64 = m.trim().parse().ok()?;
    let secs: f64 = rest.trim().parse().ok()?;
    Some(mins * 60.0 + secs)
}

#[async_trait::async_trait]
pub trait LyricsProvider: Send + Sync {
    async fn fetch(&self, track: &Track) -> Result<Option<Lyrics>>;
    /// Persiste el desfase que el usuario cuadro a mano.
    async fn save_offset(&self, track: &Track, offset: f64) -> Result<()>;
}
