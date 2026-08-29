import init, {
  start,
  set_mode,
  set_count,
  set_text_points,
  set_param,
  set_paused,
  reset,
  next_shape,
  get_stats,
} from './pkg/wasm_particles.js';

const SHAPE_NAMES = ['球面星轨', '环面结', '银河旋臂', 'DNA 双螺旋', '洛伦兹之云'];

let mode = 0;

async function main() {
  await init();
  const canvas = document.getElementById('view');
  try {
    start(canvas, 12000);
  } catch (e) {
    showErr('引擎启动失败: ' + (e?.message || e));
    return;
  }

  wireUi();
  setInterval(pollStats, 300);
}

function showErr(msg) {
  const el = document.getElementById('err');
  el.textContent = msg;
  el.classList.remove('hidden');
}

function wireUi() {
  const btns = document.querySelectorAll('.btn.mode');
  btns.forEach((b) =>
    b.addEventListener('click', () => {
      const m = +b.dataset.mode;
      if (m === 2) {
        applyText(document.getElementById('text-input').value || 'RUST+WASM');
      }
      set_mode(m);
      setMode(m);
    })
  );

  document.getElementById('text-go').addEventListener('click', () => {
    applyText(document.getElementById('text-input').value || 'RUST+WASM');
    set_mode(2);
    setMode(2);
  });

  document.getElementById('text-input').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') document.getElementById('text-go').click();
  });

  const slCount = document.getElementById('sl-count');
  slCount.addEventListener('input', () => {
    const v = +slCount.value;
    document.getElementById('lb-count').textContent = v;
    set_count(v);
    if (mode === 2) {
      applyText(document.getElementById('text-input').value || 'RUST+WASM');
    }
  });

  const slGrav = document.getElementById('sl-grav');
  slGrav.addEventListener('input', () => {
    const v = +slGrav.value;
    document.getElementById('lb-grav').textContent = v.toFixed(1);
    set_param('gravity', v);
  });

  const slTurb = document.getElementById('sl-turb');
  slTurb.addEventListener('input', () => {
    const v = +slTurb.value;
    document.getElementById('lb-turb').textContent = v.toFixed(2);
    set_param('turb', v);
  });

  const btPause = document.getElementById('bt-pause');
  let paused = false;
  btPause.addEventListener('click', () => {
    paused = !paused;
    set_paused(paused);
    btPause.textContent = paused ? '继续' : '暂停';
  });

  document.getElementById('bt-reset').addEventListener('click', () => reset());
  document.getElementById('bt-shape').addEventListener('click', () => next_shape());
}

function setMode(m) {
  mode = m;
  document.querySelectorAll('.btn.mode').forEach((b) =>
    b.classList.toggle('active', +b.dataset.mode === m)
  );
  document.getElementById('text-row').classList.toggle('hidden', m !== 2);
  document.getElementById('bt-shape').style.display = m === 1 ? '' : 'none';
}

function pollStats() {
  let s;
  try {
    s = get_stats();
  } catch (e) {
    return;
  }
  if (!s || typeof s.fps !== 'number') return;
  document.getElementById('st-fps').textContent = s.fps.toFixed(0);
  document.getElementById('st-n').textContent = s.particles.toLocaleString();
  document.getElementById('st-sim').textContent = s.sim_ms.toFixed(2) + ' ms';
  document.getElementById('st-ren').textContent = s.render_ms.toFixed(2) + ' ms';
  document.getElementById('st-shape').textContent =
    s.shape >= 100 ? '文字' : SHAPE_NAMES[s.shape] || '-';
}

/// Rasterize text on a 2D canvas and sample `count` particle targets from it.
function rasterizeText(str, count) {
  const W = 760;
  const H = 220;
  const c = document.createElement('canvas');
  c.width = W;
  c.height = H;
  const ctx = c.getContext('2d', { willReadFrequently: true });
  ctx.fillStyle = '#fff';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';

  let size = 150;
  const setFont = () => {
    ctx.font = `900 ${size}px "Arial Black", "PingFang SC", "Microsoft YaHei", sans-serif`;
  };
  setFont();
  while (ctx.measureText(str).width > W - 44 && size > 24) {
    size -= 6;
    setFont();
  }
  ctx.fillText(str, W / 2, H / 2);

  const data = ctx.getImageData(0, 0, W, H).data;
  const lit = [];
  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
      if (data[(y * W + x) * 4 + 3] > 128) lit.push(x, y);
    }
  }
  if (lit.length < 2) return null;
  const total = lit.length / 2;
  if (total < 10) return null;

  const worldW = 1.85;
  const worldH = (worldW * H) / W;
  const out = new Float32Array(count * 3);
  for (let i = 0; i < count; i++) {
    // stratified sampling for even coverage
    const t = (i + 0.5) / count;
    const k = Math.min(total - 1, Math.floor(t * total)) * 2;
    const px = lit[k] + (Math.random() - 0.5) * 1.4;
    const py = lit[k + 1] + (Math.random() - 0.5) * 1.4;
    out[i * 3] = (px / W - 0.5) * worldW;
    out[i * 3 + 1] = -(py / H - 0.5) * worldH;
    out[i * 3 + 2] = (Math.random() - 0.5) * 0.035;
  }
  return out;
}

function applyText(str) {
  const s = get_stats();
  const pts = rasterizeText(str, s.particles);
  if (pts) set_text_points(pts);
}

main().catch((e) => showErr('加载失败: ' + (e?.message || e)));
