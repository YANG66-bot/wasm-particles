//! WASM Particles — Rust → WebAssembly 实时粒子物理引擎
//!
//! - 物理全部在 Rust（wasm32）中计算：SPH 流体（Clavet 双重密度松弛）
//!   与形态汇聚/文字特效两种模式，SoA 内存布局 + 空间哈希网格，零逐帧堆分配
//! - 渲染通过 web-sys 直接绑定 WebGL2：HDR 累积缓冲 + 拖尾 + 色调映射

use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

mod camera;
mod gl;
mod math;
mod morph;
mod shapes;
mod sph;

use camera::Camera;
use gl::{DrawParams, Renderer};
use math::V3;
use morph::Morph;
use sph::Sph;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Sph = 0,
    Morph = 1,
    Text = 2,
}

struct Visuals {
    size: f32,
    intensity: f32,
    exposure: f32,
    trail: f32,
    speed_glow: f32,
    col_a: [f32; 3],
    col_b: [f32; 3],
    col_c: [f32; 3],
    col_d: [f32; 3],
}

struct App {
    renderer: Renderer,
    camera: Camera,
    sph: Sph,
    morph: Morph,
    mode: Mode,
    paused: bool,
    canvas: web_sys::HtmlCanvasElement,
    upload: Vec<f32>,
    vis: Visuals,
    // mouse
    mnx: f32,
    mny: f32,
    has_mouse: bool,
    dragging: bool,
    drag_x: i32,
    drag_y: i32,
    mouse_world: V3,
    // timing
    last_t: f64,
    dt_ema: f32,
    sim_ema: f32,
    render_ema: f32,
    frame_no: u32,
    // keep event closures alive
    ev_move: Option<Closure<dyn FnMut(web_sys::MouseEvent)>>,
    ev_down: Option<Closure<dyn FnMut(web_sys::MouseEvent)>>,
    ev_up: Option<Closure<dyn FnMut(web_sys::MouseEvent)>>,
    ev_wheel: Option<Closure<dyn FnMut(web_sys::WheelEvent)>>,
}

thread_local! {
    static APP: RefCell<Option<App>> = RefCell::new(None);
    static RAF: RefCell<Option<Closure<dyn FnMut()>>> = RefCell::new(None);
}

#[wasm_bindgen]
pub fn start(canvas: web_sys::HtmlCanvasElement, count: usize) -> Result<(), JsValue> {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&format!("panic: {}", info).into());
    }));

    let renderer = Renderer::new(&canvas).map_err(|e| JsValue::from_str(&e))?;
    let n = count.clamp(500, 200_000);
    let mut app = App {
        renderer,
        camera: Camera::new(),
        sph: Sph::new(n),
        morph: Morph::new(n),
        mode: Mode::Sph,
        paused: false,
        canvas: canvas.clone(),
        upload: vec![0.0; n * 7],
        vis: Visuals {
            size: 0.05,
            intensity: 0.5,
            exposure: 1.15,
            trail: 0.30,
            speed_glow: 0.55,
            col_a: [0.5, 0.55, 0.6],
            col_b: [0.45, 0.35, 0.25],
            col_c: [0.5, 0.5, 0.5],
            col_d: [0.5, 0.5, 0.5],
        },
        mnx: 0.0,
        mny: 0.0,
        has_mouse: false,
        dragging: false,
        drag_x: 0,
        drag_y: 0,
        mouse_world: [0.0, 0.0, 0.0],
        last_t: now_ms(),
        dt_ema: 1.0 / 60.0,
        sim_ema: 0.0,
        render_ema: 0.0,
        frame_no: 0,
        ev_move: None,
        ev_down: None,
        ev_up: None,
        ev_wheel: None,
    };
    app.renderer.alloc_particles(n);
    web_sys::console::log_1(
        &format!(
            "[particles] engine started, n={}, hdr={}",
            n,
            app.renderer.hdr()
        )
        .into(),
    );
    app.register_events();
    let has_app = APP.with(|a| a.borrow().is_some());
    APP.with(|a| {
        *a.borrow_mut() = Some(app);
    });
    if !has_app {
        // set up rAF loop exactly once
        RAF.with(|r| {
            let cb: Closure<dyn FnMut()> = Closure::new(|| frame());
            *r.borrow_mut() = Some(cb);
        });
    }
    frame();
    Ok(())
}

pub(crate) fn now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.performance())
            .map(|p| p.now())
            .unwrap_or(0.0)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        use std::time::Instant;
        static T0: OnceLock<Instant> = OnceLock::new();
        T0.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
    }
}

fn window() -> Option<web_sys::Window> {
    web_sys::window()
}

fn frame() {
    APP.with(|cell| {
        let mut borrow = cell.borrow_mut();
        if let Some(app) = borrow.as_mut() {
            app.frame();
        }
    });
    RAF.with(|r| {
        if let Some(cb) = r.borrow().as_ref() {
            if let Some(w) = window() {
                let _ = w.request_animation_frame(cb.as_ref().unchecked_ref());
            }
        }
    });
}

impl App {
    fn register_events(&mut self) {
        let canvas = self.canvas.clone();

        let c2 = canvas.clone();
        let mv: Closure<dyn FnMut(web_sys::MouseEvent)> = Closure::new(move |e: web_sys::MouseEvent| {
            let rect = c2.get_bounding_client_rect();
            let w = rect.width().max(1.0);
            let h = rect.height().max(1.0);
            let nx = ((e.client_x() as f64 - rect.left()) / w) * 2.0 - 1.0;
            let ny = -(((e.client_y() as f64 - rect.top()) / h) * 2.0 - 1.0);
            let (x, y) = (e.client_x(), e.client_y());
            let buttons = e.buttons();
            APP.with(|cell| {
                if let Some(app) = cell.borrow_mut().as_mut() {
                    app.mnx = nx as f32;
                    app.mny = ny as f32;
                    app.has_mouse = true;
                    if app.dragging {
                        let dx = x - app.drag_x;
                        let dy = y - app.drag_y;
                        app.drag_x = x;
                        app.drag_y = y;
                        app.camera.yaw -= dx as f32 * 0.005;
                        app.camera.pitch = (app.camera.pitch + dy as f32 * 0.005)
                            .clamp(-1.35, 1.35);
                    }
                    let _ = buttons;
                }
            });
        });
        let _ = canvas.add_event_listener_with_callback("mousemove", mv.as_ref().unchecked_ref());
        self.ev_move = Some(mv);

        let c3 = canvas.clone();
        let dn: Closure<dyn FnMut(web_sys::MouseEvent)> = Closure::new(move |e: web_sys::MouseEvent| {
            APP.with(|cell| {
                if let Some(app) = cell.borrow_mut().as_mut() {
                    app.dragging = true;
                    app.drag_x = e.client_x();
                    app.drag_y = e.client_y();
                }
            });
            let _ = &c3;
        });
        let _ = canvas.add_event_listener_with_callback("mousedown", dn.as_ref().unchecked_ref());
        self.ev_down = Some(dn);

        let up: Closure<dyn FnMut(web_sys::MouseEvent)> = Closure::new(move |_e: web_sys::MouseEvent| {
            APP.with(|cell| {
                if let Some(app) = cell.borrow_mut().as_mut() {
                    app.dragging = false;
                }
            });
        });
        if let Some(w) = window() {
            let _ = w.add_event_listener_with_callback("mouseup", up.as_ref().unchecked_ref());
        }
        self.ev_up = Some(up);

        let wh: Closure<dyn FnMut(web_sys::WheelEvent)> = Closure::new(move |e: web_sys::WheelEvent| {
            let d = e.delta_y() as f32;
            APP.with(|cell| {
                if let Some(app) = cell.borrow_mut().as_mut() {
                    app.camera.dist = (app.camera.dist * (d * 0.0012).exp()).clamp(0.7, 7.0);
                }
            });
        });
        let _ = canvas.add_event_listener_with_callback("wheel", wh.as_ref().unchecked_ref());
        self.ev_wheel = Some(wh);
    }

    fn check_resize(&mut self) {
        let dpr = window()
            .map(|w| w.device_pixel_ratio() as f32)
            .unwrap_or(1.0)
            .clamp(1.0, 2.0);
        let cw = self.canvas.client_width().max(1);
        let ch = self.canvas.client_height().max(1);
        let w = ((cw as f32 * dpr) as u32).max(1);
        let h = ((ch as f32 * dpr) as u32).max(1);
        if self.canvas.width() != w {
            self.canvas.set_width(w);
        }
        if self.canvas.height() != h {
            self.canvas.set_height(h);
        }
        self.renderer.resize(w as i32, h as i32);
    }

    fn frame(&mut self) {
        let t0 = now_ms();
        let mut dt = ((t0 - self.last_t) / 1000.0) as f32;
        self.last_t = t0;
        dt = if dt.is_finite() { dt.clamp(0.0, 0.1) } else { 1.0 / 60.0 };
        self.dt_ema = self.dt_ema * 0.9 + dt * 0.1;

        self.check_resize();
        self.camera.update(dt, self.dragging);

        let aspect = self.renderer.fb_w as f32 / self.renderer.fb_h.max(1) as f32;
        if self.has_mouse {
            self.mouse_world = self.camera.world_point(self.mnx, self.mny, aspect);
        }
        let mouse = if self.has_mouse { Some(self.mouse_world) } else { None };

        let t1 = now_ms();
        if !self.paused {
            match self.mode {
                Mode::Sph => self.sph.update(dt, mouse.as_ref()),
                _ => self.morph.update(dt, mouse.as_ref()),
            }
        }
        let t2 = now_ms();

        // build interleaved upload buffer
        let n = match self.mode {
            Mode::Sph => self.sph.n,
            _ => self.morph.n,
        };
        if self.upload.len() != n * 7 {
            self.upload.resize(n * 7, 0.0);
            self.renderer.alloc_particles(n);
        }
        let up = &mut self.upload;
        match self.mode {
            Mode::Sph => {
                let (pos, vel) = (&self.sph.pos, &self.sph.vel);
                for i in 0..n {
                    let o = i * 3;
                    let u = i * 7;
                    up[u] = pos[o];
                    up[u + 1] = pos[o + 1];
                    up[u + 2] = pos[o + 2];
                    up[u + 3] = vel[o];
                    up[u + 4] = vel[o + 1];
                    up[u + 5] = vel[o + 2];
                    let s = (vel[o] * vel[o] + vel[o + 1] * vel[o + 1] + vel[o + 2] * vel[o + 2])
                        .sqrt();
                    up[u + 6] = (s * 0.45).clamp(0.0, 1.0);
                }
            }
            _ => {
                let (pos, vel, extras) = (&self.morph.pos, &self.morph.vel, &self.morph.extras);
                for i in 0..n {
                    let o = i * 3;
                    let u = i * 7;
                    up[u] = pos[o];
                    up[u + 1] = pos[o + 1];
                    up[u + 2] = pos[o + 2];
                    up[u + 3] = vel[o];
                    up[u + 4] = vel[o + 1];
                    up[u + 5] = vel[o + 2];
                    up[u + 6] = extras[i];
                }
            }
        }

        let tan_half = (self.camera.fov_y * 0.5).tan();
        let size_px = self.vis.size * self.renderer.fb_h as f32 / (2.0 * tan_half);
        let vp = self.camera.vp(aspect);
        let params = DrawParams {
            vp,
            size_px,
            intensity: self.vis.intensity,
            speed_glow: self.vis.speed_glow,
            time: now_ms() as f32 / 1000.0,
            col_a: self.vis.col_a,
            col_b: self.vis.col_b,
            col_c: self.vis.col_c,
            col_d: self.vis.col_d,
        };

        self.renderer.begin_frame(self.vis.trail);
        self.renderer.upload_particles(up);
        self.renderer.draw_particles(n, &params);
        self.renderer.end_frame(self.vis.exposure);

        let t3 = now_ms();
        let sim = (t2 - t1) as f32;
        let render = (t3 - t2) as f32;
        let a = 0.12;
        self.sim_ema = self.sim_ema * (1.0 - a) + sim * a;
        self.render_ema = self.render_ema * (1.0 - a) + render * a;

        if self.frame_no % 120 == 60 {
            web_sys::console::log_1(
                &format!(
                    "[particles] fps={:.0} sim_raw={:.1}ms sim_ema={:.1}ms render_ema={:.1}ms n={} steps={} mode={}",
                    if self.dt_ema > 1e-5 { 1.0 / self.dt_ema } else { 0.0 },
                    sim,
                    self.sim_ema,
                    self.render_ema,
                    n,
                    if self.mode == Mode::Sph {
                        self.sph.last_steps
                    } else {
                        0
                    },
                    self.mode as u32
                )
                .into(),
            );
        }
        self.frame_no += 1;
    }

    fn set_mode(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        match mode {
            Mode::Sph => {
                self.camera.dist = 2.6;
                self.camera.target = [0.0, 0.0, 0.0];
                self.vis = Visuals {
                    size: 0.05,
                    intensity: 0.5,
                    exposure: 1.15,
                    trail: 0.30,
                    speed_glow: 0.55,
                    col_a: [0.5, 0.55, 0.6],
                    col_b: [0.45, 0.35, 0.25],
                    col_c: [0.5, 0.5, 0.5],
                    col_d: [0.5, 0.5, 0.5],
                };
            }
            Mode::Morph => {
                self.morph.text_mode = false;
                self.morph.shape_time = 999.0;
                self.camera.dist = 2.9;
                self.camera.target = [0.0, 0.0, 0.0];
                self.vis = Visuals {
                    size: 0.018,
                    intensity: 0.42,
                    exposure: 1.5,
                    trail: 0.13,
                    speed_glow: 0.3,
                    col_a: [0.5, 0.5, 0.52],
                    col_b: [0.48, 0.48, 0.46],
                    col_c: [0.85, 0.85, 0.85],
                    col_d: [0.6, 0.42, 0.24],
                };
            }
            Mode::Text => {
                self.morph.text_mode = true;
                self.camera.dist = 2.2;
                self.camera.target = [0.0, 0.0, 0.0];
                self.vis = Visuals {
                    size: 0.016,
                    intensity: 0.5,
                    exposure: 1.6,
                    trail: 0.2,
                    speed_glow: 0.3,
                    col_a: [0.62, 0.52, 0.36],
                    col_b: [0.4, 0.38, 0.3],
                    col_c: [0.5, 0.5, 0.5],
                    col_d: [0.5, 0.5, 0.5],
                };
            }
        }
    }

    fn set_count(&mut self, n: usize) {
        let n = n.clamp(500, 200_000);
        self.sph.resize(n);
        self.morph.resize(n);
        self.upload = vec![0.0; n * 7];
        self.renderer.alloc_particles(n);
    }
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut borrow = cell.borrow_mut();
        borrow.as_mut().map(f)
    })
}

#[wasm_bindgen]
pub fn set_mode(mode: u32) {
    let m = match mode {
        0 => Mode::Sph,
        1 => Mode::Morph,
        _ => Mode::Text,
    };
    let _ = with_app(|app| app.set_mode(m));
}

#[wasm_bindgen]
pub fn set_count(n: usize) {
    let _ = with_app(|app| app.set_count(n));
}

#[wasm_bindgen]
pub fn set_text_points(pts: &[f32]) {
    let _ = with_app(|app| app.morph.set_text(pts));
}

#[wasm_bindgen]
pub fn set_paused(p: bool) {
    let _ = with_app(|app| app.paused = p);
}

#[wasm_bindgen]
pub fn reset() {
    let _ = with_app(|app| match app.mode {
        Mode::Sph => app.sph.dam_break(),
        _ => {
            let n = app.morph.n;
            let mut cloud = Vec::new();
            shapes::spawn_cloud(n, &mut cloud);
            app.morph.pos.copy_from_slice(&cloud);
            for v in app.morph.vel.iter_mut() {
                *v = 0.0;
            }
        }
    });
}

#[wasm_bindgen]
pub fn next_shape() {
    let _ = with_app(|app| {
        app.morph.text_mode = false;
        app.morph.next_shape();
    });
}

#[wasm_bindgen]
pub fn set_param(name: &str, value: f32) {
    let _ = with_app(|app| {
        let a = &mut app.vis;
        match name {
            "size" => a.size = value,
            "intensity" => a.intensity = value,
            "exposure" => a.exposure = value,
            "trail" => a.trail = value,
            "speed_glow" => a.speed_glow = value,
            "gravity" => app.sph.gravity = value,
            "viscosity" => app.sph.sigma = value,
            "beta" => app.sph.beta = value,
            "stiffness" => app.sph.k = value,
            "stiffness_near" => app.sph.k_near = value,
            "mouse" => {
                app.sph.mouse_force = value;
                app.morph.mouse_force = value * 0.4;
            }
            "spring" => app.morph.spring = value,
            "damp" => app.morph.damp = value,
            "turb" => app.morph.turb = value,
            _ => {}
        }
    });
}

#[wasm_bindgen]
pub struct Stats {
    pub fps: f32,
    pub particles: u32,
    pub sim_ms: f32,
    pub render_ms: f32,
    pub shape: u32,
}

#[wasm_bindgen]
pub fn get_stats() -> Stats {
    APP.with(|cell| {
        let borrow = cell.borrow();
        if let Some(app) = borrow.as_ref() {
            let (n, shape) = match app.mode {
                Mode::Sph => (app.sph.n, 0u32),
                _ => (app.morph.n, app.morph.shape_idx as u32 + if app.morph.text_mode { 100 } else { 0 }),
            };
            Stats {
                fps: if app.dt_ema > 1e-5 { 1.0 / app.dt_ema } else { 0.0 },
                particles: n as u32,
                sim_ms: app.sim_ema,
                render_ms: app.render_ema,
                shape,
            }
        } else {
            Stats {
                fps: 0.0,
                particles: 0,
                sim_ms: 0.0,
                render_ms: 0.0,
                shape: 0,
            }
        }
    })
}
