#![allow(dead_code)] // copia completa del núcleo de Iris: no todo se usa aquí
//! NÚCLEO: el blob del panel de arriba a la derecha. Muestra qué está haciendo Jarvis con
//! la forma, el movimiento y una pupila que mira hacia donde pasan las cosas.
//!
//! Copia del núcleo de Iris (~/code/jarvis/src/nucleo.rs) sin lo de la terminal (braille y
//! Sixel): aquí cada punto sale como instancia para la GPU, en `dots`. Hasta que el núcleo viva
//! en un crate compartido, los cambios de diseño se hacen allá y se traen.
//!
//! El diseño se hizo en `docs/blob-hibrido.html`; esto es su versión para la terminal. Se
//! dibuja sobre una rejilla braille propia (2×4 puntos por celda) y no con el `Canvas` de
//! ratatui porque ahí cada celda toma el color del último punto dibujado: acá se queda con el
//! del punto más brillante, que es lo que hace legibles las capas.

use std::f64::consts::{PI, TAU};

/// Lo que en Iris era el color de ratatui: aquí solo hace falta RGB.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Color {
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Booting,
    Sleeping,
    Idle,
    Typing,
    Listening,
    NoVoice,
    Transcribing,
    Thinking,
    Planning,
    Searching,
    Reading,
    Editing,
    Running,
    Testing,
    Git,
    Web,
    Delegating,
    Speaking,
    Asking,
    Compacting,
    Offline,
}

/// Cosas que pasan una vez, encima de cualquier estado.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    /// Una tecla en la entrada: respingo y onda corta.
    Key,
    /// Una herramienta terminó en error: destello rojo, sacudida y púas.
    Error,
    /// Fin de turno: rebote y anillo verde.
    Done,
    /// Esc a mitad de turno: se desinfla de golpe.
    Cancel,
    /// Whisper devolvió una alucinación y se descartó: niega con la mirada.
    Nope,
    /// Volvió un subagente: una gota llega desde la escala y se funde.
    Merge,
    /// Algo se copió al portapapeles: destello y guiño.
    Copy,
    /// Las pruebas pasaron: la escala se pinta de verde.
    Pass,
}

/// Lo que el núcleo necesita saber de la app en cada cuadro.
#[derive(Clone, Default)]
pub struct Signals {
    /// RMS del micrófono.
    pub level: f32,
    /// Contexto usado, 0..1.
    pub ctx: f64,
    /// Mensajes escritos a mitad de turno que esperan su lectura.
    pub queue: usize,
    /// Tareas de TodoWrite: (total, completadas).
    pub todos: (usize, usize),
    /// Subagentes vivos: (id, lo que están haciendo). Cada uno es un hijo del núcleo.
    pub kids: Vec<(u64, State)>,
    /// Segundos sin actividad.
    pub idle: f64,
    /// Menos movimiento (`/calma`).
    pub calm: bool,
    /// Lo que suena en el equipo, 0..1 (cava, como el teclado). En reposo, baila con eso.
    pub music: f32,
}

/// Fracción de la escala donde el medidor de música ya es rojo, como la barra del teclado.
const METER_TOP: f64 = 0.5;

/// Motas de la corona, repartidas en tres capas de profundidad.
const MOTES: usize = 24;

/// Fijos, como en el teclado: escuchando y transcribiendo no dependen del tema.
const YELLOW: [f64; 3] = [255.0, 210.0, 0.0];
const PURPLE: [f64; 3] = [150.0, 0.0, 255.0];
const GREEN: [f64; 3] = [40.0, 235.0, 90.0];
const RED: [f64; 3] = [255.0, 40.0, 40.0];
const WARM: [f64; 3] = [255.0, 170.0, 40.0];

impl State {
    pub fn label(self) -> &'static str {
        use State::*;
        match self {
            Booting => "ARRANCANDO",
            Sleeping => "EN REPOSO",
            Idle => "EN ESPERA",
            Typing => "TE LEO",
            Listening => "ESCUCHANDO",
            NoVoice => "NO TE OIGO",
            Transcribing => "TRANSCRIBIENDO",
            Thinking => "PENSANDO",
            Planning => "PLANIFICANDO",
            Searching => "BUSCANDO",
            Reading => "LEYENDO",
            Editing => "EDITANDO",
            Running => "EJECUTANDO",
            Testing => "PROBANDO",
            Git => "GIT",
            Web => "EN LA RED",
            Delegating => "DELEGANDO",
            Speaking => "RESPONDIENDO",
            Asking => "ESPERANDO RESPUESTA",
            Compacting => "COMPACTANDO",
            Offline => "DESCONECTADO",
        }
    }

    /// Por familias: presencia en el acento del tema, la mente en azules y violetas, las
    /// herramientas del verde al rosa. Escuchar y transcribir, en el amarillo y el morado del
    /// teclado (ghubd). Lo de alrededor (escala, arcos, motas) va siempre en el acento: el
    /// color del cuerpo es el que dice el estado.
    fn rgb(self) -> [f64; 3] {
        use State::*;
        use crate::theme::{accent_darker, accent_rgb, accent_toward_white};
        let c = match self {
            Booting | Idle => return accent_rgb(),
            Sleeping => return accent_darker(0.43),
            Typing => return accent_toward_white(0.35),
            Speaking => return accent_toward_white(0.6),
            Listening => return YELLOW,
            NoVoice => return mix(YELLOW, [0.0; 3], 0.35),
            Transcribing => return PURPLE,
            Asking => [255.0, 205.0, 60.0],
            Thinking => [90.0, 150.0, 255.0],
            Planning => [125.0, 125.0, 255.0],
            Delegating => [175.0, 140.0, 255.0],
            Compacting => [160.0, 165.0, 205.0],
            Searching => [60.0, 215.0, 165.0],
            Reading => [70.0, 220.0, 215.0],
            Editing => [175.0, 235.0, 80.0],
            Testing => [120.0, 235.0, 120.0],
            Running => [235.0, 225.0, 90.0],
            Git => [255.0, 130.0, 70.0],
            Web => [255.0, 110.0, 170.0],
            Offline => return [90.0, 110.0, 125.0],
        };
        apart(c, accent_rgb())
    }

    pub fn color(self) -> Color {
        rgb(self.rgb())
    }

    /// Estos se muestran al instante; el resto espera a que el anterior haya durado su mínimo.
    fn urgent(self) -> bool {
        use State::*;
        matches!(self, Listening | NoVoice | Transcribing | Asking | Offline)
    }

    /// Lo mínimo que se muestra un estado, para que un Read de 50 ms no sea un parpadeo.
    fn dwell(self) -> f64 {
        use State::*;
        match self {
            Planning => 1.4,
            Searching | Reading | Editing | Running | Testing | Git | Web | Delegating => 0.6,
            Thinking => 0.35,
            _ => 0.25,
        }
    }

    fn is_tool(self) -> bool {
        use State::*;
        matches!(self, Planning | Searching | Reading | Editing | Running | Testing | Git | Web | Delegating)
    }

    fn params(self) -> Params {
        use State::*;
        let b = BASE;
        match self {
            Booting => Params { boot: 1.0, arc_speed: 0.5, ..b },
            Sleeping => Params { blob_r: 0.34, amp: 0.02, wob: 0.12, arcs: 0.0, pupil: 0.0, zzz: 1.0, motes: 0.3, ..b },
            Idle => b,
            Typing => Params { amp: 0.04, wob: 0.5, arc_speed: 0.4, ..b },
            Listening => Params { amp: 0.05, wob: 0.8, chaos: 0.5, arc_speed: 0.6, arc_len: 0.8, vu: 1.0, dilate: 1.3, ..b },
            NoVoice => Params { amp: 0.025, wob: 0.3, arc_speed: 0.15, arc_len: 0.6, droop: 1.0, dilate: 1.4, ..b },
            Transcribing => Params {
                blob_r: 0.32, amp: 0.05, wob: 3.0, chaos: 0.9, arc_speed: 2.2, arc_len: 0.6, fill: 1.0, ..b
            },
            Thinking => Params { amp: 0.10, wob: 0.7, chaos: 0.3, arc_speed: 1.1, orbit: 1.0, dilate: 0.75, ..b },
            Planning => Params { amp: 0.04, wob: 0.5, arc_speed: 0.5, plan: 1.0, ..b },
            Searching => Params { amp: 0.03, wob: 0.5, arc_speed: 0.8, lens: 1.0, ..b },
            Reading => Params { amp: 0.03, wob: 0.4, arc_speed: 0.7, scan: 1.0, ..b },
            Editing => Params { amp: 0.05, wob: 0.9, arc_speed: 1.2, arc_len: 0.7, stitch: 1.0, ..b },
            Running => Params { blob_r: 0.38, amp: 0.08, wob: 2.0, chaos: 0.4, arc_speed: 2.4, arc_len: 0.7, sweep: 1.0, ..b },
            Testing => Params { amp: 0.04, wob: 1.0, arc_speed: 1.4, arc_len: 0.6, tests: 1.0, ..b },
            Git => Params { blob_r: 0.34, amp: 0.04, wob: 0.6, arc_speed: 0.6, git: 1.0, ..b },
            Web => Params { blob_r: 0.36, amp: 0.05, wob: 1.0, arc_speed: 0.8, packets: 1.0, ..b },
            Delegating => Params { blob_r: 0.36, amp: 0.06, wob: 0.8, arc_speed: 0.9, bud: 1.0, ..b },
            Speaking => Params { amp: 0.05, wob: 1.0, chaos: 0.2, arc_speed: 0.9, ripple: 1.0, dilate: 1.1, ..b },
            Asking => Params { blob_r: 0.38, amp: 0.03, wob: 0.3, arc_speed: 0.12, cardinal: 1.0, bob: 1.0, ..b },
            Compacting => Params { blob_r: 0.36, amp: 0.03, wob: 0.6, arc_speed: 1.6, arc_len: 0.5, press: 1.0, ..b },
            Offline => Params {
                blob_r: 0.30, amp: 0.015, wob: 0.0, chaos: 0.0, arc_speed: 0.0, arcs: 0.0, dashed: 1.0, pupil: 0.0,
                motes: 0.0, ..b
            },
        }
    }
}

/// Qué estado corresponde a una herramienta. `detail` es la primera línea de su entrada
/// (el comando, en el caso de Bash).
pub fn tool_state(name: &str, detail: &str) -> State {
    match name {
        "Read" | "NotebookRead" => State::Reading,
        "Grep" | "Glob" | "LS" => State::Searching,
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => State::Editing,
        "WebFetch" | "WebSearch" => State::Web,
        "Agent" | "Task" => State::Delegating,
        "TodoWrite" => State::Planning,
        "Bash" => bash_state(detail),
        _ => State::Running,
    }
}

fn bash_state(cmd: &str) -> State {
    const TESTS: &[&str] = &[
        "cargo test", "cargo nextest", "pytest", "npm test", "npm run test", "pnpm test", "pnpm run test",
        "yarn test", "bun test", "vitest", "jest", "go test", "make test", "deno test",
    ];
    // Cada tramo de `a && b; c | d`, sin variables de entorno delante.
    let parts = cmd.split(['&', ';', '|']).map(|p| {
        p.split_whitespace().skip_while(|w| w.contains('=') && !w.starts_with('-')).collect::<Vec<_>>().join(" ")
    });
    let mut state = State::Running;
    for p in parts {
        if TESTS.iter().any(|t| p == *t || p.starts_with(&format!("{t} ")) || p.contains(&format!(" {t}"))) {
            return State::Testing;
        }
        if p == "git" || p.starts_with("git ") {
            state = State::Git;
        }
    }
    state
}

#[derive(Clone, Copy)]
struct Params {
    blob_r: f64,
    amp: f64,
    wob: f64,
    chaos: f64,
    arc_speed: f64,
    arcs: f64,
    arc_len: f64,
    ripple: f64,
    orbit: f64,
    sweep: f64,
    fill: f64,
    vu: f64,
    cardinal: f64,
    dashed: f64,
    scan: f64,
    stitch: f64,
    packets: f64,
    bud: f64,
    zzz: f64,
    pupil: f64,
    bob: f64,
    motes: f64,
    boot: f64,
    lens: f64,
    plan: f64,
    tests: f64,
    git: f64,
    press: f64,
    droop: f64,
    /// Apertura de la pupila: >1 dilatada (escucha, busca), <1 contraída (concentrado).
    dilate: f64,
}

const BASE: Params = Params {
    blob_r: 0.40,
    amp: 0.035,
    wob: 0.35,
    chaos: 0.1,
    arc_speed: 0.25,
    arcs: 1.0,
    arc_len: 1.0,
    ripple: 0.0,
    orbit: 0.0,
    sweep: 0.0,
    fill: 0.0,
    vu: 0.0,
    cardinal: 0.0,
    dashed: 0.0,
    scan: 0.0,
    stitch: 0.0,
    packets: 0.0,
    bud: 0.0,
    zzz: 0.0,
    pupil: 1.0,
    bob: 0.0,
    motes: 1.0,
    boot: 0.0,
    lens: 0.0,
    plan: 0.0,
    tests: 0.0,
    git: 0.0,
    press: 0.0,
    droop: 0.0,
    dilate: 1.0,
};

impl Params {
    fn approach(&mut self, to: &Params, k: f64) {
        macro_rules! go {
            ($($f:ident),*) => { $( self.$f += (to.$f - self.$f) * k; )* };
        }
        go!(blob_r, amp, wob, chaos, arc_speed, arcs, arc_len, ripple, orbit, sweep, fill, vu, cardinal, dashed,
            scan, stitch, packets, bud, zzz, pupil, bob, motes, boot, lens, plan, tests, git, press, droop, dilate);
    }
}

/// Un subagente: un ojito que orbita al principal, unido a él por una línea por la que
/// suben partículas. Nace del cuerpo, hace lo suyo con la pupila y vuelve a fundirse.
struct Kid {
    id: u64,
    /// Ángulo en la órbita; se reparte con los demás hijos vivos.
    a: f64,
    born: f64,
    /// Cuándo terminó y si salió bien: bien vuelve y se funde, mal se apaga en rojo donde está.
    end: Option<(f64, bool)>,
    merged: bool,
    /// El hijo es un núcleo entero en miniatura: los mismos estados, colores y detalles que el
    /// principal, sin lo de alrededor (escala, arcos, motas).
    core: Box<Core>,
    /// Partículas que suben por la línea: cuándo salieron y si llevan un error.
    pulses: Vec<(f64, bool)>,
}

/// Radio de la órbita de los hijos: entre el cuerpo y la escala.
const KID_ORBIT: f64 = 0.70;
/// Lo que tarda un hijo en salir del cuerpo y en volver a él.
const KID_TRAVEL: f64 = 0.8;
/// Lo que tarda una partícula en subir del hijo al principal.
const PULSE_TRAVEL: f64 = 0.65;

#[derive(Clone, Copy, PartialEq)]
enum Gesture {
    Stretch,
    LookClock,
    LookChat,
    Yawn,
}

#[derive(Clone, Copy)]
struct Mote {
    a: f64,
    r: f64,
    s: f64,
    /// Profundidad: 0 al fondo, 1 al frente. Lo cercano brilla más, es más grande, va más
    /// rápido y se desplaza más con la mirada: eso es lo que da el paralaje.
    z: f64,
}

pub struct Core {
    t: f64,
    shown: State,
    since: f64,
    p: Params,
    col: [f64; 3],
    phase: f64,
    arc_phase: f64,
    /// Aplastamiento elástico (resorte) y su velocidad.
    sq: f64,
    sq_v: f64,
    shake: f64,
    red: f64,
    deflate: f64,
    nope: f64,
    copy: f64,
    pass: f64,
    /// Al pasar de escuchar a transcribir, las motas se tragan hacia el centro.
    swallow: f64,
    blooms: Vec<f64>,
    keys: Vec<f64>,
    merges: Vec<(f64, f64, bool)>,
    gaze: [f64; 2],
    gaze_t: [f64; 2],
    next_saccade: f64,
    gesture: Option<(Gesture, f64)>,
    next_gesture: f64,
    motes: Vec<Mote>,
    kids: Vec<Kid>,
    /// Hacia dónde mira el principal cuando un hijo le manda algo: (cuándo, ángulo).
    heard_kid: Option<(f64, f64)>,
    level: f64,
    /// Nivel de la música y cuánto está bailando (0..1: entra en medio segundo, se va de golpe).
    music: f64,
    groove: f64,
    sig: Signals,
    rng: u64,
    fired: Vec<Event>,
    /// Es un hijo: sin escala, arcos ni motas, que en miniatura solo ensucian.
    mini: bool,
    /// Si está puesto, la pupila mira hacia ahí (−1..1) en vez de lo que diga el estado: el
    /// hijo mira al principal cuando le manda algo.
    look: Option<[f64; 2]>,
}

impl Core {
    pub fn new() -> Self {
        let mut c = Core {
            t: 0.0,
            shown: State::Booting,
            since: 0.0,
            p: State::Booting.params(),
            col: State::Booting.rgb(),
            phase: 0.0,
            arc_phase: 0.0,
            sq: 0.0,
            sq_v: 0.0,
            shake: 0.0,
            red: 0.0,
            deflate: 0.0,
            nope: 0.0,
            copy: 0.0,
            pass: 0.0,
            swallow: 0.0,
            blooms: vec![],
            keys: vec![],
            merges: vec![],
            gaze: [0.0; 2],
            gaze_t: [0.0; 2],
            next_saccade: 0.0,
            gesture: None,
            next_gesture: 20.0,
            motes: vec![],
            kids: vec![],
            heard_kid: None,
            level: 0.0,
            music: 0.0,
            groove: 0.0,
            sig: Signals::default(),
            rng: 0x9E37_79B9_7F4A_7C15,
            fired: vec![],
            mini: false,
            look: None,
        };
        c.motes = (0..MOTES)
            .map(|k| {
                let z = ((k % 3) as f64 / 2.0 + (c.rand() - 0.5) * 0.2).clamp(0.0, 1.0);
                let (lo, hi) = mote_band(z, 0.40);
                Mote { a: k as f64 * TAU / MOTES as f64 + c.rand(), r: lo + c.rand() * (hi - lo), s: c.rand() - 0.5, z }
            })
            .collect();
        c
    }

    /// Un hijo: ya despierto en `state`, sin arranque ni motas.
    fn mini(state: State, seed: u64) -> Self {
        let mut c = Core::new();
        c.mini = true;
        c.motes.clear();
        c.shown = state;
        c.p = state.params();
        c.col = state.rgb();
        c.rng ^= seed.wrapping_mul(0x2545_F491_4F6C_DD1D) | 1;
        // Cada hijo con su fase, para que no respiren ni parpadeen todos a la vez.
        c.t = 3.7 * seed as f64;
        c.since = c.t;
        c
    }

    /// El estado que se ve, que puede ir un poco detrás del pedido (ver `State::dwell`).
    pub fn state(&self) -> State {
        self.shown
    }

    /// El color actual, con las transiciones y el rojo de los errores.
    pub fn color(&self) -> Color {
        rgb(self.col)
    }

    /// Reinicia la animación de arranque (al relanzar el motor).
    pub fn reboot(&mut self) {
        self.kids_clear();
        self.switch(State::Booting);
    }

    /// Nace un subagente: brota del cuerpo hacia su lugar en la órbita.
    pub fn kid_born(&mut self, id: u64) {
        // Sale por donde va a quedar: el hueco que deja el reparto con uno más.
        let alive = self.kids.iter().filter(|k| k.end.is_none()).count();
        let a = kid_slot(self.t, alive, alive + 1, self.calm());
        let t = self.t;
        self.kids.push(Kid {
            id,
            a,
            born: t,
            end: None,
            merged: false,
            core: Box::new(Core::mini(State::Thinking, id)),
            pulses: vec![],
        });
        self.kick(1.6);
    }

    /// El hijo hizo algo (pidió una herramienta o le llegó su resultado): una partícula sube.
    pub fn kid_pulse(&mut self, id: u64, error: bool) {
        let t = self.t;
        if let Some(k) = self.kids.iter_mut().find(|k| k.id == id) {
            k.pulses.push((t, error));
            k.core.fire(if error { Event::Error } else { Event::Key });
        }
    }

    /// Terminó el subagente: si salió bien vuelve al cuerpo; si no, se apaga en rojo.
    pub fn kid_end(&mut self, id: u64, ok: bool) {
        let t = self.t;
        if let Some(k) = self.kids.iter_mut().find(|k| k.id == id && k.end.is_none()) {
            k.end = Some((t, ok));
            if !ok {
                k.core.fire(Event::Error);
            }
        }
    }

    /// El motor se fue: los hijos se van con él, sin ceremonia.
    pub fn kids_clear(&mut self) {
        self.kids.clear();
        self.heard_kid = None;
    }

    /// Cuántos hijos hay (vivos o despidiéndose).
    pub fn kid_count(&self) -> usize {
        self.kids.len()
    }

    fn rand(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    fn calm(&self) -> f64 {
        if self.sig.calm { 0.4 } else { 1.0 }
    }

    fn kick(&mut self, v: f64) {
        self.sq_v += v * self.calm();
    }

    /// Los eventos disparados desde la última vez, para avisarle al teclado.
    pub fn take_fired(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.fired)
    }

    pub fn fire(&mut self, ev: Event) {
        self.fired.push(ev);
        let t = self.t;
        match ev {
            Event::Key => {
                self.keys.push(t);
                self.kick(1.2);
            }
            Event::Error => {
                self.red = 1.0;
                self.shake = if self.sig.calm { 0.0 } else { 1.0 };
                self.kick(-2.5);
            }
            Event::Done => {
                self.blooms.push(t);
                self.kick(3.2);
            }
            Event::Cancel => {
                self.deflate = 1.0;
                self.kick(-3.0);
            }
            Event::Nope => self.nope = 1.0,
            Event::Merge => {
                let a = self.rand() * TAU;
                self.merges.push((t, a, false));
            }
            Event::Copy => {
                self.copy = 1.0;
                self.kick(0.8);
            }
            Event::Pass => {
                self.pass = 1.0;
                self.kick(1.5);
            }
        }
    }

    fn switch(&mut self, to: State) {
        use State::*;
        let from = self.shown;
        self.shown = to;
        self.since = self.t;
        // Transiciones con intención: cada cambio tiene su gesto.
        match (from, to) {
            (_, Offline) => self.kick(-1.5),
            (Sleeping, _) => {
                self.kick(3.0);
                self.copy = 0.5; // parpadea al despertar
            }
            (Listening | NoVoice, Transcribing) => self.swallow = 1.0,
            (f, Thinking) if f.is_tool() => self.kick(-0.8),
            (_, t) if t.is_tool() => self.kick(1.2),
            _ => self.kick(1.0),
        }
        self.gesture = None;
    }

    /// Avanza la animación `dt` segundos hacia el estado `want`.
    pub fn step(&mut self, dt: f64, want: State, sig: &Signals) {
        let dt = dt.clamp(0.0, 0.1);
        self.t += dt;
        self.sig = sig.clone();
        let t = self.t;
        let shown_for = t - self.since;
        if want != self.shown && (want.urgent() || self.shown.urgent() || shown_for >= self.shown.dwell()) {
            self.switch(want);
        }
        let st = t - self.since;
        let calm = self.calm();

        let k = 1.0 - (-dt / 0.35).exp();
        let mut target = self.shown.params();
        // Con hijos, el principal se hace un poco a un lado para dejarles la órbita.
        // y los arcos se apagan un poco para que la red se lea.
        if self.kids.iter().any(|k| k.end.is_none()) {
            target.blob_r *= 0.72;
            target.arcs *= 0.45;
            target.motes *= 0.5;
        }
        self.p.approach(&target, k);
        let base = self.shown.rgb();
        let want_col = if self.red > 0.05 { mix(base, RED, (self.red * 1.4).min(1.0)) } else { base };
        let kc = if self.red > 0.05 { 0.5 } else { k };
        for i in 0..3 {
            self.col[i] += (want_col[i] - self.col[i]) * kc;
        }

        // Resorte del aplastamiento: rebota y se asienta.
        self.sq_v += (-140.0 * self.sq - 9.0 * self.sq_v) * dt;
        self.sq = (self.sq + self.sq_v * dt * 0.06).clamp(-0.18, 0.18);

        for v in [&mut self.red, &mut self.nope] {
            *v = (*v - dt * 1.1).max(0.0);
        }
        self.shake = (self.shake - dt * 1.6).max(0.0);
        self.deflate = (self.deflate - dt * 1.4).max(0.0);
        self.copy = (self.copy - dt * 3.0).max(0.0);
        self.pass = (self.pass - dt * 1.2).max(0.0);
        self.swallow = (self.swallow - dt * 1.5).max(0.0);
        self.blooms.retain(|b| t - b < 1.2);
        self.keys.retain(|k0| t - k0 < 0.5);
        let mut pops = 0;
        for m in &mut self.merges {
            if t - m.0 > 0.7 && !m.2 {
                m.2 = true;
                pops += 1;
            }
        }
        for _ in 0..pops {
            self.kick(2.4);
            self.blooms.push(t);
        }
        self.merges.retain(|m| t - m.0 < 1.0);
        self.step_kids(dt);

        self.phase += dt * self.p.wob * calm;
        self.arc_phase += dt * self.p.arc_speed * calm;

        let lvl = if matches!(self.shown, State::Listening | State::Speaking) { (sig.level as f64 * 6.0).min(1.0) } else { 0.0 };
        self.level += (lvl - self.level) * if lvl > self.level { 0.5 } else { 0.12 };
        // Música: cava ya viene suavizado, así que se sigue casi tal cual. Como en el teclado,
        // aparece en medio segundo y una tecla (o cualquier otro estado) lo apaga al instante.
        let m = sig.music as f64;
        // Un golpe (el nivel sube de golpe) lo aplasta un poco: así se ve que baila.
        if m - self.music > 0.15 {
            self.kick(1.5 * self.groove);
        }
        self.music += (m - self.music) * (dt * 30.0).min(1.0);
        self.groove = if self.shown != State::Idle {
            (self.groove - dt * 8.0).max(0.0)
        } else if m > 0.02 {
            (self.groove + dt / 0.5).min(1.0)
        } else {
            (self.groove - dt).max(0.0) // entre canción y canción se apaga despacio
        };

        // Gestos ocasionales cuando lleva rato quieto, para que no repita siempre lo mismo.
        if self.shown == State::Idle && self.groove < 0.05 && sig.idle > 15.0 && t > self.next_gesture && self.gesture.is_none() {
            let g = if sig.idle > 95.0 {
                Gesture::Yawn
            } else {
                [Gesture::Stretch, Gesture::LookClock, Gesture::LookChat][(self.rand() * 3.0) as usize % 3]
            };
            if g == Gesture::Stretch {
                self.kick(3.5);
            }
            self.gesture = Some((g, t));
            self.next_gesture = t + 8.0 + self.rand() * 8.0;
        }
        if let Some((g, t0)) = self.gesture {
            let len = match g {
                Gesture::Stretch => 0.8,
                Gesture::LookClock => 1.6,
                Gesture::LookChat => 1.2,
                Gesture::Yawn => 2.2,
            };
            if t - t0 > len || self.shown != State::Idle {
                self.gesture = None;
            }
        }

        // Mirada: salto rápido hacia el objetivo, como una sacada.
        let gt = self.gaze_target(st);
        let gt = if self.nope > 0.01 {
            [self.p.blob_r * 0.4 * (t * 22.0).sin() * (self.nope * 2.0).min(1.0), gt[1]]
        } else {
            gt
        };
        let kg = (dt * 14.0).min(1.0);
        for i in 0..2 {
            self.gaze[i] += (gt[i] - self.gaze[i]) * kg;
        }

        // Motas: caen al centro escuchando, salen respondiendo, flotan el resto.
        let r0 = self.p.blob_r;
        let (shown, level, swallow, groove) = (self.shown, self.level, self.swallow, self.groove * self.music);
        for m in &mut self.motes {
            if swallow > 0.01 {
                m.r += (r0 - m.r) * (dt * 6.0).min(1.0);
                if m.r < r0 + 0.03 {
                    m.r = 0.86;
                }
            } else if shown == State::Listening {
                m.r -= dt * (0.18 + level * 0.5) * mote_speed(m.z);
                if m.r < r0 + 0.08 {
                    m.r = mote_band(m.z, r0).1;
                }
            } else if shown == State::Speaking {
                m.r += dt * 0.22 * mote_speed(m.z);
                if m.r > mote_band(m.z, r0).1 {
                    m.r = r0 + 0.1;
                }
            } else {
                let (lo, hi) = mote_band(m.z, r0);
                m.a += dt * 0.12 * (1.0 + m.s * 0.5) * mote_speed(m.z) * calm * (1.0 + groove * 4.0);
                m.r += (t * 0.7 + m.a * 3.0).sin() * dt * 0.02;
                m.r = m.r.clamp(lo, hi);
            }
        }
    }

    /// Los hijos: su estado, su lugar en la órbita, su mirada y su despedida.
    fn step_kids(&mut self, dt: f64) {
        let t = self.t;
        let calm = self.calm();
        let alive: Vec<usize> = (0..self.kids.len()).filter(|&i| self.kids[i].end.is_none()).collect();
        let n = alive.len();
        for (slot, &i) in alive.iter().enumerate() {
            let want = kid_slot(t, slot, n, calm);
            let k = &mut self.kids[i];
            let d = (want - k.a + PI).rem_euclid(TAU) - PI;
            k.a += d * (dt * 2.5).min(1.0);
        }
        let child = Signals { calm: self.sig.calm, ..Default::default() };
        let mut merged = vec![];
        for k in &mut self.kids {
            let want = match self.sig.kids.iter().find(|(id, _)| *id == k.id) {
                Some(&(_, st)) if k.end.is_none() => st,
                _ => k.core.state(),
            };
            k.pulses.retain(|p| t - p.0 < PULSE_TRAVEL);
            // Justo al mandar algo, o al volver, mira al principal (que está hacia el centro).
            let fresh = k.pulses.last().is_some_and(|p| t - p.0 < 0.3);
            k.core.look = (fresh || k.end.is_some()).then(|| [-0.8 * k.a.cos(), -0.8 * k.a.sin()]);
            k.core.step(dt, want, &child);
            k.core.take_fired();
            if let Some((t0, true)) = k.end {
                if !k.merged && t - t0 >= KID_TRAVEL {
                    k.merged = true;
                    merged.push(k.a);
                }
            }
        }
        // Cada partícula que llega al principal le hace mirar hacia ese hijo.
        for k in &self.kids {
            if k.pulses.iter().any(|p| (t - p.0 - PULSE_TRAVEL).abs() < dt.max(0.02)) {
                self.heard_kid = Some((t, k.a));
            }
        }
        for a in merged {
            self.kick(2.4);
            self.blooms.push(t);
            self.heard_kid = Some((t, a));
        }
        self.kids.retain(|k| match k.end {
            Some((t0, true)) => t - t0 < KID_TRAVEL + 0.05,
            Some((t0, false)) => t - t0 < 1.4,
            None => true,
        });
    }

    /// La «cámara» se mece despacio y sigue un poco a la mirada.
    fn camera(&self) -> [f64; 2] {
        let c = self.calm();
        [
            0.12 * (self.t * 0.23).sin() * c - self.gaze[0] * 0.8,
            0.07 * (self.t * 0.17 + 1.0).sin() * c - self.gaze[1] * 0.8,
        ]
    }

    /// Dónde cae una mota: lo cercano se corre mucho con la cámara, lo lejano casi nada.
    fn mote_pos(&self, m: &Mote, cam: [f64; 2], press: f64, t: f64) -> (f64, f64) {
        let r = m.r * (1.0 - press * 0.25 * (1.0 + (t * 5.0).sin()));
        let shift = 0.1 + m.z * m.z * 1.9;
        (r * m.a.cos() + cam[0] * shift, r * m.a.sin() + cam[1] * shift)
    }

    fn gaze_target(&mut self, st: f64) -> [f64; 2] {
        use State::*;
        let t = self.t;
        let lim = self.p.blob_r * 0.42;
        if let Some([x, y]) = self.look {
            return [lim * x, lim * y];
        }
        if let Some((g, _)) = self.gesture {
            match g {
                // El reloj está arriba a la derecha de la pantalla; el chat, a la izquierda.
                Gesture::LookClock => return [lim * 0.8, lim * 0.85],
                Gesture::LookChat => return [-lim * 0.9, 0.0],
                _ => {}
            }
        }
        // Un hijo le mandó algo: lo mira un momento, salvo que esté ocupado con vos.
        if let Some((t0, a)) = self.heard_kid {
            if t - t0 < 0.7 && matches!(self.shown, Idle | Thinking | Delegating | Speaking | Reading | Searching) {
                return [lim * 0.9 * a.cos(), lim * 0.9 * a.sin()];
            }
        }
        match self.shown {
            Idle => {
                if t > self.next_saccade {
                    let a = self.rand() * TAU;
                    let r = self.rand() * lim;
                    self.gaze_t = [r * a.cos(), r * a.sin() * 0.8];
                    self.next_saccade = t + 1.2 + self.rand() * 2.5;
                }
                self.gaze_t
            }
            Typing => [-lim * 0.6, -lim * 0.8],
            Listening => [0.0, -lim * 0.7],
            NoVoice => [lim * 0.9 * (t * 1.4).sin().signum(), -lim * 0.2],
            Transcribing => [0.02 * (t * 30.0).sin(), 0.02 * (t * 27.0).cos()],
            Thinking => [lim * 0.5 * (t * 0.7).sin(), lim * 0.75],
            Reading => {
                let line = st * 0.9;
                [-lim * 0.8 + line.fract() * lim * 1.6, lim * 0.6 - (line.floor() % 4.0) * lim * 0.4]
            }
            Searching => [self.p.blob_r * 0.55 * (st * 1.3).sin(), self.p.blob_r * 0.45 * (st * 2.1 + 1.0).sin()],
            Planning => {
                let (n, d) = self.sig.todos;
                let a = if n > 0 { PI / 2.0 - d.min(n - 1) as f64 * TAU / n as f64 } else { PI / 2.0 };
                [lim * 0.9 * a.cos(), lim * 0.9 * a.sin()]
            }
            Editing => [lim * 0.6 * (t * 2.6).cos(), lim * 0.6 * (t * 2.6).sin()],
            Running => [0.0, -lim * 0.75],
            Testing => {
                let a = PI / 2.0 - (st * 9.0 % 24.0) * TAU / 24.0;
                [lim * 0.8 * a.cos(), lim * 0.8 * a.sin()]
            }
            Git => [lim * 0.3, lim * 0.9],
            Web => [lim * 0.7 * (t * 0.8).cos(), lim * 0.7 * (t * 0.8).sin()],
            Delegating => {
                let a = self.phase * 0.9;
                [lim * 0.8 * a.cos(), lim * 0.8 * a.sin()]
            }
            Speaking => [-lim * 0.8, 0.0],
            Asking => [-lim * 0.4, -lim * 0.8],
            _ => [0.0, 0.0],
        }
    }

    /// Colores por tinta (estado, verde, rojo, ámbar, acento y uno por hijo) y tono
    /// (1 apagado … 3 pleno).
    fn palette(&self) -> Vec<[[f64; 3]; 4]> {
        let dim = crate::theme::dim_rgb();
        [self.col, GREEN, RED, WARM, crate::theme::accent_rgb()]
            .into_iter()
            .chain(self.kids.iter().take(MAX_KID_INKS).map(|k| k.core.col))
            .map(|c| [dim, dim, mix(c, dim, 0.38), c])
            .collect()
    }

    /// Los puntos del núcleo en una rejilla de `dw`×`dh`: posición (0..1 del área) y color.
    /// Bloques de ~3×3 puntos para lo de atrás, como hacía la imagen Sixel.
    pub fn dots(&self, dw: usize, dh: usize) -> Vec<([f32; 2], [f32; 3])> {
        let (dw, dh) = (dw.max(8), dh.max(8));
        let mut g = Grid::with_dots(dw, dh, 3, 3);
        self.paint(&mut g);
        let pal = self.palette();
        let mut out = Vec::new();
        for j in 0..dh {
            for i in 0..dw {
                let d = g.dots[j * dw + i];
                if d & 3 == 0 {
                    continue;
                }
                let c = pal[(d >> 2) as usize][(d & 3) as usize];
                let pos = [(i as f32 + 0.5) / dw as f32, (j as f32 + 0.5) / dh as f32];
                out.push((pos, [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]));
            }
        }
        out
    }

    fn paint(&self, g: &mut Grid) {
        use State::*;
        let p = &self.p;
        let t = self.t;
        let st = t - self.since;
        let lv = self.level;
        let low = g.dh as f64 * g.k < 40.0; // panel (o hijo) chico: se apagan los detalles finos
        let err = if self.red > 0.35 { INK_RED } else { INK_MAIN };
        // Lo de alrededor va en el acento; con un error, en rojo como todo.
        let ring = if self.red > 0.35 { INK_RED } else { INK_ACCENT };

        // Centro del cuerpo: deriva suave + flotar (preguntando) + sacudida (error) − encorvado.
        let cx = 0.025 * (t * 0.37).sin() + 0.015 * (t * 0.91 + 1.0).sin() + self.shake * 0.05 * (t * 48.0).sin();
        let cy = 0.02 * (t * 0.29 + 2.0).sin() + p.bob * 0.05 * (t * 2.2).sin() - p.droop * 0.08;
        let boot = if p.boot > 0.01 { (st / 2.4).min(1.0) } else { 1.0 };
        let grow = if p.boot > 0.01 { ease_out_back((st / 2.0).min(1.0)) } else { 1.0 };
        let body = |r: f64, a: f64| (cx + r * a.cos(), cy + r * a.sin());

        // Escala: 24 marcas, ancladas al marco. Los hijos no tienen.
        for k in 0..if self.mini { 0 } else { 24 } {
            if k as f64 / 24.0 > boot * 1.05 {
                continue;
            }
            let a = PI / 2.0 - k as f64 * TAU / 24.0;
            let (mut tone, mut ink) = (1, ring);
            if p.vu > 0.01 && (k as f64 / 24.0) < lv * 1.15 * p.vu {
                tone = 3;
            }
            if p.fill > 0.01 && k as f64 / 24.0 <= (st * 0.35).fract() * p.fill {
                tone = 3;
            }
            if p.sweep > 0.01 {
                let d = (a + self.arc_phase * 1.3).rem_euclid(TAU);
                if d < 0.7 * p.sweep {
                    tone = if d < 0.25 { 3 } else { 2 };
                }
            }
            if p.cardinal > 0.01 && k % 6 == 0 && (t * 4.0).sin() > 0.0 {
                tone = 3;
            }
            // Música: el medidor de la barra del teclado, llenándose desde abajo hacia los dos
            // lados, en el acento y rojo cuando el golpe pasa de la mitad.
            if self.groove > 0.01 {
                let from = (k as f64 - 12.0).abs() / 12.0;
                if from <= self.music * self.groove {
                    tone = 3;
                    ink = if from < METER_TOP * 0.6 { INK_ACCENT } else { INK_RED };
                }
            }
            if p.tests > 0.5 && (k as f64) < (st * 9.0) % 25.0 {
                tone = 3; // avance de la corrida; el resultado lo dicen Pass o Error
            }
            for b in &self.blooms {
                let u = (t - b) / 1.1;
                if u > 0.45 && u < 1.0 && ((k as f64 / 24.0 - (u - 0.45) * 1.8).rem_euclid(1.0)) < 0.08 {
                    tone = 3;
                    ink = INK_GREEN;
                }
            }
            if self.pass > 0.3 {
                tone = 3;
                ink = INK_GREEN;
            }
            if self.copy > 0.5 {
                tone = 3;
            }
            if self.red > 0.35 {
                tone = tone.max(2);
            }
            g.polar(0.94, a, tone, ink);
            g.polar(0.90, a, tone, ink);
            if k % 6 == 0 && !low {
                g.polar(0.86, a, tone, ink);
            }
        }

        // Tareas (planificando): una marca grande por tarea; la actual parpadea.
        let (n_todo, done) = self.sig.todos;
        if p.plan > 0.5 && n_todo > 0 {
            for i in 0..n_todo.min(24) {
                let a = PI / 2.0 - i as f64 * TAU / n_todo.min(24) as f64;
                let tone = if i < done {
                    3
                } else if i == done && (t * 8.0).sin() > 0.0 {
                    3
                } else {
                    1
                };
                g.polar(0.82, a, tone, INK_MAIN);
                g.polar(0.78, a, tone, INK_MAIN);
            }
        }

        // Arcos: tres por fuera, seis por dentro, en sentidos opuestos.
        if p.arcs * boot > 0.05 && !self.mini {
            let n1 = (60.0 * p.arc_len * p.arcs * (boot * 1.4 - 0.4).max(0.0)).round() as usize;
            for k in 0..3 {
                for q in 0..n1 {
                    let a = self.arc_phase + k as f64 * TAU / 3.0 + (TAU / 5.0) * p.arc_len * q as f64 / n1.max(1) as f64;
                    g.polar(0.80, a, 2, ring);
                }
            }
            let n2 = (14.0 * p.arcs).round() as usize;
            for k in 0..6 {
                for q in 0..n2 {
                    let a = -self.arc_phase * 1.6 + k as f64 * TAU / 6.0 + (TAU / 16.0) * q as f64 / 14.0;
                    g.polar(0.70, a, 1, ring);
                }
            }
        }

        let cam = self.camera();

        // Mensajes en cola: satélites sólidos que esperan su turno.
        for k in 0..self.sig.queue.min(6) {
            let a = PI / 2.0 + t * 0.4 + k as f64 * 0.5;
            for (dr, da) in [(0.0, 0.0), (0.03, 0.0), (0.0, 0.06), (0.03, 0.06)] {
                let (x, y) = body(0.6 + dr, a + da);
                g.plot(x, y, 3, INK_MAIN);
            }
        }

        // Prensas (compactando).
        if p.press > 0.01 {
            let gap = p.blob_r + 0.08 + 0.1 * (0.5 + 0.5 * (t * 5.0).cos());
            let mut x = -0.35;
            while x <= 0.35 {
                g.plot(cx + x, cy + gap * p.press + (1.0 - p.press) * 0.8, 3, INK_MAIN);
                g.plot(cx + x, cy - gap * p.press - (1.0 - p.press) * 0.8, 3, INK_MAIN);
                x += 0.03;
            }
        }

        // Ondas de voz (respondiendo).
        if p.ripple > 0.01 {
            for w in 0..2 {
                let u = (t * 0.55 + w as f64 / 2.0).fract();
                if u > 0.9 {
                    continue;
                }
                let r = p.blob_r + 0.1 + u * (0.62 - p.blob_r);
                let n = (40.0 + u * 30.0) as usize;
                for q in 0..n {
                    let (x, y) = body(r, q as f64 * TAU / n as f64 + w as f64 * 0.2);
                    g.plot(x, y, if u < 0.4 { 2 } else { 1 }, INK_MAIN);
                }
            }
        }
        // Ondas de tecla.
        for k0 in &self.keys {
            let u = (t - k0) / 0.45;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let r = p.blob_r + 0.05 + u * 0.16;
            for q in 0..36 {
                let (x, y) = body(r, q as f64 * TAU / 36.0);
                g.plot(x, y, if u < 0.5 { 2 } else { 1 }, INK_MAIN);
            }
        }
        // Listo: anillo verde que se expande hasta la escala.
        for b in &self.blooms {
            let u = (t - b) / 1.1;
            if !(0.0..=0.75).contains(&u) {
                continue;
            }
            let r = p.blob_r + 0.04 + u * 0.62;
            let n = 50 + (u * 40.0) as usize;
            for q in 0..n {
                let (x, y) = body(r, q as f64 * TAU / n as f64);
                g.plot(x, y, if u < 0.45 { 3 } else { 2 }, INK_GREEN);
            }
        }

        // Radar (ejecutando).
        if p.sweep > 0.01 {
            let a0 = -self.arc_phase * 1.3;
            for s in 0..10 {
                let a = a0 + s as f64 * 0.06;
                let mut r = p.blob_r + 0.14;
                while r < 0.78 {
                    g.polar(r, a, if s < 2 { 3 } else if s < 5 { 2 } else { 1 }, INK_MAIN);
                    r += 0.035;
                }
            }
        }

        // Ideas en órbita (pensando). Al irse a otro estado se hunden en el cuerpo.
        if p.orbit > 0.01 {
            for k in 0..3 {
                let base = self.phase * (1.1 + k as f64 * 0.25) + k as f64 * TAU / 3.0;
                let rr = p.blob_r + p.amp + 0.12 + k as f64 * 0.06 + 0.03 * (t * 1.7 + k as f64 * 2.0).sin();
                let r = p.blob_r + (rr - p.blob_r) * p.orbit;
                for s in 0..7 {
                    let a = base - s as f64 * 0.11;
                    let (x, y) = body(r, a);
                    g.plot(x, y, if s < 2 { 3 } else { 2 }, INK_MAIN);
                    if s < 2 && !low {
                        for dr in [0.035, -0.035] {
                            let (x, y) = body(r + dr, a);
                            g.plot(x, y, 3, INK_MAIN);
                        }
                    }
                }
            }
        }

        // Paquetes (en la red): salen al anillo y vuelven; donde rebotan se enciende la escala.
        if p.packets > 0.01 {
            for k in 0..4 {
                let trip = t * 0.8 + k as f64 / 4.0;
                let (n, u) = (trip.floor(), trip.fract());
                let h = ((n * 4.0 + k as f64) * 12.9898).sin() * 43758.5453;
                let a = h.fract().abs() * TAU;
                let out = if u < 0.5 { u * 2.0 } else { 2.0 - u * 2.0 };
                let r = p.blob_r + 0.06 + out * (0.84 - p.blob_r) * p.packets;
                for s in 0..if low { 2 } else { 4 } {
                    let rs = r - if u < 0.5 { 1.0 } else { -1.0 } * s as f64 * 0.04;
                    if rs > p.blob_r {
                        let (x, y) = body(rs, a);
                        g.plot(x, y, if s == 0 { 3 } else if s < 2 { 2 } else { 1 }, INK_MAIN);
                    }
                }
                if (u - 0.5).abs() < 0.08 {
                    g.polar(0.94, a, 3, INK_MAIN);
                    g.polar(0.90, a, 3, INK_MAIN);
                }
            }
        }

        // Piezas (editando): entran desde la escala hacia el contorno.
        if p.stitch > 0.01 {
            for k in 0..3 {
                let v = t * 0.9 + k as f64 / 3.0;
                let a = k as f64 * 2.4 + v.floor() * 1.7;
                let r = 0.84 - v.fract() * (0.84 - p.blob_r - 0.06);
                let (x, y) = body(r, a);
                g.plot(x, y, 3, INK_MAIN);
                let (x, y) = body(r + 0.04, a);
                g.plot(x, y, 2, INK_MAIN);
            }
        }

        // Grafo de git: tronco que sube, una rama y nodos que aparecen con un pop.
        if p.git > 0.01 {
            let gx = cx + 0.1;
            let base = cy + p.blob_r + 0.02;
            let grow2 = (st / 2.2).min(1.0);
            let mut y = 0.0;
            while y < 0.42 * grow2 {
                g.plot(gx, base + y, 2, INK_MAIN);
                y += 0.03;
            }
            let mut u = 0.0;
            while u < (grow2 - 0.35).max(0.0) / 0.65 {
                g.plot(gx - u * 0.22, base + 0.12 + u * 0.22, 2, INK_MAIN);
                u += 0.04;
            }
            for (nx, ny, at) in [(gx, base + 0.10, 0.2), (gx, base + 0.26, 0.5), (gx - 0.22, base + 0.34, 0.8), (gx, base + 0.40, 0.95)] {
                if grow2 > at {
                    let pop = ((grow2 - at) * 8.0).min(1.0);
                    let rr = 0.035 + 0.06 * (1.0 - pop).max(0.0);
                    for q in 0..10 {
                        let a = q as f64 * TAU / 10.0;
                        g.plot(nx + rr * a.cos(), ny + rr * a.sin(), 3, INK_MAIN);
                    }
                }
            }
        }

        // «?» (no te oigo).
        if p.droop > 0.5 {
            let (qx, qy) = (cx + 0.32, cy + p.blob_r + 0.12 + 0.02 * (t * 3.0).sin());
            for (a, b) in [(0, 4), (1, 5), (2, 5), (3, 4), (3, 3), (2, 2), (1, 1), (1, 0), (1, -2)] {
                g.plot(qx + a as f64 * 0.025 - 0.04, qy + b as f64 * 0.03, 3, INK_MAIN);
            }
        }

        // Zetas (en reposo).
        if p.zzz > 0.01 && !low {
            for k in 0..3 {
                let u = (t * 0.22 + k as f64 / 3.0).fract();
                let s = 0.03 + u * 0.04;
                let (zx, zy) = (cx + p.blob_r * 0.6 + u * 0.32, cy + p.blob_r * 0.5 + u * 0.38);
                let tone = if u < 0.5 { 2 } else { 1 };
                for q in -1..=1 {
                    g.plot(zx + q as f64 * s, zy + s, tone, INK_MAIN);
                    g.plot(zx + q as f64 * s, zy - s, tone, INK_MAIN);
                }
                g.plot(zx, zy, tone, INK_MAIN);
            }
        }

        // Vuelve el subagente: una gota viaja desde la escala y se funde.
        for &(t0, a, _) in &self.merges {
            let u = (t - t0) / 0.7;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let r = 0.86 - u * (0.86 - p.blob_r);
            let rr = 0.07 * (1.0 - u * 0.5);
            let (bx, by) = body(r, a);
            for q in 0..14 {
                let b = q as f64 * TAU / 14.0;
                g.plot(bx + rr * b.cos(), by + rr * b.sin(), 3, INK_MAIN);
            }
        }

        // Brote (delegando): se separa, se aleja y vuelve.
        let bud = (p.bud > 0.01).then(|| {
            let a = self.phase * 0.9;
            let out = 0.5 + 0.5 * (t * 1.1 - PI / 2.0).sin();
            let d = p.blob_r + (0.05 + out * 0.26) * p.bud;
            if out > 0.35 {
                for q in (0..6).step_by(2) {
                    let (x, y) = body(p.blob_r + q as f64 / 6.0 * (d - p.blob_r), a);
                    g.plot(x, y, 1, INK_MAIN);
                }
            }
            (cx + d * a.cos(), cy + d * a.sin(), 0.10 + 0.02 * (t * 3.0).sin())
        });

        // Cuerpo.
        let yawn = match self.gesture {
            Some((Gesture::Yawn, t0)) => (PI * ((t - t0) / 2.2).min(1.0)).sin(),
            _ => 0.0,
        };
        let breath = p.zzz * 0.03 * (t * 1.25).sin() + (1.0 - p.zzz) * 0.012 * (t * 1.2).sin() + yawn * 0.05;
        // Respondiendo late con el volumen real de su voz; sin voz, a su propio ritmo.
        let beat = p.ripple * 0.045 * if self.level > 0.01 { self.level * 1.6 } else { (t * 5.0).sin().abs() };
        let groove = self.groove * self.music;
        let voice = p.vu * lv * 0.22 + groove * 0.10;
        // Con música crece desde el centro con el nivel, como el brillo de las teclas.
        let radius = p.blob_r * grow * (1.0 - 0.35 * self.deflate) * (1.0 + groove * 0.18);
        let chaos = p.chaos + self.red * 2.0;
        let red = self.red;
        let phase = self.phase;
        let r_at = |th: f64| {
            radius + breath + beat + voice * (0.6 + 0.4 * (5.0 * th + t * 7.0).sin())
                + p.amp
                    * (0.60 * (3.0 * th + phase).sin()
                        + 0.40 * (5.0 * th - phase * 1.6 + 1.0).sin()
                        + 0.30 * chaos * (7.0 * th + phase * 2.7 + 2.0).sin())
                + red * 0.09 * (11.0 * th + t * 20.0).sin().max(0.0)
        };
        let (sx, sy) = (1.0 + self.sq, 1.0 - self.sq);
        let cursor = t * 2.6;
        let scan_y = cy + p.blob_r * 0.8 * (st * 1.6).cos();
        // Pupila: se dilata según el estado, se contrae de golpe con un error, y nunca baja de
        // ~2,5 puntos de radio (en el panel grande de Cine se perdía).
        let pr = if p.pupil > 0.01 {
            (0.085 * p.dilate * (1.0 - self.red * 0.45)).max(2.5 * g.du) * p.pupil * (1.0 - yawn * 0.8)
        } else {
            0.0
        };
        let blink = (matches!(self.shown, Idle | Typing) && (t % 4.3) < 0.12) || self.copy > 0.3;
        let (px, py) = (cx + self.gaze[0], cy + self.gaze[1]);
        let (lx, ly, lr) = (cx + p.blob_r * 0.55 * (st * 1.3).sin(), cy + p.blob_r * 0.45 * (st * 2.1 + 1.0).sin(), 0.15);
        if p.lens > 0.01 {
            for q in 0..40 {
                let a = q as f64 * TAU / 40.0;
                g.plot(lx + lr * a.cos(), ly + lr * a.sin(), 3, INK_MAIN);
            }
            if !low {
                for q in 0..5 {
                    let d = (lr + q as f64 * 0.025) * 0.7;
                    g.plot(lx + d, ly - d, 3, INK_MAIN);
                }
            }
        }
        let ctx = self.sig.ctx.clamp(0.0, 1.0);
        let liquid_ink = if ctx > 0.8 { INK_WARM } else { err };
        // Contorno: al menos ~2 puntos de grosor, aunque el panel sea chico.
        let band = 0.065_f64.max(1.9 * g.du);

        for j in 0..g.dh {
            for i in 0..g.dw {
                let (x, y) = g.center(i, j);
                let (dx, dy) = ((x - cx) / sx, (y - cy) / sy);
                let r = dx.hypot(dy);
                let mut in_bud = false;
                if let Some((bx, by, br)) = bud {
                    let e = br - (x - bx).hypot(y - by);
                    if (0.0..0.05).contains(&e) {
                        g.dot(i, j, 3, err);
                        continue;
                    }
                    in_bud = e >= 0.05;
                }
                if r > 0.8 && !in_bud {
                    continue;
                }
                let th = dy.atan2(dx);
                let d = r_at(th) - r;
                if (0.0..band).contains(&d) {
                    if p.dashed > 0.5 && (th * 18.0).sin() <= 0.0 {
                        continue;
                    }
                    let mut tone = 3;
                    if p.stitch > 0.5 {
                        let da = ((th - cursor).rem_euclid(TAU) - PI).abs();
                        tone = if da > PI - 0.5 { 3 } else { 2 };
                    }
                    g.dot(i, j, tone, err);
                } else if (d > band + 0.035 || in_bud) && p.dashed < 0.5 {
                    // Capas de adentro hacia afuera: pupila > lupa > línea de lectura > líquido > trama.
                    let pupil_r = pr * if p.lens > 0.5 { 1.5 } else { 1.0 };
                    let (ex, ey) = (x - px, y - py);
                    let er = ex.hypot(ey);
                    if pupil_r > 0.01 && er < pupil_r {
                        // Al parpadear queda solo una raya.
                        if !blink || ey.abs() < 0.02 {
                            g.dot(i, j, 3, err);
                        }
                        continue;
                    }
                    if p.lens > 0.5 && (x - lx).hypot(y - ly) < lr {
                        if (i + j) % 2 == 0 {
                            g.dot(i, j, 3, INK_MAIN);
                        }
                        continue;
                    }
                    if p.scan > 0.01 && (y - scan_y).abs() < 0.025 {
                        g.dot(i, j, 3, INK_MAIN);
                        continue;
                    }
                    if ctx > 0.01 && y < cy - radius + 2.0 * radius * ctx + 0.02 * (x * 14.0 + t * 3.0).sin() {
                        if (i + j) % 2 == 0 {
                            g.dot(i, j, 2, liquid_ink);
                        }
                        continue;
                    }
                    if (i + j * 2) % 3 == 0 && j % 2 == 0 {
                        g.dot(i, j, 2, err);
                    }
                }
            }
        }

        self.paint_kids(g, cx, cy, radius);

        // Motas lejanas y medias: al final y solo en celdas vacías, así el anillo, los arcos
        // y el cuerpo las tapan. Las cercanas van después, por encima de todo.
        if p.motes > 0.05 && !low && !self.mini {
            let n = (MOTES as f64 * p.motes * boot).round() as usize;
            for m in self.motes.iter().take(n).filter(|m| m.z < 0.66) {
                let (x, y) = self.mote_pos(m, cam, p.press, t);
                g.plot_under(x, y, 1);
                if m.z >= 0.33 {
                    g.plot_under(x + g.du, y, 1);
                }
            }
        }

        // Motas cercanas: al frente de todo, al color pleno, más grandes y con estela.
        if p.motes > 0.05 && !self.mini {
            let n = (MOTES as f64 * p.motes * boot).round() as usize;
            let d = g.du;
            for m in self.motes.iter().take(n).filter(|m| m.z >= 0.66) {
                let (x, y) = self.mote_pos(m, cam, p.press, t);
                for (ox, oy) in [(0.0, 0.0), (d, 0.0), (0.0, d), (d, d)] {
                    g.plot(x + ox, y + oy, 3, INK_ACCENT);
                }
                // Estela hacia atrás en su giro.
                for k in 1..3 {
                    let back = Mote { a: m.a - k as f64 * 0.05, ..*m };
                    let (x, y) = self.mote_pos(&back, cam, p.press, t);
                    g.plot(x, y, if k == 1 { 2 } else { 1 }, INK_ACCENT);
                }
            }
        }
    }
}

impl Core {
    /// Los hijos, con su línea al principal y las partículas que suben por ella.
    fn paint_kids(&self, g: &mut Grid, cx: f64, cy: f64, radius: f64) {
        let t = self.t;
        let alive = self.kids.iter().filter(|k| k.end.is_none()).count();
        // Radio del cuerpo de un hijo: con muchos se achican; nunca por debajo de lo que se
        // lee como un ojo.
        let kr = (if alive > 4 { 0.10_f64 } else { 0.13 }).max(2.6 * g.du);
        for (n, k) in self.kids.iter().enumerate() {
            // Cuánto salió del cuerpo (0 adentro, 1 en la órbita) y qué tamaño tiene.
            let (out, size) = match k.end {
                None => {
                    let u = ((t - k.born) / KID_TRAVEL).min(1.0);
                    (ease_out_back(u).min(1.08), 0.35 + 0.65 * ease_out_back(u))
                }
                Some((t0, true)) => {
                    let u = ((t - t0) / KID_TRAVEL).min(1.0);
                    let back = 1.0 - u * u;
                    (back, 0.4 + 0.6 * back)
                }
                Some((t0, false)) => (1.0, 1.0 - 0.8 * ((t - t0) / 1.4).min(1.0)),
            };
            let red = k.core.red > 0.35;
            let ink = INK_KID + n.min(MAX_KID_INKS - 1) as u8;
            let bob = 0.012 * (t * 1.7 + k.id as f64).sin();
            let orbit = radius + (KID_ORBIT - radius) * out;
            let (kx, ky) = (orbit * k.a.cos(), orbit * k.a.sin() + bob);
            let r = kr * size;

            // La línea: del borde del principal al del hijo, en el acento. Puntos que fluyen
            // hacia adentro.
            let (ax, ay) = (cx + (radius + 0.05) * k.a.cos(), cy + (radius + 0.05) * k.a.sin());
            let (bx, by) = (kx - (r + 0.05) * k.a.cos(), ky - (r + 0.05) * k.a.sin());
            let len = (bx - ax).hypot(by - ay);
            if len > 0.02 {
                let line = if red { INK_RED } else { INK_ACCENT };
                let step = 2.2 * g.du;
                let steps = (len / step).floor() as usize;
                let flow = if k.end.is_none() { t * 1.6 } else { 0.0 };
                for q in 0..=steps {
                    let u = q as f64 / steps.max(1) as f64;
                    // Una cresta que viaja del hijo al principal: la línea «respira» hacia el centro.
                    let tone = if (u * 3.0 + flow).fract() < 0.18 { 3 } else { 2 };
                    g.plot(bx + (ax - bx) * u, by + (ay - by) * u, tone, line);
                }
                // Partículas: una por cada cosa que hizo el hijo, en su color, del hijo al principal.
                let d = g.du;
                for &(p0, err) in &k.pulses {
                    let u = ((t - p0) / PULSE_TRAVEL).clamp(0.0, 1.0);
                    let e = u * u * (3.0 - 2.0 * u);
                    let (px, py) = (bx + (ax - bx) * e, by + (ay - by) * e);
                    let pink = if err { INK_RED } else { ink };
                    for oy in [-d, 0.0, d] {
                        for ox in [-d, 0.0, d] {
                            g.plot(px + ox, py + oy, 3, pink);
                        }
                    }
                    // Estela hacia el hijo.
                    for (q, back) in [(3, 0.07), (2, 0.13)] {
                        let e2 = (e - back).max(0.0);
                        let (qx, qy) = (bx + (ax - bx) * e2, by + (ay - by) * e2);
                        g.plot(qx, qy, q, pink);
                        g.plot(qx + d, qy, q, pink);
                    }
                }
            }

            // El hijo: su propio núcleo, a escala y en su lugar, pintado con su tinta.
            g.view(kx, ky, r / BASE.blob_r, ink);
            k.core.paint(g);
            g.unview();
        }
    }
}

/// Dónde va el hijo número `slot` de `n` en este momento: repartidos parejo, girando
/// despacio, el primero arriba a la izquierda (el reloj ocupa la esquina de la derecha).
fn kid_slot(t: f64, slot: usize, n: usize, calm: f64) -> f64 {
    PI * 0.75 + t * 0.06 * calm + slot as f64 * TAU / n.max(1) as f64
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}

const INK_MAIN: u8 = 0;
const INK_GREEN: u8 = 1;
const INK_RED: u8 = 2;
const INK_WARM: u8 = 3;
/// El acento del tema (Aether): lo de alrededor, que no cambia con el estado.
const INK_ACCENT: u8 = 4;
/// Desde acá, una tinta por hijo: cada uno con el color de lo que está haciendo.
const INK_KID: u8 = 5;
const MAX_KID_INKS: usize = 40;

/// Rejilla de puntos. Coordenadas del mundo: y ∈ [-1, 1] (x proporcional); si el panel es
/// más alto que ancho, se encoge todo para que el anillo entre.
///
/// Cada punto guarda su tono (1-3, 0 = vacío) y su tinta. La salida decide cómo se ven: en
/// braille se agrupan de a 2×4 por celda y la celda toma el color del más brillante; en
/// imagen (Sixel) cada punto es un círculo con su propio color.
struct Grid {
    dw: usize,
    dh: usize,
    asp: f64,
    s: f64,
    /// Tamaño de un punto en unidades del mundo.
    du: f64,
    /// tono | tinta << 2, por punto.
    dots: Vec<u8>,
    /// Bloques para `plot_under` (lo de atrás solo se dibuja donde no hay nada): en braille,
    /// la celda; en imagen, un cuadrado de unos pocos puntos.
    bw: usize,
    bh: usize,
    used: Vec<bool>,
    /// Vista para pintar un hijo: el mundo se corre a (`ox`, `oy`) y se escala por `k`, y la
    /// tinta del estado se pinta con la del hijo (`ink`). Sin hijo: 0, 0, 1 y ninguna.
    ox: f64,
    oy: f64,
    k: f64,
    ink: Option<u8>,
    du0: f64,
}

impl Grid {
    /// Para braille: `cw`×`ch` celdas de 2×4 puntos.
    fn new(cw: usize, ch: usize) -> Self {
        Self::with_dots(cw * 2, ch * 4, 2, 4)
    }

    fn with_dots(dw: usize, dh: usize, bw: usize, bh: usize) -> Self {
        let asp = dw as f64 / dh as f64;
        let s = asp.min(1.0);
        let (bx, by) = (dw.div_ceil(bw), dh.div_ceil(bh));
        let du = 2.0 / (s * dh as f64);
        Grid { dw, dh, asp, s, du, dots: vec![0; dw * dh], bw, bh, used: vec![false; bx * by], ox: 0.0, oy: 0.0, k: 1.0, ink: None, du0: du }
    }

    fn view(&mut self, ox: f64, oy: f64, k: f64, ink: u8) {
        (self.ox, self.oy, self.k, self.ink) = (ox, oy, k, Some(ink));
        self.du = self.du0 / k;
    }

    fn unview(&mut self) {
        (self.ox, self.oy, self.k, self.ink) = (0.0, 0.0, 1.0, None);
        self.du = self.du0;
    }

    fn index(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let (x, y) = (self.ox + x * self.k, self.oy + y * self.k);
        let i = ((x * self.s / self.asp + 1.0) / 2.0 * (self.dw - 1) as f64).round();
        let j = ((1.0 - (y * self.s + 1.0) / 2.0) * (self.dh - 1) as f64).round();
        (i >= 0.0 && j >= 0.0 && i < self.dw as f64 && j < self.dh as f64).then_some((i as usize, j as usize))
    }

    fn plot(&mut self, x: f64, y: f64, tone: u8, ink: u8) {
        if let Some((i, j)) = self.index(x, y) {
            self.dot(i, j, tone, ink);
        }
    }

    /// Como `plot`, pero solo si su bloque está vacío: para lo que queda detrás (las motas,
    /// en el acento).
    fn plot_under(&mut self, x: f64, y: f64, tone: u8) {
        if let Some((i, j)) = self.index(x, y) {
            if !self.used[self.block(i, j)] {
                self.dot(i, j, tone, INK_ACCENT);
            }
        }
    }

    fn block(&self, i: usize, j: usize) -> usize {
        (j / self.bh) * self.dw.div_ceil(self.bw) + i / self.bw
    }

    fn polar(&mut self, r: f64, a: f64, tone: u8, ink: u8) {
        self.plot(r * a.cos(), r * a.sin(), tone, ink);
    }

    fn dot(&mut self, i: usize, j: usize, tone: u8, ink: u8) {
        let ink = match self.ink {
            Some(kid) if ink == INK_MAIN || ink == INK_ACCENT => kid,
            _ => ink,
        };
        let k = j * self.dw + i;
        let old = self.dots[k];
        let (ot, oi) = (old & 3, old >> 2);
        if tone > ot || (tone == ot && ink > oi) {
            self.dots[k] = tone | ink << 2;
        }
        let b = self.block(i, j);
        self.used[b] = true;
    }

    fn center(&self, i: usize, j: usize) -> (f64, f64) {
        let x = (i as f64 / (self.dw - 1) as f64 * 2.0 - 1.0) * self.asp / self.s;
        let y = (1.0 - j as f64 / (self.dh - 1) as f64 * 2.0) / self.s;
        ((x - self.ox) / self.k, (y - self.oy) / self.k)
    }
}

/// Entre qué radios vive una mota según su profundidad: las lejanas entre el cuerpo y el
/// anillo, las cercanas por fuera, encima de la escala.
fn mote_band(z: f64, blob_r: f64) -> (f64, f64) {
    if z < 0.33 {
        (blob_r + 0.12, 0.72)
    } else if z < 0.66 {
        (0.55, 0.88)
    } else {
        (0.70, 1.05)
    }
}

/// Velocidad relativa de una mota: las cercanas van ~10× más rápido.
fn mote_speed(z: f64) -> f64 {
    0.25 + 2.4 * z * z
}

/// Si un color fijo cae demasiado cerca del acento del tema (Pensando azul con un tema azul),
/// se gira su tono hasta separarlo: cada estado tiene que seguir leyéndose distinto de En espera.
fn apart(c: [f64; 3], accent: [f64; 3]) -> [f64; 3] {
    let (h, s, v) = hsv(c);
    let (ha, sa, _) = hsv(accent);
    if s < 0.15 || sa < 0.15 {
        return c; // grises: no hay tono que comparar
    }
    let d = (h - ha + 540.0) % 360.0 - 180.0; // diferencia con signo, en grados
    if d.abs() >= 40.0 {
        return c;
    }
    let h = ha + if d >= 0.0 { 40.0 } else { -40.0 };
    from_hsv(h.rem_euclid(360.0), s, v)
}

fn hsv(c: [f64; 3]) -> (f64, f64, f64) {
    let [r, g, b] = c.map(|x| x / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

fn from_hsv(h: f64, s: f64, v: f64) -> [f64; 3] {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [(r + m) * 255.0, (g + m) * 255.0, (b + m) * 255.0]
}

fn mix(a: [f64; 3], b: [f64; 3], f: f64) -> [f64; 3] {
    [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
}

fn rgb(c: [f64; 3]) -> Color {
    Color::Rgb(c[0].round() as u8, c[1].round() as u8, c[2].round() as u8)
}

fn ease_out_back(u: f64) -> f64 {
    1.0 + 2.7 * (u - 1.0).powi(3) + 1.7 * (u - 1.0).powi(2)
}
