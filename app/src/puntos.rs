//! Los puntos del núcleo dibujados en la GPU: cada punto es una instancia (un cuadrado que el
//! fragment shader recorta en círculo con borde suave), todos en una sola llamada de dibujo.
//! En canvas 2D, un arc por punto se comía ~5 ms de CPU con 4k puntos; aquí da igual cuántos.

use iced::widget::shader::{self, Pipeline, Viewport};
use iced::wgpu;
use iced::{Rectangle, mouse};

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Dot {
    /// Centro, 0..1 del área.
    pub pos: [f32; 2],
    pub color: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    /// Tamaño del área en píxeles físicos.
    size: [f32; 2],
    /// Radio de cada punto en píxeles físicos.
    radius: f32,
    _pad: f32,
}

/// Lo que se dibuja en un cuadro: los puntos y la separación entre ellos en píxeles lógicos.
#[derive(Debug)]
pub struct Puntos {
    pub dots: Vec<Dot>,
    pub spacing: f32,
}

impl<Message> shader::Program<Message> for Puntos {
    type State = ();
    type Primitive = Frame;

    fn draw(&self, _: &(), _: mouse::Cursor, _: Rectangle) -> Frame {
        Frame { dots: self.dots.clone(), spacing: self.spacing }
    }
}

#[derive(Debug)]
pub struct Frame {
    dots: Vec<Dot>,
    spacing: f32,
}

impl Frame {
    pub fn new(dots: Vec<Dot>, spacing: f32) -> Self {
        Frame { dots, spacing }
    }
}

pub struct Gpu {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind: wgpu::BindGroup,
    instances: wgpu::Buffer,
    capacity: usize,
    count: u32,
}

const SHADER: &str = r#"
struct U { size: vec2<f32>, radius: f32, pad: f32 };
@group(0) @binding(0) var<uniform> u: U;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs(@builtin(vertex_index) v: u32, @location(0) center: vec2<f32>, @location(1) color: vec3<f32>) -> Out {
    // Dos triángulos por punto; un píxel de margen para el borde suave.
    var corners = array<vec2<f32>, 6>(vec2(-1., -1.), vec2(1., -1.), vec2(1., 1.), vec2(-1., -1.), vec2(1., 1.), vec2(-1., 1.));
    let c = corners[v];
    let px = center * u.size + c * (u.radius + 1.0);
    var o: Out;
    o.pos = vec4(px.x / u.size.x * 2.0 - 1.0, 1.0 - px.y / u.size.y * 2.0, 0.0, 1.0);
    o.local = c * (u.radius + 1.0);
    o.color = color;
    return o;
}

@fragment
fn fs(i: Out) -> @location(0) vec4<f32> {
    let a = clamp(u.radius - length(i.local) + 0.5, 0.0, 1.0);
    if (a <= 0.0) { discard; }
    return vec4(i.color * a, a);
}
"#;

impl Pipeline for Gpu {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("puntos"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("puntos.u"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("puntos"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Dot>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x3],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let capacity = 4096;
        let instances = instance_buffer(device, capacity);
        Gpu { pipeline, uniforms, bind, instances, capacity, count: 0 }
    }
}

fn instance_buffer(device: &wgpu::Device, n: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("puntos.i"),
        size: (n * std::mem::size_of::<Dot>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl shader::Primitive for Frame {
    type Pipeline = Gpu;

    fn prepare(&self, gpu: &mut Gpu, device: &wgpu::Device, queue: &wgpu::Queue, bounds: &Rectangle, viewport: &Viewport) {
        let scale = viewport.scale_factor() as f32;
        let u = Uniforms {
            size: [bounds.width * scale, bounds.height * scale],
            radius: self.spacing * scale * 0.36,
            _pad: 0.0,
        };
        queue.write_buffer(&gpu.uniforms, 0, bytemuck::bytes_of(&u));
        if self.dots.len() > gpu.capacity {
            gpu.capacity = self.dots.len().next_power_of_two();
            gpu.instances = instance_buffer(device, gpu.capacity);
        }
        queue.write_buffer(&gpu.instances, 0, bytemuck::cast_slice(&self.dots));
        gpu.count = self.dots.len() as u32;
    }

    fn draw(&self, gpu: &Gpu, pass: &mut wgpu::RenderPass<'_>) -> bool {
        pass.set_pipeline(&gpu.pipeline);
        pass.set_bind_group(0, &gpu.bind, &[]);
        pass.set_vertex_buffer(0, gpu.instances.slice(..));
        pass.draw(0..6, 0..gpu.count);
        true
    }
}
