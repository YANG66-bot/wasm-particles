use crate::math::Mat4;
use wasm_bindgen::JsCast;
use web_sys::{WebGl2RenderingContext, WebGlProgram, WebGlShader, HtmlCanvasElement};

const PARTICLE_VS: &str = r#"#version 300 es
precision highp float;
layout(location=0) in vec3 a_pos;
layout(location=1) in vec3 a_vel;
layout(location=2) in float a_extra;
uniform mat4 u_vp;
uniform float u_size_px;
out float v_extra;
out float v_speed;
void main() {
    vec4 clip = u_vp * vec4(a_pos, 1.0);
    gl_Position = clip;
    gl_PointSize = clamp(u_size_px / max(clip.w, 1e-4), 1.5, 110.0);
    v_extra = a_extra;
    v_speed = length(a_vel);
}"#;

const PARTICLE_FS: &str = r#"#version 300 es
precision highp float;
in float v_extra;
in float v_speed;
uniform vec3 u_col_a;
uniform vec3 u_col_b;
uniform vec3 u_col_c;
uniform vec3 u_col_d;
uniform float u_intensity;
uniform float u_speed_glow;
uniform float u_time;
out vec4 out_color;
void main() {
    vec2 p = gl_PointCoord - 0.5;
    float d = length(p);
    if (d > 0.5) discard;
    float soft = smoothstep(0.5, 0.0, d);
    float core = pow(soft, 3.0);
    float tw = 0.8 + 0.2 * sin(u_time * 2.3 + v_extra * 41.0);
    vec3 col = u_col_a + u_col_b * cos(6.28318 * (u_col_c * v_extra + u_col_d));
    float boost = 1.0 + v_speed * u_speed_glow;
    out_color = vec4(col * (soft * 0.32 + core) * u_intensity * boost * tw, 1.0);
}"#;

const SCREEN_VS: &str = r#"#version 300 es
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}"#;

const FADE_FS: &str = r#"#version 300 es
precision highp float;
in vec2 v_uv;
uniform float u_fade;
out vec4 out_color;
void main() {
    out_color = vec4(0.0, 0.0, 0.0, u_fade);
}"#;

const PRESENT_FS: &str = r#"#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_tex;
uniform float u_exposure;
out vec4 out_color;
void main() {
    vec3 c = texture(u_tex, v_uv).rgb;
    c = max(c - 0.008, 0.0);
    c = vec3(1.0) - exp(-c * u_exposure);
    c = pow(c, vec3(0.9));
    float r = length(v_uv - 0.5);
    c *= 1.0 - 0.42 * r * r;
    out_color = vec4(c, 1.0);
}"#;

pub struct DrawParams {
    pub vp: Mat4,
    pub size_px: f32,
    pub intensity: f32,
    pub speed_glow: f32,
    pub time: f32,
    pub col_a: [f32; 3],
    pub col_b: [f32; 3],
    pub col_c: [f32; 3],
    pub col_d: [f32; 3],
}

pub struct Renderer {
    gl: WebGl2RenderingContext,
    particle_prog: WebGlProgram,
    fade_prog: WebGlProgram,
    present_prog: WebGlProgram,
    vao: web_sys::WebGlVertexArrayObject,
    screen_vao: web_sys::WebGlVertexArrayObject,
    vbo: web_sys::WebGlBuffer,
    fbo: web_sys::WebGlFramebuffer,
    accum_tex: web_sys::WebGlTexture,
    pub fb_w: i32,
    pub fb_h: i32,
    hdr: bool,
}

impl Renderer {
    pub fn new(canvas: &HtmlCanvasElement) -> Result<Renderer, String> {
        let gl: WebGl2RenderingContext = canvas
            .get_context("webgl2")
            .map_err(|e| format!("get_context failed: {:?}", e))?
            .ok_or_else(|| "当前浏览器不支持 WebGL2".to_string())?
            .dyn_into::<WebGl2RenderingContext>()
            .map_err(|_| "WebGL2 context cast failed".to_string())?;

        let particle_prog = compile(&gl, PARTICLE_VS, PARTICLE_FS)?;
        let fade_prog = compile(&gl, SCREEN_VS, FADE_FS)?;
        let present_prog = compile(&gl, SCREEN_VS, PRESENT_FS)?;

        let vao = gl
            .create_vertex_array()
            .ok_or_else(|| "create_vertex_array failed".to_string())?;
        let vbo = gl
            .create_buffer()
            .ok_or_else(|| "create_buffer failed".to_string())?;
        gl.bind_vertex_array(Some(&vao));
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&vbo));
        // interleaved: pos(3) vel(3) extra(1) = 7 floats = 28 bytes
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_with_i32(0, 3, WebGl2RenderingContext::FLOAT, false, 28, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_with_i32(1, 3, WebGl2RenderingContext::FLOAT, false, 28, 12);
        gl.enable_vertex_attrib_array(2);
        gl.vertex_attrib_pointer_with_i32(2, 1, WebGl2RenderingContext::FLOAT, false, 28, 24);
        gl.bind_vertex_array(None);

        let screen_vao = gl
            .create_vertex_array()
            .ok_or_else(|| "create screen vao failed".to_string())?;

        let fbo = gl
            .create_framebuffer()
            .ok_or_else(|| "create_framebuffer failed".to_string())?;
        let accum_tex = gl
            .create_texture()
            .ok_or_else(|| "create_texture failed".to_string())?;

        let hdr = gl
            .get_extension("EXT_color_buffer_float")
            .map(|e| e.is_some())
            .unwrap_or(false);

        gl.enable(WebGl2RenderingContext::BLEND);

        Ok(Renderer {
            gl,
            particle_prog,
            fade_prog,
            present_prog,
            vao,
            screen_vao,
            vbo,
            fbo,
            accum_tex,
            fb_w: 0,
            fb_h: 0,
            hdr,
        })
    }

    pub fn hdr(&self) -> bool {
        self.hdr
    }

    /// (Re-)allocate the particle VBO.
    pub fn alloc_particles(&self, n: usize) {
        let gl = &self.gl;
        let zeros = vec![0.0f32; n * 7];
        // SAFETY: the view is only used for the duration of this call, while
        // `zeros` is alive and does not move.
        let view = unsafe { js_sys::Float32Array::view(&zeros) };
        let obj = js_sys::Object::from(view);
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&self.vbo));
        gl.buffer_data_with_array_buffer_view(
            WebGl2RenderingContext::ARRAY_BUFFER,
            &obj,
            WebGl2RenderingContext::DYNAMIC_DRAW,
        );
    }

    pub fn upload_particles(&self, data: &[f32]) {
        let gl = &self.gl;
        // SAFETY: zero-copy view into WASM linear memory; `data` outlives the
        // GL call and is not mutated concurrently (single-threaded).
        let view = unsafe { js_sys::Float32Array::view(data) };
        let obj = js_sys::Object::from(view);
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&self.vbo));
        gl.buffer_sub_data_with_i32_and_array_buffer_view(
            WebGl2RenderingContext::ARRAY_BUFFER,
            0,
            &obj,
        );
    }

    pub fn resize(&mut self, w: i32, h: i32) {
        if w == self.fb_w && h == self.fb_h {
            return;
        }
        if w <= 0 || h <= 0 {
            return;
        }
        let gl = &self.gl;
        self.fb_w = w;
        self.fb_h = h;
        let (internal, format, ty) = if self.hdr {
            (
                WebGl2RenderingContext::RGBA16F,
                WebGl2RenderingContext::RGBA,
                WebGl2RenderingContext::HALF_FLOAT,
            )
        } else {
            (
                WebGl2RenderingContext::RGBA8,
                WebGl2RenderingContext::RGBA,
                WebGl2RenderingContext::UNSIGNED_BYTE,
            )
        };
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&self.accum_tex));
        gl.tex_storage_2d(
            WebGl2RenderingContext::TEXTURE_2D,
            1,
            internal,
            w,
            h,
        );
        let _ = format;
        let _ = ty;
        gl.tex_parameteri(
            WebGl2RenderingContext::TEXTURE_2D,
            WebGl2RenderingContext::TEXTURE_MIN_FILTER,
            WebGl2RenderingContext::LINEAR as i32,
        );
        gl.tex_parameteri(
            WebGl2RenderingContext::TEXTURE_2D,
            WebGl2RenderingContext::TEXTURE_MAG_FILTER,
            WebGl2RenderingContext::LINEAR as i32,
        );
        gl.tex_parameteri(
            WebGl2RenderingContext::TEXTURE_2D,
            WebGl2RenderingContext::TEXTURE_WRAP_S,
            WebGl2RenderingContext::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameteri(
            WebGl2RenderingContext::TEXTURE_2D,
            WebGl2RenderingContext::TEXTURE_WRAP_T,
            WebGl2RenderingContext::CLAMP_TO_EDGE as i32,
        );
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, Some(&self.fbo));
        gl.framebuffer_texture_2d(
            WebGl2RenderingContext::FRAMEBUFFER,
            WebGl2RenderingContext::COLOR_ATTACHMENT0,
            WebGl2RenderingContext::TEXTURE_2D,
            Some(&self.accum_tex),
            0,
        );
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, None);
        self.clear_accum();
    }

    pub fn clear_accum(&self) {
        let gl = &self.gl;
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, Some(&self.fbo));
        gl.viewport(0, 0, self.fb_w, self.fb_h);
        gl.clear_color(0.0, 0.0, 0.0, 1.0);
        gl.clear(WebGl2RenderingContext::COLOR_BUFFER_BIT);
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, None);
    }

    /// Begin frame: fade the accumulation buffer (motion trails), keep additive space.
    pub fn begin_frame(&self, fade: f32) {
        let gl = &self.gl;
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, Some(&self.fbo));
        gl.viewport(0, 0, self.fb_w, self.fb_h);
        gl.use_program(Some(&self.fade_prog));
        gl.bind_vertex_array(Some(&self.screen_vao));
        if let Some(loc) = gl.get_uniform_location(&self.fade_prog, "u_fade") {
            gl.uniform1f(Some(&loc), fade);
        }
        gl.blend_func(
            WebGl2RenderingContext::SRC_ALPHA,
            WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA,
        );
        gl.draw_arrays(WebGl2RenderingContext::TRIANGLES, 0, 3);
    }

    pub fn draw_particles(&self, n: usize, p: &DrawParams) {
        if n == 0 {
            return;
        }
        let gl = &self.gl;
        gl.use_program(Some(&self.particle_prog));
        gl.bind_vertex_array(Some(&self.vao));
        if let Some(loc) = gl.get_uniform_location(&self.particle_prog, "u_vp") {
            gl.uniform_matrix4fv_with_f32_array(Some(&loc), false, &p.vp.0);
        }
        if let Some(loc) = gl.get_uniform_location(&self.particle_prog, "u_size_px") {
            gl.uniform1f(Some(&loc), p.size_px);
        }
        if let Some(loc) = gl.get_uniform_location(&self.particle_prog, "u_intensity") {
            gl.uniform1f(Some(&loc), p.intensity);
        }
        if let Some(loc) = gl.get_uniform_location(&self.particle_prog, "u_speed_glow") {
            gl.uniform1f(Some(&loc), p.speed_glow);
        }
        if let Some(loc) = gl.get_uniform_location(&self.particle_prog, "u_time") {
            gl.uniform1f(Some(&loc), p.time);
        }
        if let Some(loc) = gl.get_uniform_location(&self.particle_prog, "u_col_a") {
            gl.uniform3f(Some(&loc), p.col_a[0], p.col_a[1], p.col_a[2]);
        }
        if let Some(loc) = gl.get_uniform_location(&self.particle_prog, "u_col_b") {
            gl.uniform3f(Some(&loc), p.col_b[0], p.col_b[1], p.col_b[2]);
        }
        if let Some(loc) = gl.get_uniform_location(&self.particle_prog, "u_col_c") {
            gl.uniform3f(Some(&loc), p.col_c[0], p.col_c[1], p.col_c[2]);
        }
        if let Some(loc) = gl.get_uniform_location(&self.particle_prog, "u_col_d") {
            gl.uniform3f(Some(&loc), p.col_d[0], p.col_d[1], p.col_d[2]);
        }
        gl.blend_func(WebGl2RenderingContext::ONE, WebGl2RenderingContext::ONE);
        gl.draw_arrays(WebGl2RenderingContext::POINTS, 0, n as i32);
    }

    /// Tone-map the accumulation buffer to the screen.
    pub fn end_frame(&self, exposure: f32) {
        let gl = &self.gl;
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, None);
        gl.viewport(0, 0, self.fb_w, self.fb_h);
        gl.disable(WebGl2RenderingContext::BLEND);
        gl.use_program(Some(&self.present_prog));
        gl.bind_vertex_array(Some(&self.screen_vao));
        gl.active_texture(WebGl2RenderingContext::TEXTURE0);
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&self.accum_tex));
        if let Some(loc) = gl.get_uniform_location(&self.present_prog, "u_tex") {
            gl.uniform1i(Some(&loc), 0);
        }
        if let Some(loc) = gl.get_uniform_location(&self.present_prog, "u_exposure") {
            gl.uniform1f(Some(&loc), exposure);
        }
        gl.draw_arrays(WebGl2RenderingContext::TRIANGLES, 0, 3);
        gl.enable(WebGl2RenderingContext::BLEND);
    }
}

fn compile(
    gl: &WebGl2RenderingContext,
    vs_src: &str,
    fs_src: &str,
) -> Result<WebGlProgram, String> {
    let vs = shader(gl, WebGl2RenderingContext::VERTEX_SHADER, vs_src)?;
    let fs = shader(gl, WebGl2RenderingContext::FRAGMENT_SHADER, fs_src)?;
    let prog = gl
        .create_program()
        .ok_or_else(|| "create_program failed".to_string())?;
    gl.attach_shader(&prog, &vs);
    gl.attach_shader(&prog, &fs);
    gl.link_program(&prog);
    if !gl
        .get_program_parameter(&prog, WebGl2RenderingContext::LINK_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        let log = gl.get_program_info_log(&prog).unwrap_or_default();
        return Err(format!("link error: {}", log));
    }
    Ok(prog)
}

fn shader(gl: &WebGl2RenderingContext, ty: u32, src: &str) -> Result<WebGlShader, String> {
    let sh = gl
        .create_shader(ty)
        .ok_or_else(|| "create_shader failed".to_string())?;
    gl.shader_source(&sh, src);
    gl.compile_shader(&sh);
    if !gl
        .get_shader_parameter(&sh, WebGl2RenderingContext::COMPILE_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        let log = gl.get_shader_info_log(&sh).unwrap_or_default();
        return Err(format!("shader compile error: {}", log));
    }
    Ok(sh)
}
