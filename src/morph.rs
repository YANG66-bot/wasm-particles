//! Morph / attractor mode: tens of thousands of particles spring toward
//! parametric shape targets with curl-like turbulence and mouse repulsion.

use crate::math::hash01;
use crate::shapes::{self, SHAPE_COUNT};
use crate::math::V3;

pub struct Morph {
    pub n: usize,
    pub pos: Vec<f32>,  // 3n
    pub vel: Vec<f32>,  // 3n
    pub extras: Vec<f32>,
    old_extras: Vec<f32>,
    targets: Vec<f32>,
    old_targets: Vec<f32>,
    delays: Vec<f32>,
    pub shape_idx: usize,
    pub shape_time: f32,
    pub cycle_len: f32,
    pub spring: f32,
    pub damp: f32,
    pub turb: f32,
    pub mouse_force: f32,
    pub text_mode: bool,
    pub time: f32,
}

impl Morph {
    pub fn new(n: usize) -> Morph {
        let mut m = Morph {
            n: 0,
            pos: Vec::new(),
            vel: Vec::new(),
            extras: Vec::new(),
            old_extras: Vec::new(),
            targets: Vec::new(),
            old_targets: Vec::new(),
            delays: Vec::new(),
            shape_idx: 0,
            shape_time: 999.0,
            cycle_len: 9.0,
            spring: 6.5,
            damp: 0.90,
            turb: 0.55,
            mouse_force: 5.0,
            text_mode: false,
            time: 0.0,
        };
        m.resize(n);
        m
    }

    pub fn resize(&mut self, n: usize) {
        self.n = n;
        let mut cloud = Vec::new();
        shapes::spawn_cloud(n, &mut cloud);
        self.pos = cloud;
        self.vel = vec![0.0; n * 3];
        self.extras = vec![0.5; n];
        self.old_extras = self.extras.clone();
        self.targets = vec![0.0; n * 3];
        self.old_targets = vec![0.0; n * 3];
        self.delays = vec![0.0; n];
        self.shape_time = 999.0;
        self.text_mode = false;
        self.gen_shape(self.shape_idx);
    }

    fn gen_shape(&mut self, idx: usize) {
        self.old_targets.copy_from_slice(&self.targets);
        self.old_extras.copy_from_slice(&self.extras);
        self.shape_idx = idx % SHAPE_COUNT;
        shapes::gen_shape(self.shape_idx, self.n, &mut self.targets, &mut self.extras);
        self.shape_time = 0.0;
        for i in 0..self.n {
            self.delays[i] = hash01(i as u32 * 53 + 3) * 1.1;
        }
    }

    pub fn next_shape(&mut self) {
        self.gen_shape(self.shape_idx + 1);
    }

    /// Replace targets with externally supplied points (JS text rasterizer).
    pub fn set_text(&mut self, pts: &[f32]) {
        let n = pts.len() / 3;
        if n == 0 {
            return;
        }
        if n != self.n {
            self.resize(n);
        }
        self.old_targets.copy_from_slice(&self.targets);
        self.old_extras.copy_from_slice(&self.extras);
        self.targets.clear();
        self.targets.extend_from_slice(pts);
        // palette param from target height + typewriter delay from x sweep
        let mut mn_y = f32::MAX;
        let mut mx_y = f32::MIN;
        let mut mn_x = f32::MAX;
        let mut mx_x = f32::MIN;
        for i in 0..n {
            let x = pts[i * 3];
            let y = pts[i * 3 + 1];
            mn_y = mn_y.min(y);
            mx_y = mx_y.max(y);
            mn_x = mn_x.min(x);
            mx_x = mx_x.max(x);
        }
        let sy = (mx_y - mn_y).max(1e-6);
        let sx = (mx_x - mn_x).max(1e-6);
        for i in 0..n {
            let y = pts[i * 3 + 1];
            let x = pts[i * 3];
            self.extras[i] =
                ((y - mn_y) / sy + (hash01(i as u32 * 59 + 11) - 0.5) * 0.08).clamp(0.0, 1.0);
            self.delays[i] = ((x - mn_x) / sx).clamp(0.0, 1.0) * 0.9;
        }
        self.text_mode = true;
        self.shape_time = 0.0;
    }

    pub fn update(&mut self, dt: f32, mouse: Option<&V3>) {
        if self.n == 0 {
            return;
        }
        self.time += dt;
        self.shape_time += dt;
        if !self.text_mode && self.shape_time > self.cycle_len {
            self.next_shape();
        }
        let (spring, turb, t) = (self.spring, self.turb, self.time);
        let damp_f = self.damp.powf(dt * 60.0);
        let r_max = 0.42;
        let mf = self.mouse_force;
        for i in 0..self.n {
            let o = i * 3;
            // staggered target switch
            let src = if self.shape_time < self.delays[i] {
                &self.old_targets
            } else {
                &self.targets
            };
            let x = self.pos[o];
            let y = self.pos[o + 1];
            let z = self.pos[o + 2];
            let mut fx = (src[o] - x) * spring;
            let mut fy = (src[o + 1] - y) * spring;
            let mut fz = (src[o + 2] - z) * spring;
            // curl-ish turbulence
            fx += ((y * 1.9 + t * 0.7).sin() + (z * 1.3 - t * 0.43).sin()) * turb * 0.5;
            fy += ((z * 1.7 + t * 0.51).sin() + (x * 1.1 + t * 0.83).sin()) * turb * 0.5;
            fz += ((x * 1.5 - t * 0.61).sin() + (y * 1.2 + t * 0.37).sin()) * turb * 0.5;
            if let Some(m) = mouse {
                let dx = x - m[0];
                let dy = y - m[1];
                let dz = z - m[2];
                let r2 = dx * dx + dy * dy + dz * dz;
                if r2 < r_max * r_max && r2 > 1e-9 {
                    let r = r2.sqrt();
                    let s = mf * (1.0 - r / r_max) * (1.0 - r / r_max) / r;
                    fx += dx * s;
                    fy += dy * s;
                    fz += dz * s;
                }
            }
            self.vel[o] = (self.vel[o] + fx * dt) * damp_f;
            self.vel[o + 1] = (self.vel[o + 1] + fy * dt) * damp_f;
            self.vel[o + 2] = (self.vel[o + 2] + fz * dt) * damp_f;
            self.pos[o] = x + self.vel[o] * dt;
            self.pos[o + 1] = y + self.vel[o + 1] * dt;
            self.pos[o + 2] = z + self.vel[o + 2] * dt;
        }
    }
}
