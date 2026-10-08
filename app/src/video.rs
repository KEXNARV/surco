//! El video de lo que suena, en la vista completa. El audio sigue saliendo del mpv del
//! daemon; aquí se reproduce aparte el stream de solo video (H.264, sin audio) con
//! GStreamer y se le hace seguir la posición que reporta el daemon.
//!
//! Los dos relojes son el del sistema, así que no se separan solos: lo que hay que corregir
//! es el arranque y los saltos. Como saltar en el stream de YouTube tarda segundos, el
//! video salta por delante del audio, espera en pausa y se suelta cuando el audio llega.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gstreamer::{self as gst, prelude::*};
use iced_video_player::Video;

/// Desfase a partir del cual se salta: por debajo no se nota y un seek sí (un tirón).
const MAX_DRIFT: f64 = 0.15;
/// Tras soltarlo, GStreamer tarda en asentarse; medir antes da desfases falsos.
const SETTLE: Duration = Duration::from_millis(1000);

/// Un `Video` recién creado viaja en un `Msg`, que tiene que ser `Clone`.
#[derive(Clone)]
pub struct Loaded(Arc<Mutex<Option<Video>>>);

impl std::fmt::Debug for Loaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Video")
    }
}

impl Loaded {
    pub fn take(&self) -> Option<Video> {
        self.0.lock().ok()?.take()
    }
}

/// Abre la URL fuera del hilo de la interfaz: `Video::new` espera el preroll (hasta 5 s).
pub async fn open(url: String) -> Result<Loaded, String> {
    tokio::task::spawn_blocking(move || {
        let uri = url::Url::parse(&url).map_err(|e| e.to_string())?;
        let mut v = Video::new(&uri).map_err(|e| format!("no se pudo abrir el video: {e}"))?;
        v.set_muted(true);
        Ok(Loaded(Arc::new(Mutex::new(Some(v)))))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Cómo se ve el video en la vista completa.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Layout {
    /// A la izquierda, con la letra al lado.
    Side,
    /// Todo el ancho de la ventana, con el reproductor abajo.
    Wide,
    /// La ventana en pantalla completa y solo el video.
    Screen,
}

pub enum State {
    /// Pidiendo la URL o abriéndola.
    Loading,
    Ready(Player),
    Failed(String),
}

pub struct Player {
    pub video: Video,
    /// Esperando en pausa a que el audio llegue a `.0` (tras un salto, o porque el video
    /// iba adelantado); `.1` es dónde estaba el audio entonces.
    hold: Option<(f64, f64)>,
    /// Cuándo arrancó o se soltó por última vez: antes de asentarse, las medidas mienten.
    since: Instant,
    /// Cuánto por delante del audio se salta. Un salto en el stream de YouTube tarda de
    /// 0.7 a 4 s: se ajusta a lo que tardó el último, con margen.
    lead: f64,
    /// Cuándo se pidió el salto en curso, para medir cuánto tarda.
    jumped: Option<Instant>,
}

impl Player {
    pub fn new(video: Video) -> Self {
        Self { video, hold: None, since: Instant::now(), lead: 3.0, jumped: None }
    }

    /// Lo pone en la posición y el estado del audio. `pos` es la del daemon, ya adelantada
    /// por lo que pasó desde el último status. Si está por soltarlo, dice en cuánto: los
    /// status llegan cada 500 ms y los cuadros no llegan con la ventana oculta.
    pub fn follow(&mut self, pos: f64, paused: bool) -> Option<Duration> {
        if let Some((target, from)) = self.hold {
            if let Some(t) = self.jumped.filter(|_| self.prerolled()) {
                self.lead = (t.elapsed().as_secs_f64() * 1.5 + 0.3).clamp(1.0, 10.0);
                self.jumped = None;
            }
            if pos < from - 1.0 || pos > target + 5.0 {
                // Movieron el audio mientras esperaba: saltar a donde está ahora.
                self.jump(pos);
            } else if pos >= target {
                if pos > target + 0.5 || !self.prerolled() {
                    // El salto no llegó a tiempo: este, más lejos.
                    self.lead = (self.lead * 1.5).min(10.0);
                    self.jump(pos);
                } else if !paused {
                    self.hold = None;
                    self.video.set_paused(false);
                    self.since = Instant::now();
                }
            } else if !paused && target - pos < 0.6 {
                return Some(Duration::from_secs_f64(target - pos));
            }
            return None;
        }
        if self.video.paused() != paused {
            self.video.set_paused(paused);
        }
        if paused || self.since.elapsed() < SETTLE {
            return None;
        }
        let at = self.video.position().as_secs_f64();
        let end = self.video.duration().as_secs_f64();
        // Pasado el final del video (a veces es más corto que el audio) no hay nada que buscar.
        if end > 0.0 && pos >= end {
            return None;
        }
        let ahead = at - pos;
        if ahead > MAX_DRIFT && ahead < 5.0 {
            // Adelantado: basta con esperar al audio, sin pedirle nada a YouTube.
            self.video.set_paused(true);
            self.hold = Some((at, pos));
        } else if ahead.abs() > MAX_DRIFT {
            self.jump(pos);
        }
        None
    }

    /// El salto ya terminó y el cuadro de destino está listo para mostrarse.
    fn prerolled(&self) -> bool {
        !matches!(self.video.pipeline().state(gst::ClockTime::ZERO).0, Ok(gst::StateChangeSuccess::Async))
    }

    /// Salta por delante del audio y espera ahí en pausa.
    fn jump(&mut self, pos: f64) {
        let target = pos + self.lead;
        self.video.set_paused(true);
        // Preciso: el no preciso cae en el keyframe anterior, a segundos de distancia.
        if self.video.seek(Duration::from_secs_f64(target.max(0.0)), true).is_ok() {
            self.hold = Some((target, pos));
            self.jumped = Some(Instant::now());
        }
    }
}
