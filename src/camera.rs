use crate::math::{cross, norm, Mat4, V3};

pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub target: V3,
    pub fov_y: f32,
    auto_speed: f32,
}

impl Camera {
    pub fn new() -> Camera {
        Camera {
            yaw: 0.6,
            pitch: 0.28,
            dist: 2.6,
            target: [0.0, 0.0, 0.0],
            fov_y: 50.0f32.to_radians(),
            auto_speed: 0.07,
        }
    }

    pub fn eye(&self) -> V3 {
        let cp = self.pitch.cos();
        [
            self.target[0] + self.dist * self.yaw.sin() * cp,
            self.target[1] + self.dist * self.pitch.sin(),
            self.target[2] + self.dist * self.yaw.cos() * cp,
        ]
    }

    /// Camera basis: (right, up, forward)
    pub fn basis(&self) -> (V3, V3, V3) {
        let eye = self.eye();
        let forward = norm(&[
            self.target[0] - eye[0],
            self.target[1] - eye[1],
            self.target[2] - eye[2],
        ]);
        let world_up = [0.0, 1.0, 0.0];
        let right = norm(&cross(&world_up, &forward));
        let up = cross(&forward, &right);
        (right, up, forward)
    }

    pub fn vp(&self, aspect: f32) -> Mat4 {
        let eye = self.eye();
        let view = Mat4::look_at(&eye, &self.target, &[0.0, 1.0, 0.0]);
        let proj = Mat4::perspective(self.fov_y, aspect, 0.05, 20.0);
        Mat4::mul(&proj, &view)
    }

    /// World-space point under the mouse: unproject NDC, intersect the plane
    /// through the scene center facing the camera.
    pub fn world_point(&self, ndc_x: f32, ndc_y: f32, aspect: f32) -> V3 {
        let (right, up, forward) = self.basis();
        let t = (self.fov_y * 0.5).tan();
        let vx = ndc_x * t * aspect;
        let vy = ndc_y * t;
        let dir = norm(&[
            right[0] * vx + up[0] * vy - forward[0],
            right[1] * vx + up[1] * vy - forward[1],
            right[2] * vx + up[2] * vy - forward[2],
        ]);
        let eye = self.eye();
        // plane through target with normal = forward: dot(p - target, forward) = 0
        let denom = dot3(&dir, &forward);
        if denom.abs() < 1e-6 {
            return self.target;
        }
        let d = [
            self.target[0] - eye[0],
            self.target[1] - eye[1],
            self.target[2] - eye[2],
        ];
        let t = dot3(&d, &forward) / denom;
        [
            eye[0] + dir[0] * t,
            eye[1] + dir[1] * t,
            eye[2] + dir[2] * t,
        ]
    }

    pub fn update(&mut self, dt: f32, dragging: bool) {
        if !dragging {
            self.yaw += dt * self.auto_speed;
        }
    }
}

#[inline]
fn dot3(a: &V3, b: &V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
