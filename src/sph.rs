//! SPH fluid solver — Clavet et al. "Particle-based Viscoelastic Fluid Simulation"
//! (double density relaxation), tuned for real-time WASM:
//!
//! - SoA f32 arrays + counting-sort spatial hash grid, zero per-frame allocations
//! - particles are physically re-sorted by cell every step, so neighbour loops
//!   scan contiguous memory with no indirection (cache-friendly)
//! - each unordered particle pair is visited exactly once per pass via
//!   half-stencil forward cell lists; viscosity + density share one pass, and
//!   the displacement pass uses the pairwise pressure-sum form (equivalent to
//!   the classic ordered two-visit scheme at half the work)
//! - fixed 60Hz simulation steps with a per-frame time budget: physics speed is
//!   fps-independent, stiffness never blows up on slow frames, and an
//!   over-budget step count degrades to slow motion instead of a death spiral

use crate::math::V3;

const STEP: f32 = 1.0 / 60.0;
const SIM_BUDGET_MS: f64 = 25.0;

pub struct Sph {
    pub n: usize,
    pub pos: Vec<f32>, // 3n
    pub vel: Vec<f32>, // 3n
    prev: Vec<f32>,    // 3n
    rho: Vec<f32>,
    rho_near: Vec<f32>,
    cell_start: Vec<u32>, // cells + 1
    cursor: Vec<u32>,     // cells
    cell_items: Vec<u32>, // n
    scr_pos: Vec<f32>,    // 3n
    scr_vel: Vec<f32>,    // 3n
    scr_prev: Vec<f32>,   // 3n
    fwd_start: Vec<u32>,  // cells + 1
    fwd_list: Vec<u32>,
    gx: i32,
    gy: i32,
    gz: i32,
    acc: f32,
    pub last_steps: u32,
    pub h: f32,
    pub rho0: f32,
    pub k: f32,
    pub k_near: f32,
    pub sigma: f32,
    pub beta: f32,
    pub gravity: f32,
    pub mouse_force: f32,
    pub bmin: V3,
    pub bmax: V3,
}

impl Sph {
    pub fn new(n: usize) -> Sph {
        let mut s = Sph {
            n: 0,
            pos: Vec::new(),
            vel: Vec::new(),
            prev: Vec::new(),
            rho: Vec::new(),
            rho_near: Vec::new(),
            cell_start: vec![0u32; 1],
            cursor: Vec::new(),
            cell_items: Vec::new(),
            scr_pos: Vec::new(),
            scr_vel: Vec::new(),
            scr_prev: Vec::new(),
            fwd_start: vec![0u32; 1],
            fwd_list: Vec::new(),
            gx: 1,
            gy: 1,
            gz: 1,
            acc: 0.0,
            last_steps: 0,
            h: 0.1,
            rho0: 3.0,
            // calibrated for the fixed 60Hz step (displacement ~ dt^2, so
            // stiffness is 1/4 of the classic 8.3ms-substep values; verified
            // stable by the sph native tests — see settles_to_rest)
            k: 15.0,
            k_near: 55.0,
            sigma: 3.0,
            beta: 3.0,
            gravity: 3.2,
            mouse_force: 14.0,
            bmin: [-1.05, -0.62, -0.62],
            bmax: [1.05, 0.62, 0.62],
        };
        s.resize(n);
        s
    }

    pub fn resize(&mut self, n: usize) {
        self.n = n;
        let box_v = (self.bmax[0] - self.bmin[0])
            * (self.bmax[1] - self.bmin[1])
            * (self.bmax[2] - self.bmin[2]);
        // spacing so the dam block occupies ~45% of the box volume
        let d = (0.45 * box_v / n.max(1) as f32).cbrt();
        self.h = d * 2.2;
        self.gx = (((self.bmax[0] - self.bmin[0]) / self.h) as i32).max(1);
        self.gy = (((self.bmax[1] - self.bmin[1]) / self.h) as i32).max(1);
        self.gz = (((self.bmax[2] - self.bmin[2]) / self.h) as i32).max(1);
        let cells = (self.gx * self.gy * self.gz) as usize;
        self.cell_start = vec![0u32; cells + 1];
        self.cursor = vec![0u32; cells];
        self.build_fwd();
        self.pos = vec![0.0; n * 3];
        self.vel = vec![0.0; n * 3];
        self.prev = vec![0.0; n * 3];
        self.scr_pos = vec![0.0; n * 3];
        self.scr_vel = vec![0.0; n * 3];
        self.scr_prev = vec![0.0; n * 3];
        self.rho = vec![0.0; n];
        self.rho_near = vec![0.0; n];
        self.cell_items = vec![0u32; n];
        self.acc = 0.0;
        self.dam_break();
    }

    /// Forward neighbour cell lists: for every unordered pair of adjacent cells
    /// exactly one direction is stored, so pair loops visit each cell pair once.
    fn build_fwd(&mut self) {
        let (gx, gy, gz) = (self.gx, self.gy, self.gz);
        let cells = (gx * gy * gz) as usize;
        let mut fwd_start = vec![0u32; cells + 1];
        let mut fwd_list = Vec::with_capacity(cells * 13);
        let mut c = 0usize;
        for cz in 0..gz {
            for cy in 0..gy {
                for cx in 0..gx {
                    for dz in -1i32..=1 {
                        let z = cz + dz;
                        if z < 0 || z >= gz {
                            continue;
                        }
                        for dy in -1i32..=1 {
                            let y = cy + dy;
                            if y < 0 || y >= gy {
                                continue;
                            }
                            for dx in -1i32..=1 {
                                // lexicographically after (0,0,0)
                                let fwd = dz > 0 || (dz == 0 && (dy > 0 || (dy == 0 && dx > 0)));
                                if !fwd {
                                    continue;
                                }
                                let x = cx + dx;
                                if x < 0 || x >= gx {
                                    continue;
                                }
                                fwd_list.push((x + y * gx + z * gx * gy) as u32);
                            }
                        }
                    }
                    c += 1;
                    fwd_start[c] = fwd_list.len() as u32;
                }
            }
        }
        self.fwd_start = fwd_start;
        self.fwd_list = fwd_list;
    }

    /// Reset into a dam-break column stacked at the left wall.
    pub fn dam_break(&mut self) {
        let n = self.n;
        if n == 0 {
            return;
        }
        let d = self.h / 2.2;
        let [bx0, by0, bz0] = self.bmin;
        let ny = (((self.bmax[1] - by0) * 0.97) / d) as usize;
        let nz = (((self.bmax[2] - bz0) * 0.97) / d) as usize;
        let ny = ny.max(1);
        let nz = nz.max(1);
        let nx = (n + ny * nz - 1) / (ny * nz);
        let mut i = 0usize;
        'outer: for ix in 0..nx {
            for iy in 0..ny {
                for iz in 0..nz {
                    if i >= n {
                        break 'outer;
                    }
                    let o = i * 3;
                    self.pos[o] = bx0 + 0.015 + ix as f32 * d;
                    self.pos[o + 1] = by0 + 0.015 + iy as f32 * d;
                    self.pos[o + 2] = bz0 + 0.02 + iz as f32 * d;
                    i += 1;
                }
            }
        }
        // fill remainder randomly inside the block region (tiny n edge case)
        while i < n {
            let o = i * 3;
            self.pos[o] = bx0 + 0.02 + (i as f32 % nx as f32) * d;
            self.pos[o + 1] = by0 + 0.02;
            self.pos[o + 2] = bz0 + 0.02;
            i += 1;
        }
        self.vel.fill(0.0);
        self.prev.copy_from_slice(&self.pos);
        // calibrate rest density from the initial lattice
        // (sdt = 0 makes the viscosity term a no-op, only densities are read)
        self.build_grid();
        self.pair_pass(0.0);
        let sum: f32 = self.rho.iter().sum();
        self.rho0 = sum / n as f32 * 0.92;
        self.acc = 0.0;
    }

    /// Counting sort by cell, then physically permute pos/vel/prev into cell
    /// order so all subsequent neighbour loops run over contiguous memory.
    fn build_grid(&mut self) {
        let n = self.n;
        let cells = self.cell_start.len() - 1;
        let (h, gx, gy, gz) = (self.h, self.gx, self.gy, self.gz);
        let (x0, y0, z0) = (self.bmin[0], self.bmin[1], self.bmin[2]);
        for v in self.cell_start.iter_mut() {
            *v = 0;
        }
        {
            let pos = &self.pos;
            let cell_of = |o: usize| -> usize {
                let ix = (((pos[o] - x0) / h) as i32).clamp(0, gx - 1);
                let iy = (((pos[o + 1] - y0) / h) as i32).clamp(0, gy - 1);
                let iz = (((pos[o + 2] - z0) / h) as i32).clamp(0, gz - 1);
                (ix + iy * gx + iz * gx * gy) as usize
            };
            for i in 0..n {
                self.cell_start[cell_of(i * 3) + 1] += 1;
            }
            for c in 0..cells {
                self.cell_start[c + 1] += self.cell_start[c];
            }
            self.cursor.copy_from_slice(&self.cell_start[..cells]);
            for i in 0..n {
                let c = cell_of(i * 3);
                self.cell_items[self.cursor[c] as usize] = i as u32;
                self.cursor[c] += 1;
            }
        }
        for k in 0..n {
            let o = self.cell_items[k] as usize * 3;
            let u = k * 3;
            self.scr_pos[u] = self.pos[o];
            self.scr_pos[u + 1] = self.pos[o + 1];
            self.scr_pos[u + 2] = self.pos[o + 2];
            self.scr_vel[u] = self.vel[o];
            self.scr_vel[u + 1] = self.vel[o + 1];
            self.scr_vel[u + 2] = self.vel[o + 2];
            self.scr_prev[u] = self.prev[o];
            self.scr_prev[u + 1] = self.prev[o + 1];
            self.scr_prev[u + 2] = self.prev[o + 2];
        }
        std::mem::swap(&mut self.pos, &mut self.scr_pos);
        std::mem::swap(&mut self.vel, &mut self.scr_vel);
        std::mem::swap(&mut self.prev, &mut self.scr_prev);
    }

    pub fn update(&mut self, dt: f32, mouse: Option<&V3>) {
        if self.n == 0 {
            return;
        }
        self.acc += dt.clamp(0.0, 0.1);
        let t_start = crate::now_ms();
        let mut steps = 0u32;
        while self.acc >= STEP {
            self.step(STEP, mouse);
            self.acc -= STEP;
            steps += 1;
            if crate::now_ms() - t_start > SIM_BUDGET_MS {
                // over budget this frame: stop simulating, drop backlog so the
                // app stays responsive (fluid slows down instead of spiraling)
                self.acc = self.acc.min(STEP);
                break;
            }
        }
        self.last_steps = steps;
    }

    fn step(&mut self, sdt: f32, mouse: Option<&V3>) {
        self.external_forces(sdt, mouse);
        // predict positions (prev keeps the pre-step state for velocity recovery)
        self.prev.copy_from_slice(&self.pos);
        for i in 0..self.n * 3 {
            let v = self.vel[i];
            self.pos[i] += v * sdt;
        }
        self.build_grid();
        self.pair_pass(sdt);
        self.displacement_pass(sdt);
        self.boundary();
        // velocity from positions
        let inv = 1.0 / sdt;
        for i in 0..self.n * 3 {
            self.vel[i] = (self.pos[i] - self.prev[i]) * inv;
        }
    }

    fn external_forces(&mut self, sdt: f32, mouse: Option<&V3>) {
        let g = self.gravity;
        for i in (1..self.n * 3).step_by(3) {
            self.vel[i] -= g * sdt;
        }
        if let Some(m) = mouse {
            let r_max = 0.34;
            let f = self.mouse_force;
            for i in 0..self.n {
                let o = i * 3;
                let dx = self.pos[o] - m[0];
                let dy = self.pos[o + 1] - m[1];
                let dz = self.pos[o + 2] - m[2];
                let r2 = dx * dx + dy * dy + dz * dz;
                if r2 < r_max * r_max && r2 > 1e-9 {
                    let r = r2.sqrt();
                    let s = f * (1.0 - r / r_max) * (1.0 - r / r_max) * sdt / r;
                    self.vel[o] += dx * s;
                    self.vel[o + 1] += dy * s;
                    self.vel[o + 2] += dz * s;
                }
            }
        }
    }

    /// One pass over every unordered pair: density accumulation plus the
    /// Clavet viscosity impulse. The impulse is applied as a position delta
    /// (positions are already predicted), which is exactly equivalent to the
    /// classic velocity-space form applied before prediction.
    fn pair_pass(&mut self, sdt: f32) {
        self.rho.fill(0.0);
        self.rho_near.fill(0.0);
        let h = self.h;
        let h2 = h * h;
        let inv_h = 1.0 / h;
        let (sigma, beta) = (self.sigma, self.beta);
        let cells = self.cell_start.len() - 1;
        let pos = self.pos.as_mut_slice();
        let vel = self.vel.as_slice();
        let rho = self.rho.as_mut_slice();
        let rho_near = self.rho_near.as_mut_slice();
        let cell_start = self.cell_start.as_slice();
        let fwd_start = self.fwd_start.as_slice();
        let fwd_list = self.fwd_list.as_slice();

        macro_rules! pair {
            ($a:expr, $b:expr) => {{
                let (a, b) = ($a, $b);
                let (oa, ob) = (a * 3, b * 3);
                // SAFETY: a and b are slot indices taken from cell ranges,
                // cell_start values are particle counts <= n, so all accesses
                // below are within pos/vel (3n) and rho/rho_near (n).
                let dx = unsafe { *pos.get_unchecked(ob) - *pos.get_unchecked(oa) };
                let dy = unsafe { *pos.get_unchecked(ob + 1) - *pos.get_unchecked(oa + 1) };
                let dz = unsafe { *pos.get_unchecked(ob + 2) - *pos.get_unchecked(oa + 2) };
                let r2 = dx * dx + dy * dy + dz * dz;
                if r2 < h2 && r2 > 1e-12 {
                    let r = r2.sqrt();
                    let q = 1.0 - r * inv_h;
                    let q2 = q * q;
                    unsafe {
                        *rho.get_unchecked_mut(a) += q2;
                        *rho_near.get_unchecked_mut(a) += q2 * q;
                        *rho.get_unchecked_mut(b) += q2;
                        *rho_near.get_unchecked_mut(b) += q2 * q;
                    }
                    // viscosity: relative separating velocity along r (a -> b)
                    let ux = dx / r;
                    let uy = dy / r;
                    let uz = dz / r;
                    let u = unsafe {
                        (*vel.get_unchecked(ob) - *vel.get_unchecked(oa)) * ux
                            + (*vel.get_unchecked(ob + 1) - *vel.get_unchecked(oa + 1)) * uy
                            + (*vel.get_unchecked(ob + 2) - *vel.get_unchecked(oa + 2)) * uz
                    };
                    if u > 0.0 {
                        // attracts the separating pair; sdt^2 turns the
                        // velocity impulse into its equivalent position delta
                        let mag = 0.5 * sdt * sdt * q * (sigma * u + beta * u * u);
                        let (mx, my, mz) = (ux * mag, uy * mag, uz * mag);
                        unsafe {
                            *pos.get_unchecked_mut(ob) -= mx;
                            *pos.get_unchecked_mut(ob + 1) -= my;
                            *pos.get_unchecked_mut(ob + 2) -= mz;
                            *pos.get_unchecked_mut(oa) += mx;
                            *pos.get_unchecked_mut(oa + 1) += my;
                            *pos.get_unchecked_mut(oa + 2) += mz;
                        }
                    }
                }
            }};
        }

        for c in 0..cells {
            let s0 = cell_start[c] as usize;
            let s1 = cell_start[c + 1] as usize;
            if s0 == s1 {
                continue;
            }
            for a in s0..s1 {
                for b in (a + 1)..s1 {
                    pair!(a, b);
                }
            }
            for f in fwd_start[c] as usize..fwd_start[c + 1] as usize {
                let c2 = fwd_list[f] as usize;
                let t0 = cell_start[c2] as usize;
                let t1 = cell_start[c2 + 1] as usize;
                if t0 == t1 {
                    continue;
                }
                for a in s0..s1 {
                    for b in t0..t1 {
                        pair!(a, b);
                    }
                }
            }
        }
    }

    /// Double density relaxation: pressure-driven pair displacement.
    /// Pairwise form D = dt^2/2 * ((P_a+P_b) q + (Pn_a+Pn_b) q^2) applied
    /// symmetrically — identical to the ordered two-visit scheme, half the work.
    fn displacement_pass(&mut self, sdt: f32) {
        let h = self.h;
        let h2 = h * h;
        let inv_h = 1.0 / h;
        let dt2 = sdt * sdt;
        let (k, k_near, rho0) = (self.k, self.k_near, self.rho0);
        let cells = self.cell_start.len() - 1;
        let pos = self.pos.as_mut_slice();
        let rho = self.rho.as_slice();
        let rho_near = self.rho_near.as_slice();
        let cell_start = self.cell_start.as_slice();
        let fwd_start = self.fwd_start.as_slice();
        let fwd_list = self.fwd_list.as_slice();

        macro_rules! pair {
            ($a:expr, $b:expr) => {{
                let (a, b) = ($a, $b);
                let (oa, ob) = (a * 3, b * 3);
                // SAFETY: see pair_pass
                let dx = unsafe { *pos.get_unchecked(ob) - *pos.get_unchecked(oa) };
                let dy = unsafe { *pos.get_unchecked(ob + 1) - *pos.get_unchecked(oa + 1) };
                let dz = unsafe { *pos.get_unchecked(ob + 2) - *pos.get_unchecked(oa + 2) };
                let r2 = dx * dx + dy * dy + dz * dz;
                if r2 < h2 && r2 > 1e-12 {
                    let r = r2.sqrt();
                    let q = 1.0 - r * inv_h;
                    let d = unsafe {
                        0.5
                            * dt2
                            * ((k
                                * (*rho.get_unchecked(a) + *rho.get_unchecked(b) - 2.0 * rho0))
                                * q
                                + k_near
                                    * (*rho_near.get_unchecked(a) + *rho_near.get_unchecked(b))
                                    * q
                                    * q)
                    };
                    let s = d / r;
                    let (sx, sy, sz) = (dx * s, dy * s, dz * s);
                    unsafe {
                        *pos.get_unchecked_mut(ob) += sx;
                        *pos.get_unchecked_mut(ob + 1) += sy;
                        *pos.get_unchecked_mut(ob + 2) += sz;
                        *pos.get_unchecked_mut(oa) -= sx;
                        *pos.get_unchecked_mut(oa + 1) -= sy;
                        *pos.get_unchecked_mut(oa + 2) -= sz;
                    }
                }
            }};
        }

        for c in 0..cells {
            let s0 = cell_start[c] as usize;
            let s1 = cell_start[c + 1] as usize;
            if s0 == s1 {
                continue;
            }
            for a in s0..s1 {
                for b in (a + 1)..s1 {
                    pair!(a, b);
                }
            }
            for f in fwd_start[c] as usize..fwd_start[c + 1] as usize {
                let c2 = fwd_list[f] as usize;
                let t0 = cell_start[c2] as usize;
                let t1 = cell_start[c2 + 1] as usize;
                if t0 == t1 {
                    continue;
                }
                for a in s0..s1 {
                    for b in t0..t1 {
                        pair!(a, b);
                    }
                }
            }
        }
    }

    fn boundary(&mut self) {
        let (mn, mx) = (self.bmin, self.bmax);
        for i in 0..self.n * 3 {
            let axis = i % 3;
            if self.pos[i] < mn[axis] {
                self.pos[i] = mn[axis];
            } else if self.pos[i] > mx[axis] {
                self.pos[i] = mx[axis];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step_timed(s: &mut Sph, sdt: f32, t: &mut [f64; 3]) {
        s.external_forces(sdt, None);
        s.prev.copy_from_slice(&s.pos);
        for i in 0..s.n * 3 {
            let v = s.vel[i];
            s.pos[i] += v * sdt;
        }
        let t0 = std::time::Instant::now();
        s.build_grid();
        let t1 = std::time::Instant::now();
        s.pair_pass(sdt);
        let t2 = std::time::Instant::now();
        s.displacement_pass(sdt);
        let t3 = std::time::Instant::now();
        s.boundary();
        let inv = 1.0 / sdt;
        for i in 0..s.n * 3 {
            s.vel[i] = (s.pos[i] - s.prev[i]) * inv;
        }
        t[0] += t1.duration_since(t0).as_secs_f64();
        t[1] += t2.duration_since(t1).as_secs_f64();
        t[2] += t3.duration_since(t2).as_secs_f64();
    }

    fn diagnostics(s: &Sph) -> String {
        let cells = s.cell_start.len() - 1;
        let mut max_occ = 0usize;
        let mut pairs = 0u64;
        for c in 0..cells {
            let occ = (s.cell_start[c + 1] - s.cell_start[c]) as usize;
            max_occ = max_occ.max(occ);
            pairs += (occ * (occ - 1) / 2) as u64;
            for f in s.fwd_start[c] as usize..s.fwd_start[c + 1] as usize {
                let c2 = s.fwd_list[f] as usize;
                let o2 = (s.cell_start[c2 + 1] - s.cell_start[c2]) as usize;
                pairs += (occ * o2) as u64;
            }
        }
        let rho_mean = s.rho.iter().sum::<f32>() / s.n as f32;
        let vel_max = s
            .vel
            .chunks(3)
            .map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt())
            .fold(0.0f32, f32::max);
        let finite = s.pos.iter().all(|p| p.is_finite());
        format!(
            "cells={} max_occ={} pairs={} rho_mean={:.2} rho0={:.2} vel_max={:.2} finite={}",
            cells, max_occ, pairs, rho_mean, s.rho0, vel_max, finite
        )
    }

    #[test]
    fn perf_and_sanity_12k() {
        let mut s = Sph::new(12000);
        let mut t = [0.0f64; 3];
        let frames = 360; // 6 simulated seconds
        for _ in 0..frames {
            step_timed(&mut s, STEP, &mut t);
        }
        println!("[after 6s] {}", diagnostics(&s));
        let per = |x: f64| x / frames as f64 * 1000.0;
        println!(
            "grid={:.2}ms pair={:.2}ms disp={:.2}ms total={:.2}ms/frame",
            per(t[0]),
            per(t[1]),
            per(t[2]),
            per(t[0] + t[1] + t[2])
        );
        assert!(s.pos.iter().all(|p| p.is_finite()), "NaN in positions");
        assert!(per(t[0] + t[1] + t[2]) < 20.0, "step too slow on native");
    }

    #[test]
    fn settles_to_rest() {
        let mut s = Sph::new(6000);
        for _ in 0..900 {
            // 15 simulated seconds: dam collapses and settles
            s.update(1.0 / 60.0, None);
        }
        let ke: f32 = s
            .vel
            .chunks(3)
            .map(|v| 0.5 * (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]))
            .sum();
        let rho_mean = s.rho.iter().sum::<f32>() / s.n as f32;
        println!(
            "[settled] ke_total={:.3} rho_mean={:.2} rho0={:.2} {}",
            ke,
            rho_mean,
            s.rho0,
            diagnostics(&s)
        );
        // settled fluid: low kinetic energy, density near rest density
        assert!(ke / (s.n as f32) < 0.1, "still boiling: ke={}", ke);
        assert!(
            (rho_mean / s.rho0 - 1.0).abs() < 0.25,
            "density off: mean={} rho0={}",
            rho_mean,
            s.rho0
        );
    }
}
