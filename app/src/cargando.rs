//! Señales de "ya estoy en ello": un giro de puntos, como los del núcleo, para lo que
//! tarda (yt-dlp resolviendo, mpv llenando el búfer, una página que carga).

use std::f32::consts::TAU;

use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path};
use iced::{Color, Element, Point, Rectangle, Renderer, Theme, mouse};

const DOTS: usize = 8;
/// Segundos por vuelta.
const TURN: f32 = 0.9;

struct Spinner {
    t: f32,
    color: Color,
}

impl<M> canvas::Program<M> for Spinner {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let c = f.center();
        let r = bounds.width.min(bounds.height) / 2.0;
        let dot = (r * 0.26).max(1.5);
        let ring = r - dot;
        // La cabeza avanza continua; cada punto se apaga según lo lejos que va detrás.
        let head = (self.t / TURN).fract() * DOTS as f32;
        for i in 0..DOTS {
            let a = i as f32 / DOTS as f32 * TAU - TAU / 4.0;
            let behind = (head - i as f32).rem_euclid(DOTS as f32) / DOTS as f32;
            let alpha = (1.0 - behind).powf(1.3).max(0.25);
            f.fill(&Path::circle(Point::new(c.x + ring * a.cos(), c.y + ring * a.sin()), dot), self.color.scale_alpha(alpha));
        }
        vec![f.into_geometry()]
    }
}

/// El giro, de `size` píxeles de lado. `t`: segundos desde cualquier origen fijo.
pub fn spinner<'a, M: 'a>(t: f32, size: f32, color: Color) -> Element<'a, M> {
    Canvas::new(Spinner { t, color }).width(size).height(size).into()
}

/// Para los fondos de lo que aún no llega: respira entre `lo` y `hi` de opacidad.
pub fn pulse(t: f32, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * (0.5 - 0.5 * (t * TAU / 1.4).cos())
}
