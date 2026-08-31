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
use std::rc::Rc;
use std::thread;
use std::time::Duration;
use vte4::{TerminalExt, TerminalExtManual};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--donut") {
        return run_donut();
    }
    if args.iter().any(|a| a == "--matrix") {
        let cols = args.get(2).and_then(|s| s.parse::<usize>().ok()).unwrap_or(200);
        let rows = args.get(3).and_then(|s| s.parse::<usize>().ok()).unwrap_or(100);
        return run_matrix(cols, rows);
    }

    gtk4::init()?;
    let self_bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "ttfx-bg-rs".into());

    // Track the per-monitor layers so we can tear them down and rebuild them
    // whenever the monitor set or any resolution changes.
    let windows: Rc<RefCell<Vec<gtk4::ApplicationWindow>>> = Rc::new(RefCell::new(Vec::new()));
    rebuild_layers(&windows, &self_bin);

    // The Gdk monitor list is a ListModel; it emits `items-changed` when a
    // monitor is added/removed AND when Hyprland re-adds a monitor on a
    // resolution change. Rebuild so every layer matches the new geometry.
    if let Some(display) = gdk::Display::default() {
        let monitors = display.monitors();
        let w = windows.clone();
        let b = self_bin.clone();
        monitors.connect_items_changed(move |_, _pos, _removed, _added| {
            println!("monitors changed — rebuilding background layers");
            rebuild_layers(&w, &b);
        });
    }

    glib::MainLoop::new(None, false).run();
    Ok(())
}

fn rebuild_layers(windows: &Rc<RefCell<Vec<gtk4::ApplicationWindow>>>, self_bin: &str) {
    for w in windows.borrow_mut().drain(..) {
        w.close();
    }
    let display = match gdk::Display::default() {
        Some(d) => d,
        None => return,
    };
    let monitors = display.monitors();
    let n = monitors.n_items();
    println!("found {n} monitor(s)");
    for i in 0..n {
        if let Some(monitor) = monitors.item(i).and_downcast::<gdk::Monitor>() {
            let w = spawn_layer_for_monitor(&monitor, self_bin);
            windows.borrow_mut().push(w);
        }
    }
}

fn spawn_layer_for_monitor(monitor: &gdk::Monitor, self_bin: &str) -> gtk4::ApplicationWindow {
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
    let geo = monitor.geometry();
    window.set_default_size(geo.width(), geo.height());

    let term = vte4::Terminal::new();
    let font = gtk4::gdk::pango::FontDescription::from_string("monospace 14");
    term.set_font(Some(&font));
    term.set_scrollback_lines(0);
    term.set_hexpand(true);
    term.set_vexpand(true);
    window.set_child(Some(&term));

    // Compute the terminal grid size from the monitor geometry and the fixed
    // font, and spawn the renderer immediately. We don't wait on a timeout:
    // a monitor change rebuilds the layers and would close this window before
    // a deferred spawn could run, leaving a screen without its renderer.
    let geo = monitor.geometry();
    let cols = ((geo.width() as f64) / 8.4).floor().max(80.0) as usize;
    let rows = ((geo.height() as f64) / 17.0).floor().max(24.0) as usize;
    let cmd = format!("{self_bin} --matrix {cols} {rows}");
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
            Ok(_) => println!("matrix renderer spawned ({cols}x{rows})"),
            Err(e) => eprintln!("spawn error: {e:?}"),
        },
    );

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
fn run_matrix(cols: usize, rows: usize) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let chars: Vec<char> = "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789ABCDEF".chars().collect();
    let mut head: Vec<i32> = (0..cols).map(|_| rng.gen_range(0..rows as i32)).collect();
    let mut speed: Vec<i32> = (0..cols).map(|_| rng.gen_range(1..3)).collect();
    let mut trail: Vec<usize> = (0..cols).map(|_| rng.gen_range(8..24)).collect();
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
                speed[c] = rng.gen_range(1..3);
                trail[c] = rng.gen_range(8..24);
            }
        }
        for r in 0..rows {
            for c in 0..cols {
                match bright[r][c] {
                    3 => out.push_str("\x1b[97m"),
                    2 => out.push_str("\x1b[92m"),
                    1 => out.push_str(if c % 7 == 0 { "\x1b[36m" } else { "\x1b[32m" }),
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
        thread::sleep(Duration::from_millis(33));
    }
}
