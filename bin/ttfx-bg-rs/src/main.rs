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
        return run_render(&effect, cols, rows, intensity, audio, &byline);
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

// Shared audio level 0..1 (as f32 bits) fed by the parec capture thread.
struct AudioLevel(Arc<AtomicU32>);

impl AudioLevel {
    fn start(enabled: bool) -> Self {
        let level = Arc::new(AtomicU32::new(0f32.to_bits()));
        if enabled {
            let l = level.clone();
            thread::spawn(move || audio_capture_loop(l));
        }
        AudioLevel(level)
    }
    fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

// Capture the default sink monitor via parec and keep a smoothed RMS level.
// Fast attack, slow decay — flow follows the music without jittering.
fn audio_capture_loop(level: Arc<AtomicU32>) {
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
            .args(["--device", &monitor, "--format=s16le", "--rate=4000", "--channels=1", "--latency-msec=40"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(_) => { thread::sleep(Duration::from_secs(2)); continue; }
        };
        let mut out = child.stdout.take().unwrap();
        let mut buf = [0u8; 640]; // ~40ms of 4kHz s16 mono
        let mut ema = 0f32;
        loop {
            match out.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let frames = n / 2;
                    if frames == 0 { continue; }
                    let mut sum = 0f32;
                    for i in 0..frames {
                        let s = i16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]) as f32 / 32768.0;
                        sum += s * s;
                    }
                    let rms = (sum / frames as f32).sqrt();
                    let norm = (rms / 0.18).min(1.0);
                    ema = if norm > ema { ema * 0.5 + norm * 0.5 } else { ema * 0.93 + norm * 0.07 };
                    level.store(ema.to_bits(), Ordering::Relaxed);
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
}

impl Screen {
    fn new(cols: usize, rows: usize) -> Self {
        print!("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H");
        std::io::stdout().flush().ok();
        Screen {
            cols, rows,
            out: String::with_capacity(cols * rows * 8),
            grid: vec![vec![' '; cols]; rows],
            tint: vec![vec![0u8; cols]; rows],
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
}

// Intro: typewriter title + byline, then the effect takes over.
fn show_intro(scr: &mut Screen, palette: &[&str], byline: &str, effect: &str) {
    let title = "OMARCHY AUDIO BACKGROUND";
    let by = if byline.trim().is_empty() { DEFAULT_BYLINE } else { byline.trim() };
    let ty = scr.rows / 2;
    let bx = scr.cols.saturating_sub(by.len()) / 2;
    let tx = scr.cols.saturating_sub(title.len()) / 2;
    for step in 0..=title.len() {
        scr.clear();
        for (i, ch) in title.chars().take(step).enumerate() {
            scr.put(tx + i, ty, ch, 1);
        }
        scr.present(palette);
        thread::sleep(Duration::from_millis(28));
    }
    for step in 0..=by.len() {
        scr.clear();
        for (i, ch) in title.chars().enumerate() { scr.put(tx + i, ty, ch, 1); }
        for (i, ch) in by.chars().take(step).enumerate() { scr.put(bx + i, ty + 2, ch, 2); }
        scr.present(palette);
        thread::sleep(Duration::from_millis(18));
    }
    // effect name stamp
    let tag = format!("— {effect} —");
    let ex = scr.cols.saturating_sub(tag.len()) / 2;
    for (i, ch) in tag.chars().enumerate() { scr.put(ex + i, ty + 4, ch, 3); }
    scr.present(palette);
    thread::sleep(Duration::from_millis(900));
}

fn run_render(effect: &str, cols: usize, rows: usize, intensity: i64, audio: bool, byline: &str) -> Result<()> {
    let intensity = intensity.clamp(0, 10);
    let level = AudioLevel::start(audio);
    let mut scr = Screen::new(cols, rows);

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

    show_intro(&mut scr, palette, byline, effect);

    match effect {
        "donut" => fx_donut(&mut scr, palette, intensity, &level),
        "fire" => fx_fire(&mut scr, palette, intensity, &level),
        "starfield" => fx_starfield(&mut scr, palette, intensity, &level),
        "life" => fx_life(&mut scr, palette, intensity, &level),
        "wave" => fx_wave(&mut scr, palette, intensity, &level),
        "bars" => fx_bars(&mut scr, palette, intensity, &level),
        _ => fx_matrix(&mut scr, palette, intensity, &level, effect == "rain"),
    }
}

// Audio-reactive pacing: more sound => faster flow (lower delay), smoothly.
fn frame_delay(base_ms: i64, intensity: i64, level: &AudioLevel) -> Duration {
    let base = (base_ms - intensity * 3).clamp(8, 120) as f32;
    let speed_up = 1.0 + level.get() * 2.5;
    Duration::from_millis((base / speed_up).max(6.0) as u64)
}

// --- matrix / rain: column rain. Density of new drops scales with audio. ---
fn fx_matrix(scr: &mut Screen, palette: &[&str], intensity: i64, level: &AudioLevel, rain: bool) -> Result<()> {
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
        scr.clear();
        let lv = level.get();
        let spawn_chance = 0.35 + lv * 0.65; // caudal: more drops with sound
        for c in 0..cols {
            let h = head[c];
            for t in 0..trail[c] {
                let y = h - t as i32;
                if y >= 0 && y < rows as i32 {
                    scr.put(c, y as usize, chars[rng.gen_range(0..chars.len())],
                        if t == 0 { 1 } else if t < 3 { 2 } else { 3 });
                }
            }
            head[c] += speed[c];
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
        scr.present(palette);
        thread::sleep(frame_delay(40, intensity, level));
    }
}

// --- wave: layered sine waves scrolling horizontally ---
fn fx_wave(scr: &mut Screen, palette: &[&str], intensity: i64, level: &AudioLevel) -> Result<()> {
    let mut t = 0f32;
    loop {
        scr.clear();
        let lv = level.get();
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
        scr.present(palette);
        t += 0.12 + lv * 0.25;
        thread::sleep(frame_delay(45, intensity, level));
    }
}

// --- bars: equalizer bars. Height follows audio level with per-band noise. ---
fn fx_bars(scr: &mut Screen, palette: &[&str], intensity: i64, level: &AudioLevel) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bands = 24usize;
    let mut heights = vec![0f32; bands];
    loop {
        scr.clear();
        let lv = level.get();
        let (cols, rows) = (scr.cols, scr.rows);
        let maxh = rows as f32 * 0.9;
        let bw = cols / bands.max(1);
        for b in 0..bands {
            let band_wave = (0.5 + 0.5 * ((b as f32 * 0.7) + lv * 6.0).sin()).max(0.05);
            let target = (lv * band_wave + rng.gen::<f32>() * 0.08) * maxh;
            heights[b] += (target - heights[b]) * 0.35; // smooth, no flicker
            let h = heights[b] as usize;
            for y in 0..h.min(rows) {
                let tint = if y as f32 > h as f32 * 0.7 { 1 } else if y as f32 > h as f32 * 0.4 { 2 } else { 3 };
                for x in 0..bw.saturating_sub(1) {
                    scr.put(b * bw + x, rows - 1 - y, '#', tint);
                }
            }
        }
        scr.present(palette);
        thread::sleep(frame_delay(50, intensity, level));
    }
}

// --- donut: classic 3D torus, scaled to the grid. Spin speed follows audio. ---
fn fx_donut(scr: &mut Screen, palette: &[&str], intensity: i64, level: &AudioLevel) -> Result<()> {
    let mut a = 0f32;
    let mut e = 1f32;
    let (cols, rows) = (scr.cols, scr.rows);
    let cx = cols as f32 / 2.0;
    let cy = rows as f32 / 2.0;
    let sx = cols as f32 * 0.36;
    let sy = rows as f32 * 0.36;
    let mut zbuf = vec![0f32; cols * rows];
    loop {
        scr.clear();
        for b in zbuf.iter_mut() { *b = 0.0; }
        let lv = level.get();
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
        scr.present(palette);
        let spin = 1.0 + lv * 2.0;
        a += 0.04 * spin;
        e += 0.02 * spin;
        thread::sleep(frame_delay(45, intensity, level));
    }
}

// --- fire: classic doom fire from the bottom row. Height licks with audio. ---
fn fx_fire(scr: &mut Screen, palette: &[&str], intensity: i64, level: &AudioLevel) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let (cols, rows) = (scr.cols, scr.rows);
    let palette_chars: Vec<char> = " .:-=+*#%@".chars().collect();
    let mut heat = vec![vec![0u8; cols]; rows];
    loop {
        let lv = level.get();
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
        scr.present(palette);
        thread::sleep(frame_delay(45, intensity, level));
    }
}

// --- starfield: stars flying outward from the center. Speed follows audio. ---
fn fx_starfield(scr: &mut Screen, palette: &[&str], intensity: i64, level: &AudioLevel) -> Result<()> {
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
        scr.clear();
        let lv = level.get();
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
        scr.present(palette);
        thread::sleep(frame_delay(40, intensity, level));
    }
}

// --- life: Conway's Game of Life, reseeded on stagnation ---
fn fx_life(scr: &mut Screen, palette: &[&str], intensity: i64, level: &AudioLevel) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let (cols, rows) = (scr.cols, scr.rows);
    let mut grid = vec![vec![false; cols]; rows];
    let mut next = vec![vec![false; cols]; rows];
    let mut age = vec![vec![0u8; cols]; rows];
    let density = 0.18 + level.get() * 0.1;
    for y in 0..rows { for x in 0..cols { grid[y][x] = rng.gen::<f32>() < density; } }
    let mut stagnant = 0u32;
    loop {
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
                let density = 0.18 + level.get() * 0.1;
                for y in 0..rows { for x in 0..cols {
                    if rng.gen::<f32>() < density * 0.25 { grid[y][x] = true; }
                }}
            }
        } else { stagnant = 0; }
        thread::sleep(frame_delay(70, intensity, level));
    }
}
