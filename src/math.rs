#![allow(dead_code)]

pub type V3 = [f32; 3];

#[inline]
pub fn sub(a: &V3, b: &V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
pub fn add(a: &V3, b: &V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
pub fn scale(a: &V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[inline]
pub fn dot(a: &V3, b: &V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
pub fn cross(a: &V3, b: &V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
pub fn len(a: &V3) -> f32 {
    dot(a, a).sqrt()
}

#[inline]
pub fn norm(a: &V3) -> V3 {
    let l = len(a);
    if l > 1e-9 {
        [a[0] / l, a[1] / l, a[2] / l]
    } else {
        [0.0, 1.0, 0.0]
    }
}

/// 4x4 column-major matrix, WebGL layout.
#[derive(Clone, Copy)]
pub struct Mat4(pub [f32; 16]);

impl Mat4 {
    pub fn identity() -> Mat4 {
        Mat4([
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
        let f = 1.0 / (fov_y * 0.5).tan();
        let nf = 1.0 / (near - far);
        let mut m = [0.0f32; 16];
        m[0] = f / aspect;
        m[5] = f;
        m[10] = (far + near) * nf;
        m[11] = -1.0;
        m[14] = 2.0 * far * near * nf;
        Mat4(m)
    }

    pub fn look_at(eye: &V3, center: &V3, up: &V3) -> Mat4 {
        let z = norm(&sub(eye, center)); // forward = -z
        let x = norm(&cross(up, &z));
        let y = cross(&z, &x);
        Mat4([
            x[0], y[0], z[0], 0.0, //
            x[1], y[1], z[1], 0.0, //
            x[2], y[2], z[2], 0.0, //
            -dot(&x, eye), -dot(&y, eye), -dot(&z, eye), 1.0,
        ])
    }

    /// self = a * b (apply b first, then a)
    pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
        let mut m = [0.0f32; 16];
        for c in 0..4usize {
            for r in 0..4usize {
                let mut s = 0.0;
                for k in 0..4usize {
                    s += a.0[k * 4 + r] * b.0[c * 4 + k];
                }
                m[c * 4 + r] = s;
            }
        }
        Mat4(m)
    }
}

/// Deterministic hash -> [0,1)
#[inline]
pub fn hash01(i: u32) -> f32 {
    let mut x = i.wrapping_mul(2654435761);
    x ^= x >> 16;
    x = x.wrapping_mul(2246822519);
    x ^= x >> 13;
    (x & 0x00ffffff) as f32 / 16777216.0
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}
