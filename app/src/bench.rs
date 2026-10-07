//! `surco-app --bench`: el techo de la app sin Hyprland de por medio. Mismo núcleo y mismo
//! pipeline, pero dibujando en una textura fuera de pantalla lo más rápido posible.

use std::time::{Duration, Instant};

use iced::widget::shader::{Pipeline, Primitive, Viewport};
use iced::{Rectangle, Size, wgpu};

use crate::{SPACING, nucleo, puntos};

pub fn run() {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::LowPower),
        ..Default::default()
    }))
    .expect("sin GPU");
    eprintln!("GPU: {}", adapter.get_info().name);
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("sin dispositivo");
    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    // El área del núcleo de la ventana de 900×700 a escala 1.6, como en el panel.
    let (w, h, scale) = (900.0f32, 604.0f32, 1.6f32);
    let (pw, ph) = ((w * scale) as u32, (h * scale) as u32);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut gpu = puntos::Gpu::new(&device, &queue, format);
    let viewport = Viewport::with_physical_size(Size::new(pw, ph), scale);
    let bounds = Rectangle::new(iced::Point::ORIGIN, Size::new(w, h));

    let mut core = nucleo::Core::new();
    let sig = nucleo::Signals { music: 0.6, ..Default::default() };
    // Pasar el arranque: los primeros segundos el núcleo enciende pocos puntos.
    for _ in 0..240 * 5 {
        core.step(1.0 / 240.0, nucleo::State::Idle, &sig);
    }
    let (mut n, mut cpu, mut ndots) = (0u32, Duration::ZERO, 0usize);
    // BENCH_GPU=1: la misma forma todo el rato, para ver solo la GPU. BENCH_COLA=1: sin
    // esperar a la GPU en cada cuadro (como un juego con vsync apagado).
    let solo_gpu = std::env::var_os("BENCH_GPU").is_some();
    let cola = std::env::var_os("BENCH_COLA").is_some();
    let mut fija: Option<Vec<puntos::Dot>> = None;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        let t0 = Instant::now();
        if solo_gpu && fija.is_some() {
            ndots = fija.as_ref().unwrap().len();
            let frame = puntos::Frame::new(fija.clone().unwrap(), SPACING);
            frame.prepare(&mut gpu, &device, &queue, &bounds, &viewport);
            let mut enc = device.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    ..Default::default()
                });
                frame.draw(&gpu, &mut pass);
            }
            queue.submit([enc.finish()]);
            if !cola || n % 64 == 0 {
                device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).ok();
            }
            n += 1;
            continue;
        }
        core.step(1.0 / 240.0, nucleo::State::Idle, &sig);
        let dots: Vec<puntos::Dot> = core
            .dots((w / SPACING) as usize, (h / SPACING) as usize)
            .into_iter()
            .map(|(pos, color)| puntos::Dot { pos, color })
            .collect();
        cpu += t0.elapsed();
        ndots = dots.len();
        if solo_gpu {
            fija = Some(dots.clone());
        }
        let frame = puntos::Frame::new(dots, SPACING);
        frame.prepare(&mut gpu, &device, &queue, &bounds, &viewport);
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            frame.draw(&gpu, &mut pass);
        }
        queue.submit([enc.finish()]);
        // Esperar a que la GPU termine: cuenta cuadros terminados, no encolados.
        if !cola || n % 64 == 0 {
            device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).ok();
        }
        n += 1;
    }
    device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).ok();
    let total = start.elapsed().as_secs_f64();
    eprintln!(
        "{} cuadros en {:.1} s = {:.0} fps; {} puntos; CPU (forma) {:.3} ms/cuadro; resto (subir + dibujar + esperar GPU) {:.3} ms/cuadro",
        n,
        total,
        n as f64 / total,
        ndots,
        cpu.as_secs_f64() * 1000.0 / n as f64,
        (total - cpu.as_secs_f64()) * 1000.0 / n as f64
    );
}
