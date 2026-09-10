//! Protocolo entre el daemon y quien lo maneje: JSON, una peticion por linea.
//!
//! Deliberadamente tonto y textual. Cualquier interfaz futura -- TUI, Tauri,
//! un script de Hyprland, `socat` a mano -- habla esto sin librerias.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Busca y reproduce el primer resultado; el resto queda en cola.
    Play { query: String },
    /// Añade a la cola sin interrumpir lo que suena.
    Add { query: String },
    /// Solo busca y devuelve resultados, sin tocar la reproduccion.
    Search { query: String, limit: Option<usize> },
    Toggle,
    Pause,
    Resume,
    Next,
    Prev,
    Stop,
    Status,
    Queue,
    ClearQueue,
    Volume { level: f64 },
    Seek { seconds: f64 },
    /// Salta a un indice concreto de la cola.
    Jump { index: usize },
    Quit,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok { message: String },
    Data { payload: serde_json::Value },
    Error { message: String },
}

impl Response {
    pub fn ok(message: impl Into<String>) -> Self {
        Self::Ok {
            message: message.into(),
        }
    }
    pub fn data(payload: impl Serialize) -> Self {
        match serde_json::to_value(payload) {
            Ok(payload) => Self::Data { payload },
            Err(e) => Self::Error {
                message: format!("no se pudo serializar la respuesta: {e}"),
            },
        }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

/// Socket del daemon. En el runtime dir del usuario para que se limpie solo al
/// cerrar sesion y no quede accesible a otros usuarios como pasaria en /tmp.
pub fn socket_path() -> std::path::PathBuf {
    let base = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    base.join("surco.sock")
}

pub fn mpv_socket_path() -> std::path::PathBuf {
    let base = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    base.join("surco-mpv.sock")
}
