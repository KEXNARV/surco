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
    /// Letra sincronizada de lo que suena.
    Lyrics,
    /// Corre la letra `delta` segundos y lo persiste.
    LyricsOffset { delta: f64 },
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
///
/// `SURCO_SOCKET` lo reubica, que es lo que permite levantar una instancia de
/// pruebas sin tocar la que esta sonando.
pub fn socket_path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("SURCO_SOCKET") {
        return std::path::PathBuf::from(p);
    }
    let base = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    base.join("surco.sock")
}

/// El socket de mpv acompaña al del daemon: dos instancias no pueden compartir
/// el mismo mpv.
pub fn mpv_socket_path() -> std::path::PathBuf {
    let main = socket_path();
    let name = main
        .file_name()
        .map(|n| format!("{}-mpv.sock", n.to_string_lossy()))
        .unwrap_or_else(|| "surco-mpv.sock".into());
    main.with_file_name(name)
}
