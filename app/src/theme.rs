//! Colores del tema activo de Omarchy (los que aplica Aether): el acento reemplaza al celeste
//! de siempre y los tonos apagados salen de él. Se relee solo cuando cambia el tema.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;


// Los de siempre, por si no hay tema.
static ACCENT: AtomicU32 = AtomicU32::new(0x00c8ff);
static TEXT: AtomicU32 = AtomicU32::new(0xcde1eb);
static FAINT: AtomicU32 = AtomicU32::new(0x5a6e7d);
static BG: AtomicU32 = AtomicU32::new(0x05090d);
/// El contenido leído la última vez: Omarchy copia los temas conservando sus fechas, así que
/// la fecha del archivo no alcanza para saber si cambió.
static SEEN: Mutex<String> = Mutex::new(String::new());

fn path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SURCO_THEME") {
        return Some(p.into());
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".local/state/omarchy/current/theme/colors.toml"))
}

/// Relee el tema si cambió desde la última vez. Devuelve true si cambió algo.
pub fn poll() -> bool {
    let Some(p) = path() else { return false };
    let Ok(text) = std::fs::read_to_string(&p) else { return false };
    let mut seen = SEEN.lock().unwrap();
    if *seen == text {
        return false;
    }
    *seen = text;
    apply(&seen)
}

fn apply(toml: &str) -> bool {
    let get = |key: &str| {
        toml.lines().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == key).then(|| parse_hex(v.trim().trim_matches('"')))?
        })
    };
    let Some(accent) = get("accent") else { return false };
    ACCENT.store(accent, Ordering::Relaxed);
    if let Some(fg) = get("foreground") {
        TEXT.store(fg, Ordering::Relaxed);
    }
    if let Some(f) = get("light_foreground").or_else(|| get("dark_foreground")) {
        FAINT.store(f, Ordering::Relaxed);
    }
    if let Some(bg) = get("background") {
        BG.store(bg, Ordering::Relaxed);
    }
    true
}

fn parse_hex(s: &str) -> Option<u32> {
    let h = s.strip_prefix('#')?;
    (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok())?
}

fn rgb(v: u32) -> [f64; 3] {
    [(v >> 16 & 0xff) as f64, (v >> 8 & 0xff) as f64, (v & 0xff) as f64]
}


fn mix(a: [f64; 3], b: [f64; 3], f: f64) -> [f64; 3] {
    [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
}

pub fn text_rgb() -> [f64; 3] {
    rgb(TEXT.load(Ordering::Relaxed))
}

pub fn faint_rgb() -> [f64; 3] {
    rgb(FAINT.load(Ordering::Relaxed))
}

pub fn bg_rgb() -> [f64; 3] {
    rgb(BG.load(Ordering::Relaxed))
}

pub fn accent_rgb() -> [f64; 3] {
    rgb(ACCENT.load(Ordering::Relaxed))
}

/// El acento apagado hacia el fondo: bordes, marcas de la escala, lo secundario.
pub fn dim_rgb() -> [f64; 3] {
    mix(accent_rgb(), rgb(BG.load(Ordering::Relaxed)), 0.6)
}

/// Variantes del acento para los estados de «presencia» del núcleo.
pub fn accent_toward_white(f: f64) -> [f64; 3] {
    mix(accent_rgb(), [255.0; 3], f)
}

pub fn accent_darker(f: f64) -> [f64; 3] {
    mix(accent_rgb(), [0.0; 3], f)
}
