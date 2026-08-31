// ttfx-bg-rs: 100% Rust audio-reactive desktop background.
//
// Omarchy paints its desktop background as a gtk4-layer-shell bottom layer.
// We do the same, ONE layer per connected monitor. Inside each layer we embed
// a Vte terminal and draw the effect directly. The renderer is a child process
// (`ttfx-bg-rs --render <effect> <cols> <rows> ...`) per monitor.
//
// Config lives in the plugin's state.json (written by the panel / bar widget):
//   running, effect, effects[], intensity, audio, byline, restart
// The parent polls it every 700ms and rebuilds layers on change. A `restart`
// counter bump forces a rebuild (replays the intro). When `effects` has more
// than one entry, the parent rotates the active effect every 20s.
//
// Roadmap:
//   - Step 1: vendor ttfx's effect engine as a lib and call it here instead.
//   - Step 2: (done here) audio level from parec modulates flow speed/density.

use anyhow::Result;
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use vte4::{TerminalExt, TerminalExtManual};

const DEFAULT_EFFECTS: [&str; 8] = ["matrix", "rain", "wave", "bars", "donut", "fire", "starfield", "life"];
const DEFAULT_BYLINE: &str = "By x.com/@avillagran";
const ROTATE_SECS: u64 = 20;

#[derive(Clone, PartialEq)]
struct Config {
    running: bool,
    effect: String,
    effects: Vec<String>,
    intensity: i64,
    audio: bool,
    byline: String,
    restart: i64,
    intro_size: i64,
    show_fps: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            running: true,
            effect: "matrix".into(),
            effects: DEFAULT_EFFECTS.iter().map(|s| s.to_string()).collect(),
            intensity: 5,
            audio: true,
            byline: String::new(),
            restart: 0,
            intro_size: 5,
            show_fps: false,
        }
    }
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".config/omarchy/plugins/io.github.avillagran.omarchy-audio-background/state.json")
}

// Best-effort read of the panel's state.json. Missing/invalid => defaults.
// Tolerant of compact or pretty JSON. Independent `if`s on purpose: a compact
// line carries every key at once.
fn read_config() -> Config {
    let mut cfg = Config::default();
    let text = match std::fs::read_to_string(config_path()) {
        Ok(t) => t,
        Err(_) => return cfg,
    };
    if let Some(v) = json_bool(&text, "running") { cfg.running = v; }
    if let Some(v) = json_bool(&text, "audio") { cfg.audio = v; }
    if let Some(v) = json_str(&text, "effect") { cfg.effect = v; }
    if let Some(v) = json_str(&text, "byline") { cfg.byline = v; }
    if let Some(v) = json_num(&text, "intensity") { cfg.intensity = v; }
    if let Some(v) = json_num(&text, "restart") { cfg.restart = v; }
    if let Some(v) = json_num(&text, "intro_size") { cfg.intro_size = v; }
    if let Some(v) = json_bool(&text, "show_fps") { cfg.show_fps = v; }
    if let Some(v) = json_str_list(&text, "effects") { if !v.is_empty() { cfg.effects = v; } }
    cfg
}

// --- tiny tolerant JSON field extractors (no serde dependency) ---
fn json_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\"");
    let pos = text.find(&pat)?;
    let rest = &text[pos + pat.len()..];
    let colon = rest.find(':')?;
    Some(rest[colon + 1..].trim_start())
}

fn json_bool(text: &str, key: &str) -> Option<bool> {
    let v = json_value(text, key)?;
    if v.starts_with("true") { Some(true) } else if v.starts_with("false") { Some(false) } else { None }
}

fn json_str(text: &str, key: &str) -> Option<String> {
    let v = json_value(text, key)?;
    let inner = v.strip_prefix('"')?;
    let end = inner.find('"')?;
    Some(inner[..end].to_string())
}

fn json_num(text: &str, key: &str) -> Option<i64> {
    let v = json_value(text, key)?;
    let end = v.find(|c: char| !(c.is_ascii_digit() || c == '-'))?;
    v[..end].parse().ok()
}

fn json_str_list(text: &str, key: &str) -> Option<Vec<String>> {
    let v = json_value(text, key)?;
    let inner = v.strip_prefix('[')?;
    let end = inner.find(']')?;
    let body = &inner[..end];
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        match after.find('"') {
            Some(stop) => {
                out.push(after[..stop].to_string());
                rest = &after[stop + 1..];
            }
            None => break,
        }
    }
    Some(out)
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == key { return it.next().cloned(); }
        if let Some(v) = a.strip_prefix(&format!("{key}=")) { return Some(v.to_string()); }
    }
    None
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--render") {
        let effect = arg_value(&args, "--effect").unwrap_or_else(|| "matrix".into());
        let cols = arg_value(&args, "--cols").and_then(|s| s.parse::<usize>().ok()).unwrap_or(200);
        let rows = arg_value(&args, "--rows").and_then(|s| s.parse::<usize>().ok()).unwrap_or(100);
        let intensity = arg_value(&args, "--intensity").and_then(|s| s.parse::<i64>().ok()).unwrap_or(5);
        let audio = arg_value(&args, "--audio").map(|s| s == "1").unwrap_or(false);
        let byline = arg_value(&args, "--byline").unwrap_or_default();
        let intro_size = arg_value(&args, "--intro-size").and_then(|s| s.parse::<i64>().ok()).unwrap_or(5);
        let cell_aspect = arg_value(&args, "--cell-aspect").and_then(|s| s.parse::<f32>().ok()).unwrap_or(2.0);
        let show_fps = arg_value(&args, "--show-fps").map(|s| s == "1").unwrap_or(false);
        return run_render(&effect, cols, rows, intensity, audio, &byline, intro_size, cell_aspect, show_fps);
    }

    gtk4::init()?;
    let self_bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "ttfx-bg-rs".into());

    let windows: Rc<RefCell<Vec<gtk4::ApplicationWindow>>> = Rc::new(RefCell::new(Vec::new()));
    let cfg0 = read_config();
    let active_effect: Rc<RefCell<String>> = Rc::new(RefCell::new(
        if cfg0.effects.contains(&cfg0.effect) { cfg0.effect.clone() }
        else { cfg0.effects.first().cloned().unwrap_or_else(|| "matrix".into()) }
    ));
    let last_cfg: Rc<RefCell<Config>> = Rc::new(RefCell::new(cfg0));

    rebuild_layers(&windows, &self_bin, &last_cfg.borrow(), &active_effect.borrow());

    // Rebuild ONLY when the monitor geometry actually changed (count + size +
    // scale). items-changed also fires on spurious signals; rebuilding on those
    // was the source of visible flicker (screen clear + renderer respawn).
    if let Some(display) = gdk::Display::default() {
        let monitors = display.monitors();
        let w = windows.clone();
        let b = self_bin.clone();
        let lc = last_cfg.clone();
        let ae = active_effect.clone();
        let last_sig: Rc<RefCell<String>> = Rc::new(RefCell::new(monitor_signature()));
        let rebuilding: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
        monitors.connect_items_changed(move |_, _pos, _removed, _added| {
            let sig = monitor_signature();
            if sig == *last_sig.borrow() { return; }
            *last_sig.borrow_mut() = sig;
            if *rebuilding.borrow() { return; }
            *rebuilding.borrow_mut() = true;
            // Debounce: coalesce a burst of items-changed into one rebuild.
            let w = w.clone(); let b = b.clone(); let lc = lc.clone();
            let ae = ae.clone(); let rebuilding = rebuilding.clone();
            glib::timeout_add_local_once(Duration::from_millis(250), move || {
                println!("monitors changed — rebuilding background layers");
                let cfg = read_config();
                *lc.borrow_mut() = cfg.clone();
                rebuild_layers(&w, &b, &cfg, &ae.borrow());
                *rebuilding.borrow_mut() = false;
            });
        });
    }

    // Poll the panel's state.json; if anything changed, rebuild.
    {
        let w = windows.clone();
        let b = self_bin.clone();
        let lc = last_cfg.clone();
        let ae = active_effect.clone();
        glib::timeout_add_local(Duration::from_millis(700), move || {
            let cfg = read_config();
            let changed = *lc.borrow() != cfg;
            if changed {
                let old = lc.borrow().clone();
                // If the active-effect selection changed, honor it; otherwise
                // keep rotating from where we are.
                if cfg.effect != old.effect && cfg.effects.contains(&cfg.effect) {
                    *ae.borrow_mut() = cfg.effect.clone();
                }
                *lc.borrow_mut() = cfg.clone();
                rebuild_layers(&w, &b, &cfg, &ae.borrow());
            }
            glib::ControlFlow::Continue
        });
    }

    // Rotate through the enabled effects when more than one is enabled.
    {
        let w = windows.clone();
        let b = self_bin.clone();
        let lc = last_cfg.clone();
        let ae = active_effect.clone();
        glib::timeout_add_local(Duration::from_secs(ROTATE_SECS), move || {
            let cfg = lc.borrow().clone();
            if cfg.running && cfg.effects.len() > 1 {
                let cur = ae.borrow().clone();
                let next = match cfg.effects.iter().position(|e| *e == cur) {
                    Some(i) => cfg.effects[(i + 1) % cfg.effects.len()].clone(),
                    None => cfg.effects[0].clone(),
                };
                if next != cur {
                    *ae.borrow_mut() = next.clone();
                    rebuild_layers(&w, &b, &cfg, &next);
                }
            }
            glib::ControlFlow::Continue
        });
    }

    glib::MainLoop::new(None, false).run();
    Ok(())
}

// Signature of the current monitor set: count + geometry + scale. Rebuilds
// only happen when this actually changes.
fn monitor_signature() -> String {
    let display = match gdk::Display::default() {
        Some(d) => d,
        None => return String::new(),
    };
    let monitors = display.monitors();
    let mut parts = Vec::new();
    for i in 0..monitors.n_items() {
        if let Some(m) = monitors.item(i).and_downcast::<gdk::Monitor>() {
            let g = m.geometry();
            parts.push(format!("{}x{}@{},{}s{}", g.width(), g.height(), g.x(), g.y(), m.scale_factor()));
        }
    }
    parts.join("|")
}

fn rebuild_layers(
    windows: &Rc<RefCell<Vec<gtk4::ApplicationWindow>>>,
    self_bin: &str,
    cfg: &Config,
    effect: &str,
) {
    // Kill renderer subprocesses from a previous build. The pattern `--render`
    // only matches the child renderers, never this parent binary.
    let _ = std::process::Command::new("pkill")
        .args(["-f", "ttfx-bg-rs-.* --render"])
        .output();
    for w in windows.borrow_mut().drain(..) {
        w.close();
    }
    let display = match gdk::Display::default() {
        Some(d) => d,
        None => return,
    };
    let monitors = display.monitors();
    let n = monitors.n_items();
    println!("found {n} monitor(s), running={} effect={effect}", cfg.running);
    for i in 0..n {
        if let Some(monitor) = monitors.item(i).and_downcast::<gdk::Monitor>() {
            let w = spawn_layer_for_monitor(&monitor, self_bin, cfg, effect);
            windows.borrow_mut().push(w);
        }
    }
}

fn spawn_layer_for_monitor(
    monitor: &gdk::Monitor,
    self_bin: &str,
    cfg: &Config,
    effect: &str,
) -> gtk4::ApplicationWindow {
    let window = gtk4::ApplicationWindow::builder()
        .title("ttfx-bg")
        .build();

    window.init_layer_shell();
    window.set_namespace(Some("ttfx-bg"));
    window.set_layer(Layer::Bottom);
    window.set_monitor(Some(monitor));
    for e in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
        window.set_anchor(e, true);
    }
    window.set_exclusive_zone(-1);

    let term = vte4::Terminal::new();
    // Font size in *physical pixels* (not points) so the grid scales with the
    // panel resolution: GTK applies the monitor scale to point sizes, which on
    // a 4K/scaled display produced a handful of giant characters.
    let scale = monitor.scale_factor().max(1) as f64;
    let cell_px = (9.0 / scale).max(3.0);
    let font = gtk4::gdk::pango::FontDescription::from_string(&format!("monospace {cell_px:.0}px"));
    term.set_font(Some(&font));
    term.set_scrollback_lines(0);
    term.set_hexpand(true);
    term.set_vexpand(true);
    window.set_child(Some(&term));

    if cfg.running {
        let geo = monitor.geometry();
        // Measure the real cell size from the font metrics so the grid fills
        // the screen exactly regardless of DPI / scale factor.
        let font_size = font.size() as f64 / gtk4::pango::SCALE as f64;
        let ctx = term.pango_context();
        let metrics = ctx.metrics(Some(&font), None);
        let char_w = metrics.approximate_char_width() as f64 / gtk4::pango::SCALE as f64;
        let ascent = metrics.ascent() as f64 / gtk4::pango::SCALE as f64;
        let descent = metrics.descent() as f64 / gtk4::pango::SCALE as f64;
        let cw = if char_w > 0.0 { char_w } else { font_size * 0.6 };
        let ch = if (ascent + descent) > 0.0 { ascent + descent } else { font_size * 1.2 };
        let cols = ((geo.width() as f64) / cw).floor().max(80.0) as usize;
        let rows = ((geo.height() as f64) / ch).floor().max(24.0) as usize;
        // Cell aspect (height/width in px) so effects can draw true circles.
        let cell_aspect = if cw > 0.0 { ch / cw } else { 2.0 };
        let byline = if cfg.byline.trim().is_empty() { DEFAULT_BYLINE } else { cfg.byline.trim() }.to_string();
        let audio = if cfg.audio { "1" } else { "0" };
        let argv: Vec<String> = vec![
            self_bin.to_string(),
            "--render".into(),
            "--effect".into(), effect.to_string(),
            "--cols".into(), cols.to_string(),
            "--rows".into(), rows.to_string(),
            "--intensity".into(), cfg.intensity.to_string(),
            "--audio".into(), audio.into(),
            "--byline".into(), byline,
            "--intro-size".into(), cfg.intro_size.to_string(),
            "--cell-aspect".into(), format!("{cell_aspect:.3}"),
            "--show-fps".into(), if cfg.show_fps { "1".into() } else { "0".into() },
        ];
        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
        // Pre-clear so Vte doesn't flash its "N by M cells" placeholder while
        // the renderer's PTY is still connecting.
        term.feed(b"\x1b[2J\x1b[H\x1b[40m");
        let effect_for_log = effect.to_string();
        term.spawn_async(
            vte4::PtyFlags::DEFAULT,
            None,
            &argv_refs,
            &[],
            gtk4::glib::SpawnFlags::empty(),
            || {},
            2000,
            None::<&gtk4::gio::Cancellable>,
            move |res| match res {
                Ok(_) => println!("renderer spawned ({cols}x{rows}, effect={effect_for_log})"),
                Err(e) => eprintln!("spawn error: {e:?}"),
            },
        );
    }

    window.present();
    window
}

// ---------------------------------------------------------------------------
// Renderer child process
// ---------------------------------------------------------------------------

// Shared audio state fed by the parec capture thread. Per-band spectrum (like
// the node@192.168.1.177 analyzer_light.py that reacted correctly): Goertzel
// per band with adaptive rolling-peak normalization, plus volume + beat.
const NBANDS: usize = 24;
const SAMPLE_RATE: f32 = 24000.0;
const CHUNK: usize = 1024; // samples per analysis frame (~43ms)

#[derive(Clone)]
struct AudioState {
    bands: Arc<Vec<AtomicU32>>, // per-band energy 0..1 (f32 bits)
    volume: Arc<AtomicU32>,     // overall volume 0..1
    beat: Arc<AtomicU32>,       // 1 shortly after a beat
}

impl AudioState {
    fn start(enabled: bool) -> Self {
        let st = AudioState {
            bands: Arc::new((0..NBANDS).map(|_| AtomicU32::new(0f32.to_bits())).collect()),
            volume: Arc::new(AtomicU32::new(0f32.to_bits())),
            beat: Arc::new(AtomicU32::new(0)),
        };
        if enabled {
            let s = st.clone();
            thread::spawn(move || audio_capture_loop(s));
        }
        st
    }
    fn volume(&self) -> f32 { f32::from_bits(self.volume.load(Ordering::Relaxed)) }
    fn beat(&self) -> bool { self.beat.load(Ordering::Relaxed) != 0 }
    // Band energy for screen column `c` out of `cols` (rain-equalizer mapping).
    fn band_at(&self, c: usize, cols: usize) -> f32 {
        let b = c * NBANDS / cols.max(1);
        f32::from_bits(self.bands[b.min(NBANDS - 1)].load(Ordering::Relaxed))
    }
}

// Goertzel: energy of `freq` in the sample window (no full FFT needed).
fn goertzel(samples: &[f32], rate: f32, freq: f32) -> f32 {
    let n = samples.len();
    let mut k = (0.5 + (n as f32 * freq) / rate) as i32;
    if k <= 0 || k >= n as i32 { k = k.clamp(1, n as i32 - 1); }
    let w = 2.0 * std::f32::consts::PI * k as f32 / n as f32;
    let coeff = 2.0 * w.cos();
    let (mut s_prev, mut s_prev2) = (0f32, 0f32);
    for &x in samples {
        let s = x + coeff * s_prev - s_prev2;
        s_prev2 = s_prev;
        s_prev = s;
    }
    s_prev2 * s_prev2 + s_prev * s_prev - coeff * s_prev * s_prev2
}

// Capture the default sink monitor via parec and keep per-band energies.
// Log-spaced bands 60Hz..10kHz, rolling-peak auto-normalization (quiet audio
// still moves), fast-attack/slow-decay per band so it follows without jitter.
fn audio_capture_loop(st: AudioState) {
    let band_freqs: Vec<f32> = (0..NBANDS)
        .map(|i| 60.0 * (10000.0f32 / 60.0).powf(i as f32 / (NBANDS - 1) as f32))
        .collect();
    loop {
        let sink = std::process::Command::new("pactl")
            .args(["get-default-sink"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string());
        let sink = match sink {
            Some(s) if !s.is_empty() => s,
            _ => { thread::sleep(Duration::from_secs(2)); continue; }
        };
        let monitor = format!("{sink}.monitor");
        let child = std::process::Command::new("parec")
            .args(["--device", &monitor, "--format=s16le",
                   "--rate", &format!("{}", SAMPLE_RATE as u32),
                   "--channels=1", "--latency-msec=40"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(_) => { thread::sleep(Duration::from_secs(2)); continue; }
        };
        let mut out = child.stdout.take().unwrap();
        let mut buf = vec![0u8; CHUNK * 2];
        let mut samples = vec![0f32; CHUNK];
        let mut peak = 1e-6f32;
        let mut prev_rms = 0f32;
        let mut beat_hold = 0u32;
        loop {
            match out.read_exact(&mut buf) {
                Ok(()) => {
                    for i in 0..CHUNK {
                        samples[i] = i16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]) as f32 / 32768.0;
                    }
                    let rms = (samples.iter().map(|v| v * v).sum::<f32>() / CHUNK as f32).sqrt();
                    // per-band energies
                    let mut frame_max = 0f32;
                    let mut mags = [0f32; NBANDS];
                    for (bi, &f) in band_freqs.iter().enumerate() {
                        let m = goertzel(&samples, SAMPLE_RATE, f);
                        mags[bi] = m;
                        if m > frame_max { frame_max = m; }
                    }
                    // rolling-peak adaptive normalization
                    peak = (peak * 0.995).max(frame_max).max(1e-6);
                    for (bi, &m) in mags.iter().enumerate() {
                        let norm = (m / peak).sqrt().min(1.0);
                        st.bands[bi].store(norm.to_bits(), Ordering::Relaxed);
                    }
                    // volume + beat
                    st.volume.store((rms * 4.0).min(1.0).to_bits(), Ordering::Relaxed);
                    let beat = if rms > prev_rms * 1.35 && rms > 0.02 { beat_hold = 3; 1 }
                               else if beat_hold > 0 { beat_hold -= 1; 1 } else { 0 };
                    st.beat.store(beat, Ordering::Relaxed);
                    prev_rms = rms;
                }
                Err(_) => break,
            }
        }
        let _ = child.kill();
        thread::sleep(Duration::from_secs(1));
    }
}

// Frame writer: alt-screen + hidden cursor, home each frame, and crucially NO
// trailing newline on the last row (a trailing newline scrolls the terminal
// and was the source of the visible flicker).
struct Screen {
    cols: usize,
    rows: usize,
    out: String,
    grid: Vec<Vec<char>>,
    tint: Vec<Vec<u8>>, // 0 = default, 1.. = palette index
    show_fps: bool,
    fps_frames: u32,
    fps_since: std::time::Instant,
    fps_value: f32,
}

impl Screen {
    fn new(cols: usize, rows: usize, show_fps: bool) -> Self {
        print!("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H");
        std::io::stdout().flush().ok();
        Screen {
            cols, rows,
            out: String::with_capacity(cols * rows * 8),
            grid: vec![vec![' '; cols]; rows],
            tint: vec![vec![0u8; cols]; rows],
            show_fps,
            fps_frames: 0,
            fps_since: std::time::Instant::now(),
            fps_value: 0.0,
        }
    }
    // Call once per frame; draws an FPS readout in the top-left corner when
    // the user enabled it in preferences.
    fn fps_overlay(&mut self) {
        if !self.show_fps { return; }
        self.fps_frames += 1;
        let el = self.fps_since.elapsed().as_secs_f32();
        if el >= 0.5 {
            self.fps_value = self.fps_frames as f32 / el;
            self.fps_frames = 0;
            self.fps_since = std::time::Instant::now();
        }
        let text = format!("FPS {:.0}", self.fps_value);
        for (i, ch) in text.chars().enumerate() {
            if i < self.cols { self.put(i, 0, ch, 1); }
        }
    }
    fn clear(&mut self) {
        for r in self.grid.iter_mut() { r.fill(' '); }
        for r in self.tint.iter_mut() { r.fill(0); }
    }
    fn put(&mut self, x: usize, y: usize, ch: char, tint: u8) {
        if x < self.cols && y < self.rows {
            self.grid[y][x] = ch;
            self.tint[y][x] = tint;
        }
    }
    fn present(&mut self, palette: &[&str]) {
        self.out.clear();
        self.out.push_str("\x1b[H");
        let mut cur: u8 = 255;
        for r in 0..self.rows {
            for c in 0..self.cols {
                let t = self.tint[r][c];
                if t != cur {
                    if t == 0 { self.out.push_str("\x1b[0m"); }
                    else if (t as usize) <= palette.len() { self.out.push_str(palette[(t - 1) as usize]); }
                    cur = t;
                }
                self.out.push(self.grid[r][c]);
            }
            if r + 1 < self.rows { self.out.push('\n'); }
        }
        if cur != 0 { self.out.push_str("\x1b[0m"); }
        print!("{}", self.out);
        std::io::stdout().flush().ok();
    }
    // Re-read the PTY size; if it changed (window resized / Vte expanded it
    // after spawn), reallocate the grid and report the change so the caller
    // restarts the effect cleanly at the new size.
    fn maybe_resize(&mut self) -> bool {
        if let Some((c, r)) = pty_size() {
            if c != self.cols || r != self.rows {
                self.cols = c;
                self.rows = r;
                self.grid = vec![vec![' '; c]; r];
                self.tint = vec![vec![0u8; c]; r];
                self.out = String::with_capacity(c * r * 8);
                print!("\x1b[2J\x1b[H");
                return true;
            }
        }
        false
    }
}

// Actual size of the PTY we render into. The parent computes cols/rows from
// font metrics, but Vte may round differently — rendering more rows than the
// PTY has caused a visible one-line scroll ("jump") every frame. The child
// queries the real size and clamps to it.
fn pty_size() -> Option<(usize, usize)> {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0
            && ws.ws_col > 0 && ws.ws_row > 0
        {
            Some((ws.ws_col as usize, ws.ws_row as usize))
        } else {
            None
        }
    }
}

// Intro: typewriter title + byline, then the effect takes over.
// The whole intro stack (title + byline + effect tag) is centered on BOTH axes
// as a single unit — previously the title's TOP was parked at rows/2, so the
// block hung in the lower half. `intro_size` scales the block glyphs (1..16);
// on HiDPI the cell grid is dense, so a large intro_size is what makes the boot
// text actually read as a splash — hence the wide range.
fn show_intro(scr: &mut Screen, palette: &[&str], byline: &str, effect: &str, intro_size: i64) {
    let scale = intro_size.clamp(1, 16) as usize;
    let title = "OMARCHY AUDIO BACKGROUND";
    let by = if byline.trim().is_empty() { DEFAULT_BYLINE } else { byline.trim() };

    // Block glyphs: each char becomes a `scale`×`scale` cell block. A cell is
    // `scale`× wider AND `scale`× taller, so this is uniform pixel scaling.
    // Tracking between letters grows with size so glyphs don't fuse when big.
    let ls = if scale > 1 { (scale / 5).max(1) } else { 0 };  // letter spacing (cells)
    let adv = scale + ls;                          // horizontal advance per char
    let block_h = scale;
    let title_w = title.len() * adv - ls;

    // Vertical layout of the whole stack, then center it.
    let gap = 1usize;
    let byline_off = block_h + gap;                // byline row offset from block top
    let tag_off = byline_off + 1 + gap;            // effect-tag row offset
    let total_h = tag_off + 1;
    let ty = scr.rows.saturating_sub(total_h) / 2; // top of the centered block
    let tx = scr.cols.saturating_sub(title_w) / 2;
    let bx = scr.cols.saturating_sub(by.len()) / 2;

    let draw_title = |scr: &mut Screen, upto: usize| {
        for (i, ch) in title.chars().take(upto).enumerate() {
            let cx = tx + i * adv;
            for dx in 0..scale { for dy in 0..block_h {
                scr.put(cx + dx, ty + dy, ch, 1);
            }}
        }
    };

    // Typewriter: title.
    for step in 0..=title.len() {
        scr.clear();
        draw_title(scr, step);
        scr.fps_overlay();
        scr.present(palette);
        thread::sleep(Duration::from_millis(20));
    }
    // Byline types in under the (now complete) title.
    for step in 0..=by.len() {
        scr.clear();
        draw_title(scr, title.len());
        for (i, ch) in by.chars().take(step).enumerate() { scr.put(bx + i, ty + byline_off, ch, 2); }
        scr.fps_overlay();
        scr.present(palette);
        thread::sleep(Duration::from_millis(14));
    }
    // Effect name stamp.
    let tag = format!("— {effect} —");
    let ex = scr.cols.saturating_sub(tag.len()) / 2;
    for (i, ch) in tag.chars().enumerate() { scr.put(ex + i, ty + tag_off, ch, 3); }
    scr.present(palette);
    // Hold the finished splash long enough to actually read it — the intro is the
    // "boot" moment, so don't flash past it (was 900ms; the big HiDPI text needs time).
    thread::sleep(Duration::from_millis(2600));
}

fn run_render(effect: &str, cols: usize, rows: usize, intensity: i64, audio: bool, byline: &str, intro_size: i64, cell_aspect: f32, show_fps: bool) -> Result<()> {
    let intensity = intensity.clamp(0, 10);
    let state = AudioState::start(audio);
    // Start at the REAL PTY size (Vte may round the spawn-time size); the
    // effect loops re-check it each frame and restart cleanly on change.
    let (cols, rows) = match pty_size() {
        Some((c, r)) => (c, r),
        None => (cols, rows),
    };
    let mut scr = Screen::new(cols, rows, show_fps);

    let palette: &[&str] = match effect {
        "rain" => &["\x1b[96m", "\x1b[36m", "\x1b[37m"],
        "wave" => &["\x1b[95m", "\x1b[35m", "\x1b[37m"],
        "bars" => &["\x1b[93m", "\x1b[33m", "\x1b[37m"],
        "fire" => &["\x1b[91m", "\x1b[93m", "\x1b[31m"],
        "life" => &["\x1b[92m", "\x1b[32m", "\x1b[90m"],
        "starfield" => &["\x1b[97m", "\x1b[37m", "\x1b[90m"],
        "donut" => &["\x1b[96m", "\x1b[95m", "\x1b[93m"],
        _ => &["\x1b[97m", "\x1b[92m", "\x1b[32m"], // matrix
    };

    show_intro(&mut scr, palette, byline, effect, intro_size);

    // Each effect returns Ok(()) when it detects a PTY resize; re-dispatch so
    // it restarts with fresh state at the new size (no jump, no stale grid).
    loop {
        let res = match effect {
            "donut" => fx_donut(&mut scr, palette, intensity, &state, cell_aspect),
            "fire" => fx_fire(&mut scr, palette, intensity, &state),
            "starfield" => fx_starfield(&mut scr, palette, intensity, &state),
            "life" => fx_life(&mut scr, palette, intensity, &state),
            "wave" => fx_wave(&mut scr, palette, intensity, &state),
            "bars" => fx_bars(&mut scr, palette, intensity, &state),
            _ => fx_matrix(&mut scr, palette, intensity, &state, effect == "rain"),
        };
        if let Err(e) = res {
            eprintln!("render error: {e:?}");
            break;
        }
        // Ok(()) => resize happened; loop and restart the effect.
    }
    Ok(())
}

// Audio-reactive pacing: more sound => faster flow (lower delay), smoothly.
fn frame_delay(base_ms: i64, intensity: i64, audio: &AudioState) -> Duration {
    let base = (base_ms - intensity * 3).clamp(8, 120) as f32;
    let speed_up = 1.0 + audio.volume() * 2.5;
    Duration::from_millis((base / speed_up).max(6.0) as u64)
}

// --- matrix / rain: column rain where EACH COLUMN follows its frequency band
// (rain equalizer, like the node implementation): band energy raises that
// column's fall speed and brightness. Global spawn density follows volume. ---
fn fx_matrix(scr: &mut Screen, palette: &[&str], intensity: i64, audio: &AudioState, rain: bool) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let chars: Vec<char> = if rain {
        "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿ0123456789".chars().collect()
    } else {
        "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789ABCDEF".chars().collect()
    };
    let speed_base: i32 = 1 + (intensity / 4) as i32;
    let trail_base: usize = 8 + (intensity as usize) * 2;
    let (cols, rows) = (scr.cols, scr.rows);
    let mut head: Vec<i32> = (0..cols).map(|_| rng.gen_range(0..rows as i32)).collect();
    let mut speed: Vec<i32> = (0..cols).map(|_| rng.gen_range(speed_base..speed_base + 2).max(1)).collect();
    let mut trail: Vec<usize> = (0..cols).map(|_| rng.gen_range(trail_base..trail_base + 12)).collect();
    loop {
        if scr.maybe_resize() { return Ok(()); }
        scr.clear();
        let vol = audio.volume();
        let spawn_chance = 0.35 + vol * 0.65; // caudal: more drops with sound
        for c in 0..cols {
            let band = audio.band_at(c, cols); // this column's frequency band
            let h = head[c];
            for t in 0..trail[c] {
                let y = h - t as i32;
                if y >= 0 && y < rows as i32 {
                    // brightness follows the band energy (rain equalizer)
                    let tint = if t == 0 { 1 } else if band > 0.55 && t < 3 { 2 } else { 3 };
                    scr.put(c, y as usize, chars[rng.gen_range(0..chars.len())], tint);
                }
            }
            // fall speed grows with the column's band energy
            head[c] += speed[c] + (band * 2.0) as i32;
            if head[c] - trail[c] as i32 > rows as i32 {
                if rng.gen::<f32>() < spawn_chance {
                    head[c] = -(rng.gen_range(0..30));
                    speed[c] = rng.gen_range(speed_base..speed_base + 2).max(1);
                    trail[c] = rng.gen_range(trail_base..trail_base + 12);
                } else {
                    head[c] = -(rows as i32 + 10); // rest before next drop
                }
            }
        }
        scr.fps_overlay();
        scr.present(palette);
        thread::sleep(frame_delay(40, intensity, audio));
    }
}

// --- wave: layered sine waves scrolling horizontally ---
fn fx_wave(scr: &mut Screen, palette: &[&str], intensity: i64, audio: &AudioState) -> Result<()> {
    let mut t = 0f32;
    loop {
        if scr.maybe_resize() { return Ok(()); }
        scr.clear();
        let lv = audio.volume();
        let (cols, rows) = (scr.cols, scr.rows);
        let cy = rows as f32 / 2.0;
        for layer in 0..3u8 {
            let amp = (rows as f32 * 0.18) * (0.5 + lv) * (1.0 - layer as f32 * 0.25);
            let freq = 0.045 + layer as f32 * 0.02;
            let phase = t * (1.0 + layer as f32 * 0.6);
            for x in 0..cols {
                let y = cy + amp * (x as f32 * freq + phase).sin();
                if y >= 0.0 && (y as usize) < rows {
                    scr.put(x, y as usize, if layer == 0 { '~' } else { '-' }, layer + 1);
                }
            }
        }
        scr.fps_overlay();
        scr.present(palette);
        t += 0.12 + lv * 0.25;
        thread::sleep(frame_delay(45, intensity, audio));
    }
}

// --- bars: equalizer driven by the REAL per-band spectrum (each bar = one
// frequency band's energy), smoothed so it follows without flicker. ---
fn fx_bars(scr: &mut Screen, palette: &[&str], intensity: i64, audio: &AudioState) -> Result<()> {
    let bands = NBANDS;
    let mut heights = vec![0f32; bands];
    loop {
        if scr.maybe_resize() { return Ok(()); }
        scr.clear();
        let (cols, rows) = (scr.cols, scr.rows);
        let maxh = rows as f32 * 0.9;
        let bw = cols / bands.max(1);
        for b in 0..bands {
            let energy = audio.band_at(b * (cols / bands.max(1)), cols); // band b
            let target = (energy * 1.1).min(1.0) * maxh;
            heights[b] += (target - heights[b]) * 0.4; // smooth attack/release
            let h = heights[b] as usize;
            for y in 0..h.min(rows) {
                let tint = if y as f32 > h as f32 * 0.7 { 1 } else if y as f32 > h as f32 * 0.4 { 2 } else { 3 };
                for x in 0..bw.saturating_sub(1) {
                    scr.put(b * bw + x, rows - 1 - y, '#', tint);
                }
            }
        }
        scr.fps_overlay();
        scr.present(palette);
        thread::sleep(frame_delay(50, intensity, audio));
    }
}

// --- donut: classic 3D torus, scaled to the grid. Spin speed follows audio. ---
// Scale follows the original donut.c proportions (30/80 horizontal, 15/22
// vertical), which already bake in the terminal cell aspect. That keeps the
// torus round on any screen; compressing by cell_aspect over-flattened it.
fn fx_donut(scr: &mut Screen, palette: &[&str], intensity: i64, audio: &AudioState, cell_aspect: f32) -> Result<()> {
    let _ = cell_aspect;
    let mut a = 0f32;
    let mut e = 1f32;
    let (cols, rows) = (scr.cols, scr.rows);
    let cx = cols as f32 / 2.0;
    let cy = rows as f32 / 2.0;
    let sx = cols as f32 * (30.0 / 80.0);
    let sy = rows as f32 * (15.0 / 22.0);
    let mut zbuf = vec![0f32; cols * rows];
    loop {
        if scr.maybe_resize() { return Ok(()); }
        scr.clear();
        for b in zbuf.iter_mut() { *b = 0.0; }
        let lv = audio.volume();
        let mut j = 0f32;
        while j < 6.28 {
            let mut i = 0f32;
            while i < 6.28 {
                let (sj, cj) = (j.sin(), j.cos());
                let (si, ci) = (i.sin(), i.cos());
                let (sa, ca) = (a.sin(), a.cos());
                let (se, ce) = (e.sin(), e.cos());
                let h = cj + 2.0;
                let d = 1.0 / (si * h * sa + sj * ca + 5.0);
                let t = si * h * ca - sj * sa;
                let x = (cx + sx * d * (ci * h * ce - t * se)) as i32;
                let y = (cy + sy * d * (ci * h * se + t * ce)) as i32;
                let lum = ((sj * sa - si * ca) * ce - ci * h * se - sj * ca - ci * h * sa) * 8.0;
                if y >= 0 && y < rows as i32 && x >= 0 && x < cols as i32 && d > zbuf[y as usize * cols + x as usize] {
                    zbuf[y as usize * cols + x as usize] = d;
                    let chars = b".,-~:;=!*#$@";
                    let ci2 = lum.max(0.0) as usize;
                    let ch = chars[ci2.min(chars.len() - 1)] as char;
                    let tint = if ci2 > 8 { 1 } else if ci2 > 4 { 2 } else { 3 };
                    scr.put(x as usize, y as usize, ch, tint);
                }
                i += 0.02;
            }
            j += 0.07;
        }
        scr.fps_overlay();
        scr.present(palette);
        let spin = 1.0 + lv * 2.0;
        a += 0.04 * spin;
        e += 0.02 * spin;
        thread::sleep(frame_delay(45, intensity, audio));
    }
}

// --- fire: classic doom fire from the bottom row. Height licks with audio. ---
fn fx_fire(scr: &mut Screen, palette: &[&str], intensity: i64, audio: &AudioState) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let (cols, rows) = (scr.cols, scr.rows);
    let palette_chars: Vec<char> = " .:-=+*#%@".chars().collect();
    let mut heat = vec![vec![0u8; cols]; rows];
    loop {
        if scr.maybe_resize() { return Ok(()); }
        let lv = audio.volume();
        // bottom row: fuel, fanned by the audio level
        let fuel = (28.0 + lv * 8.0 + intensity as f32 * 0.4) as u8;
        for x in 0..cols { heat[rows - 1][x] = fuel.min(36); }
        for y in 0..rows - 1 {
            for x in 0..cols {
                let src_x = (x as i32 + rng.gen_range(-1..=1)).clamp(0, cols as i32 - 1) as usize;
                let decay = rng.gen_range(0..=2);
                heat[y][x] = heat[y + 1][src_x].saturating_sub(decay);
            }
        }
        scr.clear();
        for y in 0..rows {
            for x in 0..cols {
                let h = heat[y][x] as usize;
                if h > 0 {
                    let ci = (h * palette_chars.len() / 37).min(palette_chars.len() - 1);
                    let tint = if h > 24 { 1 } else if h > 12 { 2 } else { 3 };
                    scr.put(x, y, palette_chars[ci], tint);
                }
            }
        }
        scr.fps_overlay();
        scr.present(palette);
        thread::sleep(frame_delay(45, intensity, audio));
    }
}

// --- starfield: stars flying outward from the center. Speed follows audio. ---
fn fx_starfield(scr: &mut Screen, palette: &[&str], intensity: i64, audio: &AudioState) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let (cols, rows) = (scr.cols, scr.rows);
    let cx = cols as f32 / 2.0;
    let cy = rows as f32 / 2.0;
    let n = 160usize;
    let mut stars: Vec<(f32, f32, f32)> = (0..n).map(|_| (
        rng.gen_range(-1.0..1.0), rng.gen_range(-1.0..1.0), rng.gen_range(0.05..1.0),
    )).collect();
    loop {
        if scr.maybe_resize() { return Ok(()); }
        scr.clear();
        let lv = audio.volume();
        let speed = (0.006 + intensity as f32 * 0.0012) * (1.0 + lv * 2.2);
        for s in stars.iter_mut() {
            s.2 -= speed;
            if s.2 <= 0.02 { *s = (rng.gen_range(-1.0..1.0), rng.gen_range(-1.0..1.0), 1.0); }
            let px = cx + s.0 / s.2 * cx * 0.5;
            let py = cy + s.1 / s.2 * cy * 0.5;
            if px >= 0.0 && px < cols as f32 && py >= 0.0 && py < rows as f32 {
                let depth = 1.0 - s.2;
                let (ch, tint) = if depth > 0.75 { ('@', 1) } else if depth > 0.45 { ('*', 2) } else { ('.', 3) };
                scr.put(px as usize, py as usize, ch, tint);
            }
        }
        scr.fps_overlay();
        scr.present(palette);
        thread::sleep(frame_delay(40, intensity, audio));
    }
}

// --- life: Conway's Game of Life, reseeded on stagnation ---
fn fx_life(scr: &mut Screen, palette: &[&str], intensity: i64, audio: &AudioState) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let (cols, rows) = (scr.cols, scr.rows);
    let mut grid = vec![vec![false; cols]; rows];
    let mut next = vec![vec![false; cols]; rows];
    let mut age = vec![vec![0u8; cols]; rows];
    let density = 0.18 + audio.volume() * 0.1;
    for y in 0..rows { for x in 0..cols { grid[y][x] = rng.gen::<f32>() < density; } }
    let mut stagnant = 0u32;
    loop {
        if scr.maybe_resize() { return Ok(()); }
        scr.clear();
        for y in 0..rows {
            for x in 0..cols {
                if grid[y][x] {
                    age[y][x] = age[y][x].saturating_add(1);
                    let tint = if age[y][x] > 30 { 1 } else if age[y][x] > 8 { 2 } else { 3 };
                    scr.put(x, y, if age[y][x] > 8 { 'O' } else { 'o' }, tint);
                } else {
                    age[y][x] = 0;
                }
            }
        }
        scr.fps_overlay();
        scr.present(palette);
        let mut changed = 0u32;
        for y in 0..rows {
            for x in 0..cols {
                let mut n = 0;
                for dy in [-1i32, 0, 1] { for dx in [-1i32, 0, 1] {
                    if dy == 0 && dx == 0 { continue; }
                    let yy = (y as i32 + dy).rem_euclid(rows as i32) as usize;
                    let xx = (x as i32 + dx).rem_euclid(cols as i32) as usize;
                    if grid[yy][xx] { n += 1; }
                }}
                next[y][x] = if grid[y][x] { n == 2 || n == 3 } else { n == 3 };
                if next[y][x] != grid[y][x] { changed += 1; }
            }
        }
        std::mem::swap(&mut grid, &mut next);
        if changed < cols as u32 / 8 {
            stagnant += 1;
            if stagnant > 30 {
                stagnant = 0;
                let density = 0.18 + audio.volume() * 0.1;
                for y in 0..rows { for x in 0..cols {
                    if rng.gen::<f32>() < density * 0.25 { grid[y][x] = true; }
                }}
            }
        } else { stagnant = 0; }
        thread::sleep(frame_delay(70, intensity, audio));
    }
}
