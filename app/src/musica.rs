//! Lo que suena en el equipo, para que el núcleo baile. Copia de la de Iris. Igual que el teclado
//! (ghubd): `cava` por PipeWire en modo crudo, 33 bandas de 0 a 100 por línea, y el nivel es
//! el mayor entre la media de los bajos y la de los medios.

use std::io::BufRead;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// La misma configuración que usa ghubd: con `noise_reduction` en 77 (lo de fábrica) el
/// medidor llegaba tarde a los golpes.
const CAVA_CONF: &str = "[general]
bars = 33
framerate = 60
[input]
method = pipewire
source = auto
[output]
method = raw
channels = mono
raw_target = /dev/stdout
data_format = ascii
ascii_max_range = 100
bar_delimiter = 59
frame_delimiter = 10
[smoothing]
noise_reduction = 20
";

/// Bandas de cava (de 33, logarítmicas de 50 Hz a 10 kHz): bajos 0-4, medios 5-15.
const BASS: std::ops::Range<usize> = 0..5;
const MID: std::ops::Range<usize> = 5..16;
/// El `bar_gain` del teclado, para que los dos lleguen al rojo con los mismos golpes.
const GAIN: f32 = 1.5;
/// Si cava se muere (al iniciar sesión puede arrancar antes que PipeWire), se relanza.
const RESPAWN: Duration = Duration::from_secs(2);

#[derive(Clone, Default)]
pub struct Musica {
    levels: Arc<Mutex<Vec<f32>>>,
}

impl Musica {
    /// Lanza cava en segundo plano. Sin cava no pasa nada: el nivel queda en cero.
    pub fn spawn() -> Self {
        let m = Musica::default();
        let levels = m.levels.clone();
        let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let conf = dir.join("surco-cava.conf");
        if std::fs::write(&conf, CAVA_CONF).is_err() {
            return m;
        }
        thread::spawn(move || loop {
            let child = Command::new("cava")
                .arg("-p")
                .arg(&conf)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            let Ok(mut child) = child else { return }; // no está instalado
            if let Some(out) = child.stdout.take() {
                for line in std::io::BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    let v = line.split(';').filter_map(|s| s.parse::<f32>().ok()).map(|x| x / 100.0).collect();
                    *levels.lock().unwrap() = v;
                }
            }
            let _ = child.wait();
            levels.lock().unwrap().clear();
            thread::sleep(RESPAWN);
        });
        m
    }

    /// Nivel 0..1 (puede pasarse un poco por la ganancia), como lo calcula el teclado.
    pub fn level(&self) -> f32 {
        let l = self.levels.lock().unwrap();
        (mean(&l, BASS).max(mean(&l, MID)) * GAIN).min(1.2)
    }
}

fn mean(l: &[f32], r: std::ops::Range<usize>) -> f32 {
    let r = r.start.min(l.len())..r.end.min(l.len());
    if r.is_empty() { 0.0 } else { l[r.clone()].iter().sum::<f32>() / r.len() as f32 }
}
