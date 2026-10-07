# Base de la app: Tauri 2 y su riesgo en esta máquina

Investigación del 2026-10-06. Máquina: Hyprland 0.56.2, Intel ARL + RTX 5070
Laptop (driver 610.57.04), webkit2gtk-4.1 2.52.6.

## Conclusión

Tauri 2 vale, condicionado a una medición de ~30 min con ventana real. Plan B:
Electron. Tauri con CEF no es opción hoy.

## Por qué

- Los bugs conocidos de WebKitGTK con Wayland + NVIDIA (pantalla en blanco,
  Error 71, cierres al redimensionar) vienen del renderizador DMABUF contra el
  driver NVIDIA. Remedios en orden: `__NV_DISABLE_EXPLICIT_SYNC=1`,
  `WEBKIT_DISABLE_DMABUF_RENDERER=1` (más lag), `WEBKIT_DISABLE_COMPOSITING_MODE=1`.
  Issues abiertas: tauri#10702, #9304, #14924 (transparencia), wry#1366, #618.
- **Aquí renderiza la Intel:** card0 (i915) es la primaria de aquamarine y
  `eglinfo` elige Mesa Intel para Wayland/GBM. Eso esquiva casi todo lo
  anterior. Forzar la NVIDIA: `WEBKIT_WEB_RENDER_DEVICE_FILE=/dev/dri/renderD128`.
- Riesgos propios:
  - DP-1 y HDMI-A-1 cuelgan de la NVIDIA: en esos monitores cada frame cruza de
    GPU. Hay que medir ahí, no solo en la eDP-2.
  - eDP-2 a 240 Hz: `requestAnimationFrame` irá a 240; la animación debe ir por
    tiempo y topar a 30–60 fps.
  - Ventana transparente sobre NVIDIA (tauri#14924).
- El prototipo `~/code/jarvis/docs/blob-hibrido.html` pide
  `willReadFrequently:true` sin leer píxeles: fuerza raster en CPU en WebKit y en
  Chromium. Quitarlo.

## Alternativas

- **Tauri + CEF** (`tauri-runtime-cef` 3.0.0-alpha.5): solo X11 en Linux, CPU al
  100 % en reposo (#16189), destello blanco (#16205). Descartado.
- **Electron**: Chromium con Ozone Wayland, el render más probado; >100 MB.
- **Slint / iced / egui / GTK4**: nativos y ligeros; la animación de puntos es
  fácil, pero listas, portadas y estilo cuestan bastante más que en web.

## Experimento

1. Copia del prototipo en `/tmp` con medidor de deltas de rAF durante 60 s
   (p50/p95/p99, % de frames > 2× intervalo). Variantes: tal cual, y sin
   `willReadFrequently` con 4× puntos.
2. `/usr/lib/webkit2gtk-4.1/MiniBrowser` (mismo motor que Tauri), 60 s en eDP-2
   y 60 s en DP-1 por pasada: A por defecto con `WEBKIT_SHOW_FPS=1`; B forzando
   la NVIDIA; C con `WEBKIT_DISABLE_DMABUF_RENDERER=1`. GPU en uso:
   `ls -l /proc/<WebKitWebProcess>/fd | grep renderD` y `nvtop`.
3. Chromium `--ozone-platform=wayland` en los mismos monitores, con traza CDP
   de 10 s.
4. Pasa si p95 ≤ 1.25× el periodo objetivo, < 1 % de frames > 2×, sin blanco ni
   Error 71, CPU razonable, y sobrevive a redimensionar y mover entre monitores.

Si A pasa en ambos monitores: Tauri. Si solo B o C: Tauri con esa variable. Si
WebKit falla y Chromium no: Electron.

## Fuentes

- https://v2.tauri.app/develop/debug/linux-graphics/
- https://github.com/tauri-apps/tauri/issues/10702 · /9304 · /14924 · /16189 · /16205
- https://github.com/tauri-apps/wry/issues/1366 · /618
- https://github.com/tauri-apps/tauri/tree/feat/cef/crates/tauri-runtime-cef
- https://webkitgtk.org/2026/03/18/webkitgtk2.52.0-released.html
- https://blogs.igalia.com/carlosgc/?p=1041

## Resultado de la medición (2026-10-06)

Página propia en vez del prototipo (que anima con `setInterval` a 25 fps, no con
rAF): `proto` = rejilla del prototipo (72×60 puntos de 9 px, con
`willReadFrequently`); `app` = lienzo de 200×150 puntos de 6 px sin esa opción,
más una lista de 200 filas desplazándose y una barra animada. rAF libre, 20 s por
pasada, ventana flotante de 1100×800. ws1 = eDP-2 (240 Hz, Intel), ws2 = DP-1
(75 Hz, NVIDIA). wkA = WebKitGTK por defecto, wkB = forzando la NVIDIA, wkC =
sin DMABUF, chr = Chromium 152 Wayland. `>2x%` = frames que tardaron más del
doble de la mediana. "dibujo" = ms de JavaScript por frame.

```
pasada                   fps    p50    p95    p99  >2x% dibujo p95  gpu/cpu
wkA-ws1-proto             55   17.0   29.0   42.0   2.8        4.0  renderD129/20.5%
wkA-ws1-app               57   17.0   24.0   37.0   1.1       10.0  renderD129/47.0%
wkA-ws2-proto             59   17.0   18.0   34.0   0.9        4.0  renderD129/24.7%
wkA-ws2-app               60   17.0   18.0   23.0   0.1        8.0  renderD129/49.1%
wkB-ws1-proto             32   17.0   94.0  159.0  20.7        4.0  renderD128/76.5%
wkB-ws2-proto             62   16.0   17.0   17.0   0.0        4.0  renderD128/31.9%
wkB-ws2-app               58   16.0   26.0   38.0   2.0       13.0  renderD128/60.5%
wkC-ws1-proto             56   17.0   28.0   39.0   2.2        9.0  renderD128/57.2%
wkC-ws1-app               22   40.0   82.0   92.0   5.4       41.0  renderD128/92.8%
wkC-ws2-proto             62   16.0   17.0   17.0   0.0        4.0  renderD128/25.9%
wkC-ws2-app               45   19.0   31.0   60.0   2.5       23.0  renderD128/92.1%
chr-ws1-proto             80   12.5   20.8   25.1   1.3        4.2  renderD128 renderD129/10.9%
chr-ws1-app               68   12.5   37.5   45.8  12.7        9.6  renderD128 renderD129/67.0%
chr-ws2-proto             72   13.3   15.0   28.9   2.7        0.9  renderD128 renderD129/9.3%
chr-ws2-app               60   13.4   29.4   40.1  12.0        6.3  renderD128 renderD129/50.7%
```

wkB-ws1-app no entregó resultado: a mitad de la pasada ya no había proceso web (se cayó o no arrancó).

- WebKit topa rAF a 60 fps incluso en la pantalla de 240 Hz: conviene para la app.
- **wkA (lo que usaría Tauri tal cual) es la mejor opción general**: en DP-1, p95
  18 ms y 0.1 % de saltos con la carga `app`; en la eDP-2, p95 24–29 ms y 1–3 %
  de saltos, justo encima del criterio.
- Chromium no es mejor: en `app` tuvo 12 % de saltos en los dos monitores. Electron
  no compra nada aquí.
- Forzar la NVIDIA (wkB) o quitar DMABUF (wkC) empeora y multiplica la CPU.
- Ningún motor mostró pantalla en blanco ni Error 71.

Decisión: Tauri 2 con WebKitGTK por defecto, sin variables de entorno. Los saltos
en la eDP-2 los tienen también los otros motores; se revisan con la app real.

## Segunda medición: el techo (2026-10-06)

Kevin: "el techo es muy bajo". Pasadas de 15 s en la eDP-2 (240 Hz). `vacio` =
rAF sin dibujar nada; `lote` = los mismos puntos en un Path2D por color en vez
de un arc+fill por punto. chrU = Chromium con `--disable-frame-rate-limit
--disable-gpu-vsync`; chrI = Chromium con `--render-node-override` a la Intel;
wkV = `WEBKIT_FORCE_VBLANK_TIMER=1`; wkT = `WEBKIT_DISPLAY_REFRESH_THROTTLE_FPS=240`.

```
pasada                   fps    p50    p95    p99  >2x% dibujo p95  gpu/cpu
wkA-ws1-vacio             59   17.0   18.0   26.0   0.3        1.0  renderD129/11.4%
wkV-ws1-vacio             58   17.0   20.0   28.0   0.6        1.0  renderD129/11.5%
wkV-ws1-proto             57   17.0   26.0   30.0   0.7        4.0  renderD129/22.7%
chr-ws1-vacio            119    8.3    8.4   12.5   0.2        0.2  renderD128 renderD129/15.7%
chrU-ws1-vacio          2881    0.3    0.9    1.6  16.5        0.1  renderD128 renderD129/141%
chrU-ws1-proto           127    5.0   25.0   30.4  23.7        5.2  renderD128 renderD129/105%
chrU-ws1-app             126    5.9   22.4   33.3  14.7       16.2  renderD128 renderD129/166%
wkT-ws1-vacio             60   17.0   18.0   18.0   0.0        1.0  renderD129/6.7%
wkA-ws1-lote              60   17.0   17.0   17.0   0.0        3.0  renderD129/18.4%
wkA-ws1-loteapp           11   96.0  192.0  272.0   4.5      173.0  renderD129/96.5%
chrI-ws1-vacio           100    8.4   16.7   16.7   0.7        0.1  renderD128 renderD129/10.6%
chrI-ws1-lote             86   12.5   16.7   16.8   0.0        1.4  renderD128 renderD129/24.2%
chrIU-ws1-lote           276    1.3   15.7   35.8  28.1        3.8  renderD128 renderD129/195%
chrIU-ws1-loteapp         80    8.6   30.8   35.6  21.9       26.1  renderD128 renderD129/174%
```

- **WebKitGTK tiene un tope fijo de 60 fps** aquí, aun con la página vacía y con
  las dos variables de refresco. No lo pone la máquina.
- **Chromium se queda en 120 en el panel de 240**, también con la página vacía y
  dibujando con la Intel. Sin vsync llega a 2881 fps vacío: el límite es el
  ritmo entre Chromium y Hyprland, no el cálculo. Causa sin encontrar.
- **El otro techo es la forma de dibujar.** Un arc+fill por punto en canvas 2D es
  CPU pura: ~5 ms para 4k puntos. En lote, 4k puntos dan 60 fps clavados en
  WebKit, pero 30k puntos en un solo Path2D se hunden a 11 fps (173 ms por
  frame). Para miles de puntos lo correcto es la GPU (WebGL o wgpu): todos los
  puntos en una sola llamada de dibujo.

## Por qué WebKitGTK no pasa de 60 (2026-10-06)

- WebKit sí detecta el panel: con `WEBKIT_DISPLAY_REFRESH_THROTTLE_FPS=7` responde
  "not a factor of refresh rate 240fps". El vblank DRM funciona.
- Apagar `PreferPageRenderingUpdatesNear60FPS` (`MiniBrowser
  --features=-PreferPageRenderingUpdatesNear60FPS`) no cambia nada: sigue en 60.
- Es conocido: en WebKitGTK un temporizador limita las actualizaciones de página
  a ~60 antes que esa opción, y no hay ajuste, API ni variable para subirlo. Upstream:
  https://bugs.webkit.org/show_bug.cgi?id=173434 · caso igual:
  https://github.com/Jackicus/GNOME-Turntable/issues/1 · con perfil de ahorro
  de energía baja a la mitad del refresco:
  https://github.com/solutionscay/skiff/issues/7
- Chromium en Wayland toma el intervalo de vsync del primer modo anunciado, no
  del activo (DP-1 anuncia primero 60 Hz, de ahí los ~60–72 ahí). En la eDP-2 el
  primer modo es 240 y aun así dio 120: causa sin encontrar.
  https://forums.blurbusters.com/viewtopic.php?p=68929

**Consecuencia:** con Tauri en Linux la animación nunca pasa de 60 fps. Si se
quiere seguir el refresco del monitor, la opción es Electron (120 medido, falta
explicar por qué no 240) o una app nativa en Rust sobre wgpu, que dibuja al
ritmo de los frame callbacks de Wayland.

## App nativa (iced 0.14 + wgpu): primeras medidas (2026-10-06)

Esqueleto en `app/`: núcleo de Iris en la GPU (una instancia por punto, una llamada de
dibujo), barra de lo que suena, espacio/n/p. Ventana 900×700 flotando en la eDP-2.
`SURCO_FPS=1` imprime fps y ms de CPU. La vista (pintar ~20k puntos) cuesta ~0.9 ms.

| configuración | fps |
|---|---|
| wgpu por defecto (elige la NVIDIA: cada cuadro cruza a la Intel) | 52–120 |
| `WGPU_POWER_PREF=low` (Intel) | ~150 |
| Intel + `ICED_PRESENT_MODE=immediate` | ~90–145, CPU 20 %: no es la app |
| Intel + ventana opaca | ~150 |
| Intel + regla `no_blur` para la ventana | ~190 |
| Intel + **blur de Hyprland apagado** | **240 clavados** |
| `render.new_render_scheduling = true` | 14–29 (peor) |
| blur `xray = true` | ~90–165 (no ayuda) |

El techo restante es el blur de Hyprland componiendo el panel de 2560×1600 con la Intel:
cada cuadro de la animación obliga a recalcular el blur de lo que hay alrededor/detrás
(aquí, un foot transparente en mosaico). Las pruebas de Hyprland se hicieron en caliente
con `hl.config` y se devolvieron a su valor.
