//! "Para ti": canciones nuevas para Kevin a partir de lo que ya escucha.
//!
//! Como en docs/recomendaciones.md, fase 1: los candidatos salen de la radio de YouTube Music
//! de unas pocas semillas (favoritos y lo que más se escucha entero), y el historial los
//! ordena con reglas, sin aprendizaje: lo que salta seguido se va, los artistas que escucha
//! enteros suben, lo que aparece en varias radios a la vez sube más, y no se repite artista.

use crate::library::{ArtistRef, Play};
use crate::resolver::Track;
use std::collections::{HashMap, HashSet};

/// Cuántas radios se piden: cada una son ~50 candidatos en ~1 s, en paralelo.
pub const SEEDS: usize = 3;
/// Cuántas canciones trae la lista.
pub const SIZE: usize = 30;
/// Lo que sonó hace menos de esto no se recomienda: ya lo tiene presente.
const RECENT: u64 = 3 * 24 * 3600;
/// Como mucho estas por artista en la lista.
const PER_ARTIST: usize = 2;
/// Palabras de versiones que no son la canción: solo si la semilla también lo es.
const VERSIONS: [&str; 8] = ["live", "en vivo", "lyric", "letra", "cover", "karaoke", "8d", "slowed"];

#[derive(Default, Clone, Copy)]
struct Stats {
    completes: u32,
    skips: u32,
    last: u64,
}

/// Una reproducción que se rechazó: se cambió antes de los 30 s o de la mitad.
fn is_skip(p: &Play) -> bool {
    p.end == "skip" && (p.listened < 30.0 || p.duration.is_some_and(|d| p.listened < d * 0.5))
}

/// Terminó sola o se escuchó casi entera.
fn is_complete(p: &Play) -> bool {
    p.end == "eof" || p.duration.is_some_and(|d| d > 0.0 && p.listened >= d * 0.8)
}

/// El artista principal, para agrupar: "The Chainsmokers y Coldplay" → "the chainsmokers".
pub fn main_artist(t: &Track) -> String {
    let who = t.artist.as_deref().or(t.channel.as_deref()).unwrap_or("").to_lowercase();
    let mut s = who.as_str();
    for sep in [" y ", ", ", " & ", " feat", " ft.", " x "] {
        if let Some(i) = s.find(sep) {
            s = &s[..i];
        }
    }
    s.trim().trim_end_matches(" - topic").to_string()
}

fn is_version(title: &str) -> bool {
    let t = title.to_lowercase();
    VERSIONS.iter().any(|w| t.contains(w))
}

/// Lo que se sabe de los gustos, sacado del historial, los favoritos y los artistas seguidos.
pub struct Taste {
    tracks: HashMap<String, Stats>,
    /// Afinidad por artista: positiva si lo escucha entero o lo marcó, negativa si lo salta.
    artists: HashMap<String, f64>,
    favorites: HashSet<String>,
    disliked: HashSet<String>,
}

impl Taste {
    pub fn new(history: &[Play], favorites: &[Track], followed: &[ArtistRef], disliked: &[Track]) -> Self {
        let mut tracks: HashMap<String, Stats> = HashMap::new();
        let mut artists: HashMap<String, f64> = HashMap::new();
        for p in history {
            let s = tracks.entry(p.track.id.clone()).or_default();
            s.last = s.last.max(p.at);
            let a = artists.entry(main_artist(&p.track)).or_default();
            if is_complete(p) {
                s.completes += 1;
                *a += 1.0;
            } else if is_skip(p) {
                s.skips += 1;
                *a -= 0.7;
            }
        }
        for t in favorites {
            *artists.entry(main_artist(t)).or_default() += 3.0;
        }
        for a in followed {
            *artists.entry(a.name.to_lowercase()).or_default() += 4.0;
        }
        for t in disliked {
            *artists.entry(main_artist(t)).or_default() -= 1.5;
        }
        artists.remove("");
        Self {
            tracks,
            artists,
            favorites: favorites.iter().map(|t| t.id.clone()).collect(),
            disliked: disliked.iter().map(|t| t.id.clone()).collect(),
        }
    }

    /// Las semillas: entre favoritos y lo escuchado entero, al azar pero pesando cuánto gusta,
    /// y de artistas distintos para que las radios no sean la misma. `roll` da números en [0, 1).
    pub fn seeds(&self, history: &[Play], favorites: &[Track], mut roll: impl FnMut() -> f64) -> Vec<Track> {
        let mut pool: Vec<(Track, f64)> = favorites.iter().map(|t| (t.clone(), 3.0)).collect();
        let mut seen: HashSet<String> = favorites.iter().map(|t| t.id.clone()).collect();
        for p in history.iter().rev() {
            let s = self.tracks.get(&p.track.id).copied().unwrap_or_default();
            if s.completes > 0 && s.skips < 2 && !self.disliked.contains(&p.track.id) && seen.insert(p.track.id.clone()) {
                pool.push((p.track.clone(), s.completes as f64));
            }
        }
        let mut out: Vec<Track> = vec![];
        while out.len() < SEEDS && !pool.is_empty() {
            let total: f64 = pool.iter().map(|(_, w)| w).sum();
            let mut r = roll() * total;
            let i = pool.iter().position(|(_, w)| {
                r -= w;
                r < 0.0
            });
            let (t, _) = pool.remove(i.unwrap_or(pool.len() - 1));
            let a = main_artist(&t);
            if out.iter().all(|o| main_artist(o) != a) {
                out.push(t);
            }
        }
        out
    }

    /// Junta las radios y las ordena. Cada radio viene con la semilla primero.
    pub fn rank(&self, seeds: &[Track], radios: &[Vec<Track>], now: u64) -> Vec<Track> {
        let seed_ids: HashSet<&str> = seeds.iter().map(|t| t.id.as_str()).collect();
        let versions_ok = seeds.iter().any(|t| is_version(&t.title));
        let mut score: HashMap<String, (f64, Track)> = HashMap::new();
        for radio in radios {
            for (pos, t) in radio.iter().enumerate() {
                if seed_ids.contains(t.id.as_str()) || self.favorites.contains(&t.id) || self.disliked.contains(&t.id) {
                    continue;
                }
                let s = self.tracks.get(&t.id).copied().unwrap_or_default();
                if s.skips >= 2 || (s.last > 0 && now.saturating_sub(s.last) < RECENT) {
                    continue;
                }
                if !versions_ok && is_version(&t.title) {
                    continue;
                }
                // Lo primero de una radio es lo más parecido a la semilla.
                let mut v = 1.0 / (1.0 + pos as f64 * 0.08);
                if s.skips == 1 && s.completes == 0 {
                    v *= 0.5;
                }
                if s.completes > 0 {
                    // Ya la conoce y le gusta: puede ir, pero "para ti" es sobre todo descubrir.
                    v *= 0.6;
                }
                score.entry(t.id.clone()).or_insert_with(|| (0.0, t.clone())).0 += v;
            }
        }
        let mut list: Vec<(f64, Track)> = score
            .into_values()
            .map(|(v, t)| {
                let a = self.artists.get(&main_artist(&t)).copied().unwrap_or(0.0).clamp(-3.0, 8.0);
                (v * (1.0 + 0.12 * a), t)
            })
            .collect();
        list.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));

        // Elegir de arriba abajo, sin pasarse por artista y sin dos seguidas del mismo.
        let mut per: HashMap<String, usize> = HashMap::new();
        let mut out: Vec<Track> = vec![];
        let mut held: Vec<Track> = vec![];
        for (_, t) in list {
            let a = main_artist(&t);
            let n = per.entry(a.clone()).or_default();
            if *n >= PER_ARTIST {
                continue;
            }
            *n += 1;
            if out.last().is_some_and(|l| main_artist(l) == a) {
                held.push(t);
            } else {
                out.push(t);
                // Lo apartado por repetir artista entra en cuanto ya no queda pegado.
                if let Some(i) = held.iter().position(|h| main_artist(h) != main_artist(out.last().unwrap())) {
                    out.push(held.remove(i));
                }
            }
            if out.len() >= SIZE {
                break;
            }
        }
        out.extend(held);
        out.truncate(SIZE);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, artist: &str) -> Track {
        Track { id: id.into(), title: format!("canción {id}"), artist: Some(artist.into()), album: None, duration: Some(200.0), channel: None, channel_id: None }
    }

    fn play(tr: Track, at: u64, listened: f64, end: &str) -> Play {
        Play { track: tr, at, listened, duration: Some(200.0), end: end.into() }
    }

    #[test]
    fn artista_principal() {
        assert_eq!(main_artist(&t("a", "The Chainsmokers y Coldplay")), "the chainsmokers");
        assert_eq!(main_artist(&t("a", "benny blanco, Halsey y Khalid")), "benny blanco");
        assert_eq!(main_artist(&t("a", "Soda Stereo - Topic")), "soda stereo");
    }

    #[test]
    fn fuera_saltadas_recientes_favoritas_y_semillas() {
        let now = 10_000_000;
        let hist = vec![
            play(t("salta", "X"), 1000, 5.0, "skip"),
            play(t("salta", "X"), 2000, 8.0, "skip"),
            play(t("reciente", "Y"), now - 3600, 200.0, "eof"),
        ];
        let favs = vec![t("fav", "Z")];
        let taste = Taste::new(&hist, &favs, &[], &[]);
        let seed = t("semilla", "Z");
        let radio = vec![seed.clone(), t("salta", "X"), t("reciente", "Y"), t("fav", "Z"), t("nueva", "W")];
        let ids: Vec<String> = taste.rank(&[seed], &[radio], now).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, vec!["nueva"]);
    }

    #[test]
    fn fuera_las_que_no_gustan_y_su_artista_baja() {
        let taste = Taste::new(&[], &[], &[], &[t("mala", "Malo")]);
        let seed = t("semilla", "Z");
        let radio = vec![seed.clone(), t("mala", "Malo"), t("otra_del_malo", "Malo"), t("neutra", "W")];
        let ids: Vec<String> = taste.rank(&[seed], &[radio], 0).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, vec!["neutra", "otra_del_malo"]);
    }

    #[test]
    fn sube_lo_que_sale_en_varias_radios_y_los_artistas_queridos() {
        let favs = vec![t("f1", "Querido")];
        let taste = Taste::new(&[], &favs, &[], &[]);
        let r1 = vec![t("s1", "A"), t("comun", "B"), t("solo1", "C")];
        let r2 = vec![t("s2", "D"), t("solo2", "E"), t("comun", "B")];
        let r3 = vec![t("s3", "F"), t("x", "G"), t("y", "H"), t("del_querido", "Querido")];
        let out = taste.rank(&[t("s1", "A"), t("s2", "D"), t("s3", "F")], &[r1, r2, r3], 0);
        assert_eq!(out[0].id, "comun");
        let pos = |id: &str| out.iter().position(|t| t.id == id).unwrap();
        assert!(pos("del_querido") < pos("y"));
    }

    #[test]
    fn no_mas_de_dos_por_artista_ni_seguidas() {
        let taste = Taste::new(&[], &[], &[], &[]);
        let radio: Vec<Track> = std::iter::once(t("s", "S"))
            .chain((0..5).map(|i| t(&format!("a{i}"), "A")))
            .chain((0..3).map(|i| t(&format!("b{i}"), "B")))
            .collect();
        let out = taste.rank(&[t("s", "S")], &[radio], 0);
        assert_eq!(out.iter().filter(|t| t.artist.as_deref() == Some("A")).count(), 2);
        assert!(out.windows(2).all(|w| main_artist(&w[0]) != main_artist(&w[1])), "{:?}", out.iter().map(|t| &t.id).collect::<Vec<_>>());
    }

    #[test]
    fn semillas_de_artistas_distintos() {
        let favs = vec![t("a1", "A"), t("a2", "A"), t("b1", "B"), t("c1", "C")];
        let taste = Taste::new(&[], &favs, &[], &[]);
        let mut x = 0.0;
        let seeds = taste.seeds(&[], &favs, || {
            x = (x + 0.37) % 1.0;
            x
        });
        let artists: HashSet<String> = seeds.iter().map(main_artist).collect();
        assert_eq!(seeds.len(), 3);
        assert_eq!(artists.len(), 3);
    }
}
