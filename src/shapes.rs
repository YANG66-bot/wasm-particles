use crate::math::hash01;

pub const SHAPE_NAMES: [&str; 5] = [
    "球面星轨",
    "环面结",
    "银河旋臂",
    "DNA 双螺旋",
    "洛伦兹之云",
];

pub const SHAPE_COUNT: usize = SHAPE_NAMES.len();

/// Generate target points (n * 3) and palette params (n) for shape `idx`.
pub fn gen_shape(idx: usize, n: usize, out: &mut Vec<f32>, extra: &mut Vec<f32>) {
    out.clear();
    out.resize(n * 3, 0.0);
    extra.clear();
    extra.resize(n, 0.0);
    match idx % SHAPE_COUNT {
        0 => sphere(n, out, extra),
        1 => torus_knot(n, out, extra),
        2 => galaxy(n, out, extra),
        3 => helix(n, out, extra),
        _ => lorenz(n, out, extra),
    }
}

fn sphere(n: usize, out: &mut Vec<f32>, extra: &mut Vec<f32>) {
    let ga = std::f32::consts::PI * (3.0 - (5.0f32).sqrt());
    for i in 0..n {
        let y = 1.0 - 2.0 * (i as f32 + 0.5) / n as f32;
        let r = (1.0 - y * y).max(0.0).sqrt();
        let phi = ga * i as f32;
        let s = 0.62;
        out[i * 3] = phi.cos() * r * s;
        out[i * 3 + 1] = y * s;
        out[i * 3 + 2] = phi.sin() * r * s;
        extra[i] = (y * 0.5 + 0.5 + (hash01(i as u32 * 31 + 7) - 0.5) * 0.06).clamp(0.0, 1.0);
    }
}

fn torus_knot(n: usize, out: &mut Vec<f32>, extra: &mut Vec<f32>) {
    let (p, q) = (2.0f32, 3.0f32);
    for i in 0..n {
        let t = (i as f32 + 0.5) / n as f32 * std::f32::consts::TAU;
        let r = 2.0 + (q * t).cos();
        let s = 0.27;
        let j = 0.006;
        out[i * 3] = r * (p * t).cos() * s + (hash01(i as u32 * 17 + 1) - 0.5) * j;
        out[i * 3 + 1] = (q * t).sin() * s + (hash01(i as u32 * 19 + 2) - 0.5) * j;
        out[i * 3 + 2] = r * (p * t).sin() * s + (hash01(i as u32 * 23 + 3) - 0.5) * j;
        extra[i] = (t / std::f32::consts::TAU + (hash01(i as u32 * 29 + 4) - 0.5) * 0.04)
            .clamp(0.0, 1.0);
    }
}

fn galaxy(n: usize, out: &mut Vec<f32>, extra: &mut Vec<f32>) {
    let arms = 4.0f32;
    for i in 0..n {
        let u = hash01(i as u32 * 101 + 11);
        let r = 0.05 + 0.66 * u.powf(0.62);
        let arm = (i % arms as usize) as f32;
        let spread = 0.55 / (1.0 + 3.0 * r);
        let ang = arm * (std::f32::consts::TAU / arms)
            + r * 4.6
            + (hash01(i as u32 * 103 + 13) - 0.5) * spread;
        let thick = 0.1 * (1.25 - r).max(0.06);
        out[i * 3] = ang.cos() * r;
        out[i * 3 + 1] = (hash01(i as u32 * 107 + 17) - 0.5) * thick;
        out[i * 3 + 2] = ang.sin() * r;
        extra[i] = (u + (hash01(i as u32 * 109 + 19) - 0.5) * 0.05).clamp(0.0, 1.0);
    }
}

fn helix(n: usize, out: &mut Vec<f32>, extra: &mut Vec<f32>) {
    let turns = 3.2f32;
    let radius = 0.26f32;
    let height = 1.15f32;
    for i in 0..n {
        let u = hash01(i as u32 * 211 + 5);
        let v = hash01(i as u32 * 223 + 9);
        if v < 0.78 {
            // strands
            let strand = (i % 2) as f32;
            let t = u * turns * std::f32::consts::TAU;
            out[i * 3] = (t + strand * std::f32::consts::PI).cos() * radius;
            out[i * 3 + 1] = (u - 0.5) * height;
            out[i * 3 + 2] = (t + strand * std::f32::consts::PI).sin() * radius;
        } else {
            // rungs
            let y = (u - 0.5) * height;
            let t = u * turns * std::f32::consts::TAU;
            let a = [t.cos() * radius, y, t.sin() * radius];
            let b = [
                (t + std::f32::consts::PI).cos() * radius,
                y,
                (t + std::f32::consts::PI).sin() * radius,
            ];
            let w = hash01(i as u32 * 227 + 21);
            out[i * 3] = a[0] + (b[0] - a[0]) * w;
            out[i * 3 + 1] = y;
            out[i * 3 + 2] = a[2] + (b[2] - a[2]) * w;
        }
        extra[i] = (u + (hash01(i as u32 * 229 + 23) - 0.5) * 0.05).clamp(0.0, 1.0);
    }
}

fn lorenz(n: usize, out: &mut Vec<f32>, extra: &mut Vec<f32>) {
    const K: usize = 256; // trajectories
    const STEPS: usize = 256;
    let s = 0.028f32;
    let mut table = vec![0.0f32; K * STEPS * 3];
    for k in 0..K {
        // scattered seeds on the attractor
        let mut p = [
            0.1 + (hash01(k as u32 * 3 + 1) - 0.5) * 0.2,
            0.0 + (hash01(k as u32 * 5 + 2) - 0.5) * 0.2,
            20.0 + (hash01(k as u32 * 7 + 3) - 0.5) * 4.0,
        ];
        let (sig, rho, bet) = (10.0f32, 28.0, 8.0 / 3.0);
        let dt = 0.006;
        // burn-in
        for _ in 0..300 {
            let d = [
                sig * (p[1] - p[0]),
                p[0] * (rho - p[2]) - p[1],
                p[0] * p[1] - bet * p[2],
            ];
            p[0] += d[0] * dt;
            p[1] += d[1] * dt;
            p[2] += d[2] * dt;
        }
        for st in 0..STEPS {
            for _ in 0..3 {
                let d = [
                    sig * (p[1] - p[0]),
                    p[0] * (rho - p[2]) - p[1],
                    p[0] * p[1] - bet * p[2],
                ];
                p[0] += d[0] * dt;
                p[1] += d[1] * dt;
                p[2] += d[2] * dt;
            }
            let o = (k * STEPS + st) * 3;
            table[o] = p[0] * s;
            table[o + 1] = (p[2] - 25.5) * s;
            table[o + 2] = p[1] * s;
        }
    }
    for i in 0..n {
        let k = (hash01(i as u32 * 13 + 41) * K as f32) as usize % K;
        let st = (hash01(i as u32 * 17 + 43) * STEPS as f32) as usize % STEPS;
        let o = (k * STEPS + st) * 3;
        let j = 0.004;
        out[i * 3] = table[o] + (hash01(i as u32 * 19 + 47) - 0.5) * j;
        out[i * 3 + 1] = table[o + 1] + (hash01(i as u32 * 23 + 51) - 0.5) * j;
        out[i * 3 + 2] = table[o + 2] + (hash01(i as u32 * 29 + 53) - 0.5) * j;
        extra[i] = ((st as f32 / STEPS as f32) + (hash01(i as u32 * 31 + 59) - 0.5) * 0.05)
            .clamp(0.0, 1.0);
    }
}

/// Random spawn cloud (shell) used as the start position for morph mode.
pub fn spawn_cloud(n: usize, out: &mut Vec<f32>) {
    out.clear();
    out.resize(n * 3, 0.0);
    for i in 0..n {
        let u = hash01(i as u32 * 61 + 7) * 2.0 - 1.0;
        let phi = hash01(i as u32 * 67 + 9) * std::f32::consts::TAU;
        let r = 1.05 + hash01(i as u32 * 71 + 13) * 0.75;
        let s = (1.0 - u * u).max(0.0).sqrt();
        out[i * 3] = phi.cos() * s * r;
        out[i * 3 + 1] = u * r;
        out[i * 3 + 2] = phi.sin() * s * r;
    }
}
