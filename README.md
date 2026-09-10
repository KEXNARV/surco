# surco

Reproductor de música con el motor de audio desacoplado de la fuente de las
pistas. Hoy suena con yt-dlp + mpv; mañana puede sonar con librespot o con una
biblioteca local sin reescribir el reproductor.

## Por qué así

Spotify no entrega audio por API. Las únicas vías son el Web Playback SDK
(navegador + DRM + Premium), librespot (Premium, zona gris de ToS) o mandar a
un cliente oficial por Connect (Premium). Las tres cuestan suscripción, así que
el motor arranca con yt-dlp: catálogo amplio, gratis, opus ~130 kbps.

A cambio, yt-dlp es frágil — YouTube rompe los extractores cada pocas semanas.
Por eso la fuente vive detrás de un trait y no cableada por el código.

## Arquitectura

    src/backend/     trait Backend  — convierte una URL en sonido
      mpv.rs         actor sobre el JSON-IPC de mpv
    src/resolver/    trait Resolver — busca pistas y resuelve el stream
      ytdlp.rs       subprocesos de yt-dlp
    src/player.rs    cola, estado, prefetch, avance automático
    src/ipc.rs       protocolo daemon <-> interfaz (JSON por línea)
    src/main.rs      daemon + CLI cliente

El daemon posee mpv y la cola. Cualquier interfaz habla el socket unix en
`$XDG_RUNTIME_DIR/surco.sock` — el CLI incluido, o lo que venga después.

Números medidos en esta máquina:

| operación                | tiempo |
|--------------------------|--------|
| buscar (`--flat-playlist`) | ~1.4s |
| resolver el stream (`-g`)   | ~2.7s |
| cambiar de pista con prefetch | inmediato |

Los 2.7s son la razón de existir del prefetch: en cuanto empieza una pista, la
siguiente se resuelve en segundo plano.

## Uso

    cargo build --release
    ./target/release/surco daemon &

    surco play weird fishes      # busca, suena la primera, el resto en cola
    surco add kid a              # añade sin cortar
    surco search boards of canada
    surco status / queue / next / prev / toggle / stop
    surco vol 40
    surco seek 90
    surco jump 3
    surco kill

## Detalles que cuestan sangre

- **Las URLs de googlevideo caducan** (~6h) y van atadas a la IP que las pidió.
  Por eso se resuelven justo antes de reproducir y no se cachean en disco.
- **`end-file` de mpv no significa "terminó la canción"**. Salta igual cuando
  cargas otra pista encima o al salir. Sin filtrar por `reason == "eof"`, cada
  `next` avanzaba dos pistas.
- **mpv olvida el volumen entre archivos** con `--no-config`, así que se
  reaplica en cada carga.
- **Metadata pobre**: los canales oficiales traen `artist`/`album`; las subidas
  de usuarios vienen vacías y solo hay nombre de canal.

## Pendiente

- **La interfaz.** El core no la presupone. Las opciones sobre la mesa: daemon
  con MPRIS + TUI, app Tauri, o solo barra de Hyprland con menú de búsqueda.
- **MPRIS** (`zbus`), para que playerctl, waybar y los controles multimedia del
  teclado lo reconozcan sin código específico.
- **SponsorBlock**: `--sponsorblock-remove` de yt-dlp NO sirve aquí — exige
  descargar y postprocesar con ffmpeg, y nosotros streameamos directo. La vía
  real es consultar la API de SponsorBlock y hacer `seek` sobre los segmentos.
- **Persistir la cola** entre reinicios del daemon.
- **Backend de biblioteca local** — la implementación más barata del trait, y
  la que hace que el proyecto no dependa de que YouTube siga cooperando.
