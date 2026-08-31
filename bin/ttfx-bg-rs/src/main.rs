// ttfx-bg-rs: 100% Rust audio-reactive desktop background.
//
// Omarchy paints its desktop background as a gtk4-layer-shell bottom layer.
// We do the same, ONE layer per connected monitor (so the background covers
// every screen). Inside each layer we embed a Vte terminal and draw the
// effect directly (ttfx freezes inside an embedded Vte, but our own renderer
// animates fine — proven by the donut test). This is a real desktop
// background, not a forced window.
//
// Roadmap:
//   - Step 1: vendor ttfx's effect engine as a lib and call it here instead
//     of our hand-rolled renderer (use ttfx's real matrix/rain effects).
//   - Step 2: feed audio into the engine so animations react to music.

use anyhow::Result;
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::thread;
use std::time::Duration;
use vte4::{TerminalExt, TerminalExtManual};

#[derive(Clone, Default)]
struct Config {
    running: bool,
    effect: String,
    intensity: i64,
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".config/omarchy/plugins/io.github.avillagran.omarchy-ttfx-background/state.json")
}

// Best-effort read of the panel's state.json. Missing/invalid => defaults.
// Tolerant of both pretty-printed and single-line (compact) JSON.
// NOTE: use independent `if`s (not `else if`) — a compact line carries all
// three keys at once, and `else if` would only parse the first match.
fn read_config() -> Config {
    let mut cfg = Config { running: true, effect: "matrix".into(), intensity: 5 };
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.split_once("\"running\"") {
                if let Some(v) = rest.1.split(':').nth(1) {
                    cfg.running = v
                        .trim_matches(|c| c == ' ' || c == ',' || c == '"' || c == '}' || c == ']' || c == '{')
                        .starts_with('t');
                }
            }
            if let Some(rest) = line.split_once("\"effect\"") {
                if let Some(v) = rest.1.split(':').nth(1) {
                    // Value is "...","next":... — take the text between the
                    // first pair of quotes, so we get just the effect name.
                    if let Some(e) = v.split('"').nth(1) {
                        cfg.effect = e.to_string();
                    }
                }
            }
            if let Some(rest) = line.split_once("\"intensity\"") {
                if let Some(v) = rest.1.split(':').nth(1) {
                    let s = v.trim_matches(|c| c == ' ' || c == ',' || c == '"' || c == '}' || c == ']');
                    if let Ok(n) = s.parse::<i64>() { cfg.intensity = n; }
                }
            }
        }
    }
    cfg
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--donut") {
        return run_donut();
    }
    if args.iter().any(|a| a == "--matrix") {
        let cols = args.get(2).and_then(|s| s.parse::<usize>().ok()).unwrap_or(200);
        let rows = args.get(3).and_then(|s| s.parse::<usize>().ok()).unwrap_or(100);
        let effect = arg_value(&args, "--effect").unwrap_or_else(|| "matrix".into());
        let intensity = arg_value(&args, "--intensity")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(5);
        return run_matrix(cols, rows, &effect, intensity);
    }

    gtk4::init()?;
    let self_bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "ttfx-bg-rs".into());

    // Track the per-monitor layers so we can tear them down and rebuild them
    // whenever the monitor set, resolution, or panel config changes.
    let windows: Rc<RefCell<Vec<gtk4::ApplicationWindow>>> = Rc::new(RefCell::new(Vec::new()));
    let last_cfg: Rc<RefCell<Config>> = Rc::new(RefCell::new(read_config()));

    rebuild_layers(&windows, &self_bin, &last_cfg.borrow());

    // The Gdk monitor list is a ListModel; it emits `items-changed` when a
    // monitor is added/removed AND when Hyprland re-adds a monitor on a
    // resolution change. Rebuild so every layer matches the new geometry.
    // Read fresh config here: monitor events fire often, and using a cached
    // config would clobber a panel change made in between rebuilds.
    if let Some(display) = gdk::Display::default() {
        let monitors = display.monitors();
        let w = windows.clone();
        let b = self_bin.clone();
        let lc = last_cfg.clone();
        monitors.connect_items_changed(move |_, _pos, _removed, _added| {
            println!("monitors changed — rebuilding background layers");
            let cfg = read_config();
            *lc.borrow_mut() = cfg.clone();
            rebuild_layers(&w, &b, &cfg);
        });
    }

    // Poll the panel's state.json; if running/effect/intensity changed, rebuild.
    {
        let w = windows.clone();
        let b = self_bin.clone();
        let lc = last_cfg.clone();
        glib::timeout_add_local(Duration::from_millis(700), move || {
            let cfg = read_config();
            let changed = {
                let old = lc.borrow();
                old.running != cfg.running || old.effect != cfg.effect || old.intensity != cfg.intensity
            };
            if changed {
                println!("config changed — rebuilding background layers");
                *lc.borrow_mut() = cfg.clone();
                rebuild_layers(&w, &b, &cfg);
            }
            glib::ControlFlow::Continue
        });
    }

    glib::MainLoop::new(None, false).run();
    Ok(())
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == key {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix(&format!("{key}=")) {
            return Some(v.to_string());
        }
    }
    None
}

fn rebuild_layers(windows: &Rc<RefCell<Vec<gtk4::ApplicationWindow>>>, self_bin: &str, cfg: &Config) {
    // Kill any renderer subprocesses from a previous build. The pattern
    // `--matrix` only matches the child renderers, never this parent binary,
    // so pkill won't take us down with them.
    let _ = std::process::Command::new("pkill")
        .args(["-f", "ttfx-bg-rs --matrix"])
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
    println!("found {n} monitor(s), running={}", cfg.running);
    for i in 0..n {
        if let Some(monitor) = monitors.item(i).and_downcast::<gdk::Monitor>() {
            let w = spawn_layer_for_monitor(&monitor, self_bin, cfg);
            windows.borrow_mut().push(w);
        }
    }
}

fn spawn_layer_for_monitor(monitor: &gdk::Monitor, self_bin: &str, cfg: &Config) -> gtk4::ApplicationWindow {
    let window = gtk4::ApplicationWindow::builder()
        .title("ttfx-bg")
        .build();

    window.init_layer_shell();
    window.set_namespace(Some("ttfx-bg"));
    window.set_layer(Layer::Bottom);
    window.set_monitor(Some(monitor)); // anchor this layer to this specific output
    for e in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
        window.set_anchor(e, true);
    }
    window.set_exclusive_zone(-1);

    let term = vte4::Terminal::new();
    // Use a font size in *physical pixels* (not points) so the glyph grid
    // scales with the panel's resolution: on a 4K / scaled display a point
    // size gets inflated by the monitor scale factor and you get a handful of
    // giant characters. GTK applies the monitor scale to the font, so we
    // divide by it to keep a consistent on-screen cell size (~9px) everywhere.
    let scale = monitor.scale_factor().max(1) as f64;
    let cell_px = (9.0 / scale).max(3.0);
    let font = gtk4::gdk::pango::FontDescription::from_string(&format!("monospace {cell_px:.0}px"));
    term.set_font(Some(&font));
    term.set_scrollback_lines(0);
    term.set_hexpand(true);
    term.set_vexpand(true);
    window.set_child(Some(&term));

    // If the panel turned the background off, show an empty (black) layer and
    // skip the renderer. Otherwise spawn the effect renderer sized to fit.
    if cfg.running {
        let geo = monitor.geometry();
        // Measure the real cell size from the font (robust regardless of
        // DPI / scale factor). Vte cells are monospace: width = advance of a
        // glyph, height = font line height.
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
        let effect = cfg.effect.clone();
        let intensity = cfg.intensity;
        let cmd = format!("{self_bin} --matrix {cols} {rows} --effect {effect} --intensity {intensity}");
        // Pre-clear the terminal so Vte doesn't flash its "N by M cells"
        // placeholder while the renderer's PTY is still connecting.
        term.feed(b"\x1b[2J\x1b[H\x1b[40m");
        term.spawn_async(
            vte4::PtyFlags::DEFAULT,
            None,
            &["sh", "-c", &cmd],
            &[],
            gtk4::glib::SpawnFlags::empty(),
            || {},
            2000,
            None::<&gtk4::gio::Cancellable>,
            move |res| match res {
                Ok(_) => println!("renderer spawned ({cols}x{rows}, effect={effect}, intensity={intensity})"),
                Err(e) => eprintln!("spawn error: {e:?}"),
            },
        );
    }

    window.present();
    window
}

// Classic 3D donut, rendered directly to the terminal (animation test / alt bg).
fn run_donut() -> Result<()> {
    fn rotate(t: f32, mut x: f32, mut y: f32) -> (f32, f32) {
        let f = x;
        x -= t * y;
        y += t * f;
        let factor = (3.0 - x * x - y * y) / 2.0;
        (x * factor, y * factor)
    }
    let mut a: f32 = 0.0;
    let mut e: f32 = 1.0;
    let mut c: f32 = 1.0;
    let mut d: f32 = 0.0;
    print!("\x1b[2J");
    std::io::stdout().flush().ok();
    loop {
        let mut z = [0.0f32; 1760];
        let mut b = [' '; 1760];
        let (mut g, mut h) = (0.0f32, 1.0f32);
        for _ in 0..90 {
            let (mut j_cos, mut j_sin) = (0.0f32, 1.0f32);
            for _ in 0..314 {
                let rot_radius = h + 2.0;
                let depth = 1.0 / (j_cos * rot_radius * a + g * e + 5.0);
                let t_val = (j_cos * rot_radius * e) - (g * a);
                let x = (40.0 + (30.0 * depth * (j_sin * rot_radius * d - t_val * c))) as i32;
                let y = (12.0 + (15.0 * depth * (j_sin * rot_radius * c + t_val * d))) as i32;
                let index = x + (80 * y);
                let luminance = (8.0
                    * (((g * a - j_cos * h * e) * d) - j_cos * h * a - g * e - j_sin * h * c))
                    as i32;
                if y > 0 && y < 22 && x > 0 && x < 80 && depth > z[index as usize] {
                    z[index as usize] = depth;
                    let chars = b".,-~:;=!*#$@";
                    let ci = if luminance > 0 { luminance as usize } else { 0 };
                    b[index as usize] = chars[ci.min(chars.len() - 1)] as char;
                }
                let (ns, nc) = rotate(0.02, j_sin, j_cos);
                j_sin = ns;
                j_cos = nc;
            }
            let (nh, ng) = rotate(0.07, h, g);
            h = nh;
            g = ng;
        }
        print!("\x1b[H]");
        for k in 0..1760 {
            print!("{}", if k % 80 != 0 { b[k] } else { '\n' });
        }
        std::io::stdout().flush().ok();
        let (ne, na) = rotate(0.04, e, a);
        e = ne;
        a = na;
        let (nd, nc2) = rotate(0.02, d, c);
        d = nd;
        c = nc2;
        thread::sleep(Duration::from_millis(16));
    }
}

// Matrix rain drawn in Rust (real background, animates inside Vte).
// `effect` selects a color scheme; `intensity` (0-10) scales speed/trail.
// TODO(Step 1): replace with the vendored ttfx engine effects.
fn run_matrix(cols: usize, rows: usize, effect: &str, intensity: i64) -> Result<()> {
    use rand::Rng;
    let intensity = intensity.clamp(0, 10);
    let speed_base: i32 = 1 + (intensity / 4) as i32;
    let trail_base: usize = 8 + (intensity as usize) * 2;
    let head_color: &str = match effect {
        "rain" => "\x1b[96m",
        "wave" => "\x1b[95m",
        "bars" => "\x1b[93m",
        _ => "\x1b[97m", // matrix
    };
    let trail_alt: &str = match effect {
        "rain" => "\x1b[36m",
        "wave" => "\x1b[35m",
        "bars" => "\x1b[33m",
        _ => "\x1b[32m",
    };
    let mut rng = rand::thread_rng();
    let chars: Vec<char> = "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789ABCDEF".chars().collect();
    let mut head: Vec<i32> = (0..cols).map(|_| rng.gen_range(0..rows as i32)).collect();
    let mut speed: Vec<i32> = (0..cols).map(|_| rng.gen_range(speed_base..speed_base + 2).max(1)).collect();
    let mut trail: Vec<usize> = (0..cols).map(|_| rng.gen_range(trail_base..trail_base + 12)).collect();
    print!("\x1b[2J");
    let mut out = String::with_capacity(cols * rows * 6);
    loop {
        out.clear();
        out.push_str("\x1b[H");
        let mut grid = vec![vec![' '; cols]; rows];
        let mut bright = vec![vec![0u8; cols]; rows];
        for c in 0..cols {
            let h = head[c];
            for t in 0..trail[c] {
                let y = h - t as i32;
                if y >= 0 && y < rows as i32 {
                    grid[y as usize][c] = chars[rng.gen_range(0..chars.len())];
                    bright[y as usize][c] = if t == 0 { 3 } else if t < 3 { 2 } else { 1 };
                }
            }
            head[c] += speed[c];
            if head[c] - trail[c] as i32 > rows as i32 {
                head[c] = -(rng.gen_range(0..30));
                speed[c] = rng.gen_range(speed_base..speed_base + 2).max(1);
                trail[c] = rng.gen_range(trail_base..trail_base + 12);
            }
        }
        for r in 0..rows {
            for c in 0..cols {
                match bright[r][c] {
                    3 => out.push_str(head_color),
                    2 => out.push_str("\x1b[92m"),
                    1 => out.push_str(trail_alt),
                    _ => {}
                }
                out.push(grid[r][c]);
                if bright[r][c] != 0 {
                    out.push_str("\x1b[0m");
                }
            }
            out.push('\n');
        }
        print!("{out}");
        std::io::stdout().flush().ok();
        let delay = 40 - (intensity * 3).clamp(0, 35);
        thread::sleep(Duration::from_millis(delay.max(8) as u64));
    }
}
