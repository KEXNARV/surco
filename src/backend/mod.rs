//! El motor de audio: lo unico que sabe convertir una URL en sonido.
//!
//! Todo lo que esta arriba (cola, estado, IPC) habla solo con este trait, asi
//! que cambiar mpv por librespot o por un decodificador propio no toca nada mas.

pub mod mpv;

use anyhow::Result;

/// Lo que el motor reporta sobre si mismo, muestreado cada tick.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Playback {
    /// Segundos transcurridos de la pista actual.
    pub position: f64,
    /// Duracion total, si el motor la conoce.
    pub duration: Option<f64>,
    pub paused: bool,
    /// `true` cuando no hay nada cargado o la pista termino.
    pub idle: bool,
}

#[async_trait::async_trait]
pub trait Backend: Send + Sync {
    /// Carga una URL ya resuelta y empieza a reproducirla.
    async fn load(&self, url: &str) -> Result<()>;
    async fn set_paused(&self, paused: bool) -> Result<()>;
    /// Salta a una posicion absoluta en segundos.
    async fn seek(&self, seconds: f64) -> Result<()>;
    /// Volumen 0-100.
    async fn set_volume(&self, volume: f64) -> Result<()>;
    async fn playback(&self) -> Result<Playback>;
    /// Detiene la reproduccion y descarga la pista.
    async fn stop(&self) -> Result<()>;
    async fn shutdown(&self) -> Result<()>;
}
