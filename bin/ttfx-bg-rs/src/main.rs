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

// Thread-local palette shared between the watcher (run_render) and the effects
// (fx_matrix, etc.). The watcher writes the current palette here whenever the
// theme changes; effects read it every frame so color changes apply live
// without restarting the plugin.
thread_local! {
    static THEME_PALETTE: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

fn set_theme_palette(palette: Vec<String>) {
    THEME_PALETTE.with(|p| *p.borrow_mut() = palette);
}

fn get_theme_palette() -> Vec<String> {
    THEME_PALETTE.with(|p| p.borrow().clone())
}

// Convert a tint index (0=default, 1..=palette.len()) to an ANSI color string.
fn tint_to_color(tint: u8, palette: &[String]) -> String {
    if tint == 0 || palette.is_empty() {
        String::new()
    } else {
        let idx = ((tint - 1) as usize).min(palette.len() - 1);
        palette[idx].clone()
    }
}
const DEFAULT_EFFECTS: [&str; 8] = ["matrix", "rain", "wave", "bars", "donut", "fire", "starfield", "life"];
const DEFAULT_BYLINE: &str = "By x.com/avillagran";

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
    // Show the boot splash when ROTATING between backgrounds (manual restart always
    // shows it). Time each background stays on screen before rotating.
    boot_between: bool,
    rotate_secs: i64,
    // Canvas text for ttfx effects (they animate text), rendered large as ASCII art. And a
    // resolution scale: bigger cells => fewer cols/rows => less CPU on old machines.
    ttfx_text: String,
    resolution: i64,
    // How strongly ttfx effects react to audio: slider 0..5 (0 = off, 2 = normal, 5 =
    // strong). Scales the frame-pacing speed boost driven by the live volume/beat.
    reactivity: i64,
    // When true, the boot intro types one letter per audio beat (with a timeout
    // fallback if no music is playing), so the letters appear synced to the rhythm.
    intro_beat_sync: bool,
    // When true, use the active Omarchy theme colors for the effect palettes instead
    // of the built-in hardcoded colors. Requires omarchy-theme-current to be installed.
    use_theme_colors: bool,
    // When true, the background is transparent so the user's wallpaper shows through.
    // Off by default (opaque black background).
    transparent_background: bool,
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
            intro_size: 2,
            show_fps: false,
            boot_between: true,
            rotate_secs: 20,
            ttfx_text: "OMARCHY".into(),
            resolution: 1,
            reactivity: 2,
            intro_beat_sync: true,
            use_theme_colors: true,
            transparent_background: false,
        }
    }
}

// state.json lives in ~/.local/state/omarchy/... — NOT in the plugin source dir.
// The shell runs `inotifywait -r` on ~/.config/omarchy/plugins and RELOADS the
// plugin on any file change there, which closed the open panel on every write
// (and respawned everything). Runtime state belongs in the XDG state dir, like
// the other plugins (control-panel-prefs.json etc.).
fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local/state/omarchy/audio-background/state.json")
}

// Legacy location (inside the plugin dir) — read once for migration if the new
// path doesn't exist yet.
fn legacy_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/omarchy/plugins/io.github.avillagran.omarchy-audio-background/state.json")
}

// Best-effort read of the panel's state.json. Missing/invalid => defaults.
// Tolerant of compact or pretty JSON. Independent `if`s on purpose: a compact
// line carries every key at once.
fn read_config() -> Config {
    let mut cfg = Config::default();
    // New XDG-state path first, legacy plugin-dir path as a one-time migration read.
    let text = std::fs::read_to_string(config_path())
        .or_else(|_| std::fs::read_to_string(legacy_config_path()));
    let text = match text {
        Ok(t) => t,
        Err(e) => {
            log_dbg(&format!("read_config: no state file ({e}) -> defaults effect={} effects={:?}", cfg.effect, cfg.effects));
            return cfg;
        }
    };
    if let Some(v) = json_bool(&text, "running") { cfg.running = v; }
    if let Some(v) = json_bool(&text, "audio") { cfg.audio = v; }
    if let Some(v) = json_str(&text, "effect") { cfg.effect = v; }
    if let Some(v) = json_str(&text, "byline") { cfg.byline = v; }
    if let Some(v) = json_num(&text, "intensity") { cfg.intensity = v; }
    if let Some(v) = json_num(&text, "restart") { cfg.restart = v; }
    if let Some(v) = json_num(&text, "intro_size") { cfg.intro_size = v; }
    if let Some(v) = json_bool(&text, "show_fps") { cfg.show_fps = v; }
    if let Some(v) = json_bool(&text, "boot_between") { cfg.boot_between = v; }
    if let Some(v) = json_num(&text, "rotate_secs") { cfg.rotate_secs = v; }
    if let Some(v) = json_str(&text, "ttfx_text") { if !v.trim().is_empty() { cfg.ttfx_text = v; } }
    if let Some(v) = json_num(&text, "resolution") { cfg.resolution = v; }
    if let Some(v) = json_num(&text, "reactivity") { cfg.reactivity = v; }
    if let Some(v) = json_bool(&text, "intro_beat_sync") { cfg.intro_beat_sync = v; }
    if let Some(v) = json_bool(&text, "use_theme_colors") { cfg.use_theme_colors = v; }
    if let Some(v) = json_bool(&text, "transparent_background") { cfg.transparent_background = v; }
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

// Parse a simple TOML colors.toml file from Omarchy themes.
// Only handles `key = "value"` lines (no sections, no tables). Returns a map
// of color name -> hex string (without the leading #).
fn parse_theme_colors(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') { continue; }
        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim().to_string();
            let val = line[eq+1..].trim();
            let val = val.strip_prefix('"').unwrap_or(val);
            let val = val.strip_suffix('"').unwrap_or(val);
            if !key.is_empty() && !val.is_empty() { out.push((key, val.to_string())); }
        }
    }
    out
}

// Convert a hex color string (with or without leading #) to an ANSI 24-bit SGR code.
// Supports both 6-digit (#RRGGBB) and 3-digit (#RGB) forms.
fn hex_to_ansi(hex: &str) -> String {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    let bytes = if hex.len() == 6 {
        [u8::from_str_radix(&hex[..2], 16).unwrap_or(0),
         u8::from_str_radix(&hex[2..4], 16).unwrap_or(0),
         u8::from_str_radix(&hex[4..6], 16).unwrap_or(0)]
    } else if hex.len() == 3 {
        let r = u8::from_str_radix(&hex[..1], 16).unwrap_or(0);
        let g = u8::from_str_radix(&hex[1..2], 16).unwrap_or(0);
        let b = u8::from_str_radix(&hex[2..3], 16).unwrap_or(0);
        [r * 17, g * 17, b * 17]
    } else { [0, 0, 0] };
    format!("\x1b[38;2;{};{};{}m", bytes[0], bytes[1], bytes[2])
}

// Parse a hex color string (#RRGGBB) into (r, g, b) bytes. Returns None if invalid.
fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some((r, g, b))
    } else {
        None
    }
}

// Read the active Omarchy theme's colors.toml and return a map of color name -> ANSI code.
// Returns empty map if the theme file can't be read (graceful degradation to hardcoded colors).
fn read_theme_colors() -> Vec<(String, String)> {
    let theme_name_path = std::path::PathBuf::from(
        std::env::var("HOME").unwrap_or_else(|_| ".".into())
    ).join(".local/state/omarchy/current/theme.name");
    let theme_name = match std::fs::read_to_string(&theme_name_path) {
        Ok(t) => t.trim().to_string(),
        Err(_) => return Vec::new(),
    };
    if theme_name.is_empty() { return Vec::new(); }
    let omarchy_path = std::env::var("OMARCHY_PATH").unwrap_or_else(|_| {
        std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            .join(".local/share/omarchy").to_string_lossy().to_string()
    });
    let colors_path = format!("{}/themes/{}/colors.toml", omarchy_path, theme_name);
    match std::fs::read_to_string(&colors_path) {
        Ok(c) => parse_theme_colors(&c),
        Err(_) => Vec::new(),
    }
}

// Build an effect palette from the active theme colors.
fn theme_palette(theme: &[(String, String)], effect: &str) -> Vec<String> {
    let get = |name: &str, fallback: &str| -> String {
        for (k, v) in theme { if k == name { return hex_to_ansi(v); } }
        fallback.to_string()
    };
    let accent = get("accent", "\x1b[96m");
    let foreground = get("foreground", "\x1b[37m");
    let background = get("background", "\x1b[40m");
    let red = get("red", "\x1b[31m");
    let green = get("green", "\x1b[32m");
    let blue = get("blue", "\x1b[34m");
    let cyan = get("cyan", "\x1b[36m");
    let yellow = get("yellow", "\x1b[33m");
    let magenta = get("magenta", "\x1b[35m");
    let bright_red = get("bright_red", "\x1b[91m");
    let bright_green = get("bright_green", "\x1b[92m");
    let bright_cyan = get("bright_cyan", "\x1b[96m");
    let bright_blue = get("bright_blue", "\x1b[94m");
    let bright_yellow = get("bright_yellow", "\x1b[93m");
    let bright_magenta = get("bright_magenta", "\x1b[95m");

    match effect {
        "rain" => vec![accent.clone(), cyan.clone(), foreground.clone()],
        "matrix" => vec![accent.clone(), green.clone(), blue.clone()],
        "wave" => vec![accent.clone(), magenta.clone(), foreground.clone()],
        "bars" => vec![yellow.clone(), bright_yellow.clone(), foreground.clone()],
        "fire" => vec![red.clone(), bright_red.clone(), yellow.clone()],
        "life" => vec![green.clone(), bright_green.clone(), foreground.clone()],
        "starfield" => vec![foreground.clone(), bright_cyan.clone(), background.clone()],
        "donut" => vec![accent.clone(), cyan.clone(), yellow.clone()],
        _ => vec![foreground.clone(), green.clone(), blue.clone()],
    }
}

// Hardcoded fallback palettes (used when use_theme_colors is false OR theme unavailable).
fn hardcoded_palette(effect: &str) -> Vec<String> {
    match effect {
        "rain" => vec!["\x1b[96m".to_string(), "\x1b[36m".to_string(), "\x1b[37m".to_string()],
        "wave" => vec!["\x1b[95m".to_string(), "\x1b[35m".to_string(), "\x1b[37m".to_string()],
        "bars" => vec!["\x1b[93m".to_string(), "\x1b[33m".to_string(), "\x1b[37m".to_string()],
        "fire" => vec!["\x1b[91m".to_string(), "\x1b[93m".to_string(), "\x1b[31m".to_string()],
        "life" => vec!["\x1b[92m".to_string(), "\x1b[32m".to_string(), "\x1b[90m".to_string()],
        "starfield" => vec!["\x1b[97m".to_string(), "\x1b[37m".to_string(), "\x1b[90m".to_string()],
        "donut" => vec!["\x1b[96m".to_string(), "\x1b[95m".to_string(), "\x1b[93m".to_string()],
        _ => vec!["\x1b[97m".to_string(), "\x1b[92m".to_string(), "\x1b[32m".to_string()],
    }
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
        let ttfx_text = arg_value(&args, "--ttfx-text").unwrap_or_else(|| "OMARCHY".into());
        let reactivity = arg_value(&args, "--reactivity").and_then(|s| s.parse::<i64>().ok()).unwrap_or(2);
        let intro_size = arg_value(&args, "--intro-size").and_then(|s| s.parse::<i64>().ok()).unwrap_or(2);
        let cell_aspect = arg_value(&args, "--cell-aspect").and_then(|s| s.parse::<f32>().ok()).unwrap_or(2.0);
        let show_fps = arg_value(&args, "--show-fps").map(|s| s == "1").unwrap_or(false);
        let show_intro = !args.iter().any(|a| a == "--no-intro");
        let cfg = read_config();
        let intro_beat_sync = arg_value(&args, "--intro-beat-sync").map(|s| s == "1").unwrap_or(cfg.intro_beat_sync);
        return run_render(&cfg, &effect, cols, rows, intensity, audio, &byline, intro_size, cell_aspect, show_fps, show_intro, &ttfx_text, reactivity, intro_beat_sync);
    }

    // Drive a vendored ttfx effect directly (library bridge proof).
    if args.iter().any(|a| a == "--ttfx") {
        let effect = arg_value(&args, "--effect").unwrap_or_else(|| "matrix".into());
        let cols = arg_value(&args, "--cols").and_then(|s| s.parse::<usize>().ok()).unwrap_or(80);
        let rows = arg_value(&args, "--rows").and_then(|s| s.parse::<usize>().ok()).unwrap_or(24);
        let ttfx_text = arg_value(&args, "--ttfx-text").unwrap_or_else(|| "OMARCHY".into());
        let reactivity = arg_value(&args, "--reactivity").and_then(|s| s.parse::<i64>().ok()).unwrap_or(2);
        let state = AudioState::start(false); // standalone --ttfx: no audio capture
        return run_ttfx(&effect, cols, rows, &ttfx_text, &state, false, reactivity, false);
    }

    gtk4::init()?;
    let self_bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "ttfx-bg-rs".into());

    let windows: Rc<RefCell<Vec<gtk4::ApplicationWindow>>> = Rc::new(RefCell::new(Vec::new()));
    let cfg0 = read_config();
    let active_effect: Rc<RefCell<String>> = Rc::new(RefCell::new(
        // Only honor the saved effect if it's still enabled in the rotation set.
        // Otherwise the user disabled it — start with the first enabled one.
        if is_valid_effect(&cfg0.effect) && cfg0.effects.contains(&cfg0.effect) { cfg0.effect.clone() }
        else { cfg0.effects.first().cloned().unwrap_or_else(|| "matrix".into()) }
    ));
    let last_cfg: Rc<RefCell<Config>> = Rc::new(RefCell::new(cfg0));

    rebuild_layers(&windows, &self_bin, &last_cfg.borrow(), &active_effect.borrow(), true);

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
                rebuild_layers(&w, &b, &cfg, &ae.borrow(), true);
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
                log_dbg(&format!("poll changed: old effect={} effects={:?} -> new effect={} effects={:?} ae={} running={}", old.effect, old.effects, cfg.effect, cfg.effects, ae.borrow(), cfg.running));
                // If the active-effect selection changed, honor it; otherwise
                // keep rotating from where we are. Only honor if still enabled.
                if cfg.effect != old.effect && is_valid_effect(&cfg.effect) && cfg.effects.contains(&cfg.effect) {
                    *ae.borrow_mut() = cfg.effect.clone();
                } else if !cfg.effects.contains(&ae.borrow().clone()) {
                    // Active effect was disabled via toggle — jump to first enabled.
                    if let Some(first) = cfg.effects.first() {
                        *ae.borrow_mut() = first.clone();
                    }
                }
                *lc.borrow_mut() = cfg.clone();
                // boot_between / rotate_secs are read live by the rotation timer, so a
                // change to ONLY those shouldn't respawn the background (and replay the
                // intro). Rebuild only when a structural/visual field actually changed.
                let visual = cfg.running != old.running
                    || cfg.effect != old.effect
                    || cfg.effects != old.effects
                    || cfg.intensity != old.intensity
                    || cfg.audio != old.audio
                    || cfg.byline != old.byline
                    || cfg.ttfx_text != old.ttfx_text
                    || cfg.restart != old.restart
                    || cfg.intro_size != old.intro_size
                    || cfg.show_fps != old.show_fps
                    || cfg.resolution != old.resolution;
                // Rebuild only when a structural/visual field actually changed. Changing
                // a slider (intensity, reactivity — the latter is applied live by the
                // renderer) or a rotation toggle should NOT replay the boot intro (that's
                // the jarring "restart" the user sees). Intro only on a real effect switch,
                // explicit restart, or an intro-affecting field (byline/intro_size).
                let intro = cfg.effect != old.effect
                    || cfg.restart != old.restart
                    || cfg.intro_size != old.intro_size
                    || cfg.ttfx_text != old.ttfx_text
                    || cfg.byline != old.byline;
                if visual {
                    rebuild_layers(&w, &b, &cfg, &ae.borrow(), intro);
                }
            }
            glib::ControlFlow::Continue
        });
    }

    // Rotate through the enabled effects when more than one is enabled. Fires every
    // second and counts up to cfg.rotate_secs so the interval is live-configurable
    // from the panel (no respawn needed to change it). Rotating respects
    // boot_between for whether the splash replays.
    {
        let w = windows.clone();
        let b = self_bin.clone();
        let lc = last_cfg.clone();
        let ae = active_effect.clone();
        let elapsed = std::rc::Rc::new(std::cell::Cell::new(0u64));
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let cfg = lc.borrow().clone();
            if cfg.running && cfg.effects.len() > 1 {
                // If the currently displayed effect was just disabled, switch immediately
                // instead of waiting for the full interval (otherwise a disabled
                // effect stays visible for up to rotate_secs).
                let cur = ae.borrow().clone();
                if !cfg.effects.contains(&cur) {
                    let next = cfg.effects[0].clone();
                    *ae.borrow_mut() = next.clone();
                    elapsed.set(0);
                    rebuild_layers(&w, &b, &cfg, &next, false);
                    return glib::ControlFlow::Continue;
                }
                let e = elapsed.get() + 1;
                if e >= cfg.rotate_secs.clamp(3, 3600) as u64 {
                    elapsed.set(0);
                    let next = match cfg.effects.iter().position(|x| *x == cur) {
                        Some(i) => cfg.effects[(i + 1) % cfg.effects.len()].clone(),
                        None => cfg.effects[0].clone(),
                    };
                    if next != cur {
                        *ae.borrow_mut() = next.clone();
                        rebuild_layers(&w, &b, &cfg, &next, cfg.boot_between);
                    }
                } else {
                    elapsed.set(e);
                }
            } else {
                elapsed.set(0);
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

fn log_dbg(msg: &str) {
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() % 86400).unwrap_or(0);
    let h = ts / 3600; let m = (ts % 3600) / 60; let s = ts % 60;
    let line = format!("[{h:02}:{m:02}:{s:02}] {msg}\n");
    let _ = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/ttfx-bg-debug.log").and_then(|mut f| f.write_all(line.as_bytes()));
    eprintln!("{msg}");
}

fn rebuild_layers(
    windows: &Rc<RefCell<Vec<gtk4::ApplicationWindow>>>,
    self_bin: &str,
    cfg: &Config,
    effect: &str,
    show_intro: bool,
) {
    log_dbg(&format!("rebuild_layers: effect={effect} show_intro={show_intro} running={} audio={} intensity={} reactivity={} resolution={} rotate_secs={} effect_field={} effects={:?} ttfx_text={}", cfg.running, cfg.audio, cfg.intensity, cfg.reactivity, cfg.resolution, cfg.rotate_secs, cfg.effect, cfg.effects, cfg.ttfx_text));
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
            let w = spawn_layer_for_monitor(&monitor, self_bin, cfg, effect, show_intro);
            windows.borrow_mut().push(w);
        }
    }
}

fn spawn_layer_for_monitor(
    monitor: &gdk::Monitor,
    self_bin: &str,
    cfg: &Config,
    effect: &str,
    show_intro: bool,
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
    // `resolution` scales the cell size: 1 = full-res grid (most CPU), higher =
    // bigger cells => fewer cols/rows => much less CPU (for old machines).
    let cell_px = (9.0 / scale * cfg.resolution.clamp(1, 8) as f64).max(3.0);
    let font = gtk4::gdk::pango::FontDescription::from_string(&format!("monospace {cell_px:.0}px"));
    term.set_font(Some(&font));
    term.set_scrollback_lines(0);
    term.set_hexpand(true);
    term.set_vexpand(true);
    // When transparent_background is enabled, make Vte's background transparent
    // so the user's wallpaper shows through the empty cells.
    if cfg.transparent_background {
        term.set_color_background(&gtk4::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
    }
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
        let mut argv: Vec<String> = vec![
            self_bin.to_string(),
            "--render".into(),
            "--effect".into(), effect.to_string(),
            "--cols".into(), cols.to_string(),
            "--rows".into(), rows.to_string(),
            "--intensity".into(), cfg.intensity.to_string(),
            "--audio".into(), audio.into(),
            "--byline".into(), byline,
            "--ttfx-text".into(), cfg.ttfx_text.clone(),
            "--reactivity".into(), cfg.reactivity.to_string(),
            "--intro-size".into(), cfg.intro_size.to_string(),
            "--cell-aspect".into(), format!("{cell_aspect:.3}"),
            "--show-fps".into(), if cfg.show_fps { "1".into() } else { "0".into() },
        ];
        // Rotation with "boot between backgrounds" off skips the splash.
        if !show_intro { argv.push("--no-intro".into()); }
        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
        // Pass intro_beat_sync as CLI arg so the renderer uses it
        let intro_beat_sync_arg = if cfg.intro_beat_sync { "1" } else { "0" };
        argv.push("--intro-beat-sync".into());
        argv.push(intro_beat_sync_arg.into());
        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
        // Pre-clear so Vte doesn't flash its "N by M cells" placeholder while
        // the renderer's PTY is still connecting. Use transparent background
        // when the option is enabled.
        if cfg.transparent_background {
            term.feed(b"\x1b[2J\x1b[H");
        } else {
            term.feed(b"\x1b[2J\x1b[H\x1b[40m");
        }
        let effect_for_log = effect.to_string();
        log_dbg(&format!("spawn_async: effect={effect_for_log} {cols}x{rows} intensity={} reactivity={} audio={} ttfx_text={} show_intro={}", cfg.intensity, cfg.reactivity, cfg.audio, cfg.ttfx_text, show_intro));
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
                Ok(_) => { println!("renderer spawned ({cols}x{rows}, effect={effect_for_log})"); log_dbg(&format!("spawn ok: {effect_for_log} {cols}x{rows}")); },
                Err(e) => { eprintln!("spawn error: {e:?}"); log_dbg(&format!("spawn error {effect_for_log}: {e:?}")); },
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
    color: Vec<Vec<String>>, // per-cell ANSI color string (empty = default)
    dirty: Vec<Vec<bool>>,   // cells painted this frame (need clearing next frame)
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
            color: vec![vec![String::new(); cols]; rows],
            dirty: vec![vec![false; cols]; rows],
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
            if i < self.cols { self.put(i, 0, ch, "\x1b[37m".to_string()); }
        }
    }
    fn clear(&mut self) {
        for r in self.grid.iter_mut() { r.fill(' '); }
        for r in self.color.iter_mut() { r.iter_mut().for_each(|c| c.clear()); }
        for r in self.dirty.iter_mut() { r.fill(false); }
    }
    // Clear only cells that were painted the previous frame (dirty tracking).
    // Cells NOT painted keep their color — this enables crossfade between themes.
    fn clear_dirty(&mut self) {
        for r in 0..self.rows {
            for c in 0..self.cols {
                if self.dirty[r][c] {
                    self.grid[r][c] = ' ';
                    self.color[r][c].clear();
                    self.dirty[r][c] = false;
                }
            }
        }
    }
    fn put(&mut self, x: usize, y: usize, ch: char, color: String) {
        if x < self.cols && y < self.rows {
            self.grid[y][x] = ch;
            self.color[y][x] = color;
            self.dirty[y][x] = true;
        }
    }
    // Erase a single cell (restore to blank, no color) without dirty tracking.
    fn erase(&mut self, x: usize, y: usize) {
        if x < self.cols && y < self.rows {
            self.grid[y][x] = ' ';
            self.color[y][x].clear();
            self.dirty[y][x] = false;
        }
    }
    // Return the ANSI color currently stored at a cell (empty = default).
    fn get_color(&self, x: usize, y: usize) -> String {
        if x < self.cols && y < self.rows {
            self.color[y][x].clone()
        } else {
            String::new()
        }
    }
    fn present(&mut self) {
        self.out.clear();
        self.out.push_str("\x1b[H");
        let mut cur: &str = "";
        for r in 0..self.rows {
            for c in 0..self.cols {
                let col = &self.color[r][c];
                if col.is_empty() {
                    if cur != "" { self.out.push_str("\x1b[0m"); cur = ""; }
                } else if col != cur {
                    self.out.push_str(col);
                    cur = col;
                }
                self.out.push(self.grid[r][c]);
            }
            if r + 1 < self.rows { self.out.push('\n'); }
        }
        if cur != "" { self.out.push_str("\x1b[0m"); }
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
                self.color = vec![vec![String::new(); c]; r];
                self.dirty = vec![vec![false; c]; r];
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

// Wait for Vte to size the PTY to the real widget dimensions (it spawns at 80x24
// and resizes a beat later), returning the settled (cols, rows). Reading too early
// lays a one-shot layout (the intro, a ttfx canvas) out on an 80x24 grid.
fn settle_pty_size(fallback_cols: usize, fallback_rows: usize) -> (usize, usize) {
    let fallback = (fallback_cols, fallback_rows);
    let mut prev = pty_size().unwrap_or(fallback);
    let mut settled = if prev != (80, 24) { prev } else { fallback };
    for _ in 0..150 {  // up to ~1.5s
        let cur = pty_size().unwrap_or(fallback);
        if cur != (80, 24) && cur == prev { settled = cur; break; }
        prev = cur;
        thread::sleep(Duration::from_millis(10));
    }
    settled
}

// 5-row ASCII bitmap font (FIGlet-style, pure ASCII '#'/space). Each glyph is 5
// rows tall; `intro_size` scales each font pixel into an N×N cell block, so the
// title renders as real ASCII-art letters — not a per-char solid block.
fn glyph_rows(c: char) -> [&'static str; 5] {
    match c {
        'A' => [" ### ", "#   #", "#   #", "#####", "#   #"],
        'B' => ["#### ", "#   #", "#### ", "#   #", "#### "],
        'C' => [" ####", "#    ", "#    ", "#    ", " ####"],
        'D' => ["#### ", "#   #", "#   #", "#   #", "#### "],
        'E' => ["#####", "#    ", "#### ", "#    ", "#####"],
        'F' => ["#####", "#    ", "#### ", "#    ", "#    "],
        'G' => [" ####", "#    ", "#  ##", "#   #", " ####"],
        'H' => ["#   #", "#   #", "#####", "#   #", "#   #"],
        'I' => ["#####", "  #  ", "  #  ", "  #  ", "#####"],
        'J' => ["#####", "   # ", "   # ", "#  # ", " ##  "],
        'K' => ["#   #", "#  # ", "###  ", "#  # ", "#   #"],
        'L' => ["#    ", "#    ", "#    ", "#    ", "#####"],
        'M' => ["#   #", "## ##", "# # #", "#   #", "#   #"],
        'N' => ["#   #", "##  #", "# # #", "#  ##", "#   #"],
        'O' => [" ### ", "#   #", "#   #", "#   #", " ### "],
        'P' => ["#### ", "#   #", "#### ", "#    ", "#    "],
        'Q' => [" ### ", "#   #", "# # #", "#  # ", " ## #"],
        'R' => ["#### ", "#   #", "#### ", "#  # ", "#   #"],
        'S' => [" ####", "#    ", " ### ", "    #", "#### "],
        'T' => ["#####", "  #  ", "  #  ", "  #  ", "  #  "],
        'U' => ["#   #", "#   #", "#   #", "#   #", " ### "],
        'V' => ["#   #", "#   #", "#   #", " # # ", "  #  "],
        'W' => ["#   #", "#   #", "# # #", "## ##", "#   #"],
        'X' => ["#   #", " # # ", "  #  ", " # # ", "#   #"],
        'Y' => ["#   #", " # # ", "  #  ", "  #  ", "  #  "],
        'Z' => ["#####", "   # ", "  #  ", " #   ", "#####"],
        '0' => [" ### ", "#  ##", "# # #", "##  #", " ### "],
        '1' => ["  #  ", " ##  ", "  #  ", "  #  ", "#####"],
        '2' => [" ### ", "#   #", "  ## ", " #   ", "#####"],
        '3' => ["#### ", "    #", " ### ", "    #", "#### "],
        '4' => ["#  # ", "#  # ", "#####", "   # ", "   # "],
        '5' => ["#####", "#    ", "#### ", "    #", "#### "],
        '6' => [" ### ", "#    ", "#### ", "#   #", " ### "],
        '7' => ["#####", "   # ", "  #  ", " #   ", "#    "],
        '8' => [" ### ", "#   #", " ### ", "#   #", " ### "],
        '9' => [" ### ", "#   #", " ####", "    #", " ### "],
        '.' => ["     ", "     ", "     ", "     ", "  #  "],
        ',' => ["     ", "     ", "     ", "  #  ", " #   "],
        '/' => ["    #", "   # ", "  #  ", " #   ", "#    "],
        '@' => [" ### ", "# ###", "# # #", "# ## ", " ### "],
        '-' => ["     ", "     ", " ### ", "     ", "     "],
        ':' => ["     ", "  #  ", "     ", "  #  ", "     "],
        '!' => ["  #  ", "  #  ", "  #  ", "     ", "  #  "],
        '?' => [" ### ", "#   #", "  ## ", "     ", "  #  "],
        '+' => ["     ", "  #  ", " ### ", "  #  ", "     "],
        '_' => ["     ", "     ", "     ", "     ", "#####"],
        '(' => ["   # ", "  #  ", "  #  ", "  #  ", "   # "],
        ')' => ["#    ", " #   ", " #   ", " #   ", "#    "],
        '\'' => ["  #  ", "  #  ", "     ", "     ", "     "],
        _ => ["     ", "     ", "     ", "     ", "     "],
    }
}

// Uppercase + normalize dashes so any byline / effect name maps onto the font.
fn prep(s: &str) -> String {
    s.chars().map(|c| match c { '—' | '–' => '-', other => other.to_ascii_uppercase() }).collect()
}

// Render the first `upto` chars of `text` as scaled ASCII-art rows (each font
// pixel becomes an N×N cell block). All glyphs are 5 rows × 5 cols.
fn art_prefix(text: &str, upto: usize, scale: usize, gap: usize) -> Vec<String> {
    let mut rows = vec![String::new(); 5 * scale];
    for ch in text.chars().take(upto) {
        let g = glyph_rows(ch);
        for r in 0..5 {
            let mut scaled = String::new();
            for c in g[r].chars() { for _ in 0..scale { scaled.push(c); } }
            for sy in 0..scale {
                rows[r * scale + sy].push_str(&scaled);
                for _ in 0..gap { rows[r * scale + sy].push(' '); }
            }
        }
    }
    rows
}

// Nominal rendered width of `text` as ASCII art (glyphs are 5 cols + `gap`).
fn art_width(text: &str, scale: usize, gap: usize) -> usize {
    let n = text.chars().count();
    if n == 0 { 0 } else { n * (5 * scale + gap) - gap }
}

// Intro: the title, byline and effect tag ALL render as ASCII art (scaled
// ×1..×3), centered on BOTH axes as a single stack. The title types in letter by
// letter, then the byline, then the effect tag, then it holds.
// Audio-reactive: each letter step pulses to the beat (volume/beat => faster typing).
fn show_intro(scr: &mut Screen, palette: &[String], byline: &str, effect: &str, intro_size: i64, audio: &AudioState, intro_beat_sync: bool) {
    let scale = intro_size.clamp(1, 3) as usize;
    let gap = scale.max(1); // spaces between ASCII-art letters
    let title = "OMARCHY AUDIO BACKGROUND".to_string();
    let by = prep(if byline.trim().is_empty() { DEFAULT_BYLINE } else { byline.trim() });
    let tag = format!("- {} -", prep(effect));

    let bh = 5 * scale;   // every block is 5 glyph rows × scale
    let vgap = 1usize;    // blank row between blocks

    let title_w = art_width(&title, scale, gap);
    let by_w = art_width(&by, scale, gap);
    let tag_w = art_width(&tag, scale, gap);

    // Center the whole 3-block stack vertically, each block centered horizontally.
    let total_h = 3 * bh + 2 * vgap;
    let ty = scr.rows.saturating_sub(total_h) / 2;
    let by_y = ty + bh + vgap;
    let tag_y = by_y + bh + vgap;
    let tx_title = scr.cols.saturating_sub(title_w) / 2;
    let tx_by = scr.cols.saturating_sub(by_w) / 2;
    let tx_tag = scr.cols.saturating_sub(tag_w) / 2;

    // Draw the first `upto` letters of a block at its (centered) left edge, so the
    // typewriter fills left-to-right and ends centered.
    let draw = |scr: &mut Screen, text: &str, upto: usize, x: usize, y: usize, color: u8| {
        for (r, line) in art_prefix(text, upto, scale, gap).iter().enumerate() {
            for (i, ch) in line.chars().enumerate() {
                if ch != ' ' { scr.put(x + i, y + r, ch, tint_to_color(color, palette)); }
            }
        }
    };

    let title_len = title.chars().count();
    let by_len = by.chars().count();
    let tag_len = tag.chars().count();

    // Helper: wait for the next audio beat (with timeout fallback if no music).
    let wait_for_beat = |audio: &AudioState, timeout_ms: u64| -> bool {
        let start = std::time::Instant::now();
        loop {
            if audio.beat() { return true; }
            if start.elapsed() >= Duration::from_millis(timeout_ms) { return false; }
            thread::sleep(Duration::from_millis(20));
        }
    };

    if intro_beat_sync && audio.volume() > 0.02 {
        // Efectos internos de ttfx que NO usan colores de tema; usar palett...[truncated]
        for step in 0..=title_len {
            scr.clear_dirty();
            draw(scr, &title, step, tx_title, ty, 1);
            scr.fps_overlay();
            scr.present();
            let beat_sync = wait_for_beat(audio, 1500);
            // If we got a beat, great; otherwise fall back to volume-paced delay.
            if !beat_sync {
                let vol = audio.volume().clamp(0.0, 1.0);
                let speed = 1.0 + vol * 2.5;
                thread::sleep(Duration::from_millis((70.0 / speed).max(12.0) as u64));
            }
        }
        // Phase 2: type the byline under the complete title.
        for step in 0..=by_len {
            scr.clear_dirty();
            draw(scr, &title, title_len, tx_title, ty, 1);
            draw(scr, &by, step, tx_by, by_y, 2);
            scr.fps_overlay();
            scr.present();
            let beat_sync = wait_for_beat(audio, 1500);
            if !beat_sync {
                let vol = audio.volume().clamp(0.0, 1.0);
                let speed = 1.0 + vol * 2.5;
                thread::sleep(Duration::from_millis((45.0 / speed).max(10.0) as u64));
            }
        }
    } else {
        // Phase 1: type the title, one ASCII-art letter at a time.
        for step in 0..=title_len {
            scr.clear_dirty();
            draw(scr, &title, step, tx_title, ty, 1);
            scr.fps_overlay();
            scr.present();
            let vol = audio.volume().clamp(0.0, 1.0);
            let speed = 1.0 + vol * 2.5 + if audio.beat() { 1.0 } else { 0.0 };
            thread::sleep(Duration::from_millis((70.0 / speed).max(12.0) as u64));
        }
        // Phase 2: type the byline under the complete title.
        for step in 0..=by_len {
            scr.clear_dirty();
            draw(scr, &title, title_len, tx_title, ty, 1);
            draw(scr, &by, step, tx_by, by_y, 2);
            scr.fps_overlay();
            scr.present();
            let vol = audio.volume().clamp(0.0, 1.0);
            let speed = 1.0 + vol * 2.5 + if audio.beat() { 1.0 } else { 0.0 };
            thread::sleep(Duration::from_millis((45.0 / speed).max(10.0) as u64));
        }
    }

    // Phase 3: stamp the effect tag, then hold the finished splash.
    scr.clear_dirty();
    draw(scr, &title, title_len, tx_title, ty, 1);
    draw(scr, &by, by_len, tx_by, by_y, 2);
    draw(scr, &tag, tag_len, tx_tag, tag_y, 3);
    scr.fps_overlay();
    scr.present();
    // Hold pulses with audio: beat shortens hold, quiet holds full 2600ms.
    let hold_base = 2600.0;
    // If audio active, let the hold be slightly shorter on loud sections so boot feels synced.
    let vol = audio.volume().clamp(0.0, 1.0);
    let hold_speed = 1.0 + vol * 0.8 + if audio.beat() { 0.5 } else { 0.0 };
    thread::sleep(Duration::from_millis((hold_base / hold_speed).max(900.0) as u64));
}

// ttfx effects we route to the vendored engine (everything in the catalog EXCEPT
// matrix/rain, which we keep hand-rolled because ours are audio-reactive).
const TTFX_EFFECTS: [&str; 35] = [
    "beams", "binarypath", "blackhole", "bouncyballs", "bubbles", "burn", "colorshift",
    "crumble", "decrypt", "errorcorrect", "expand", "fireworks", "highlight", "laseretch",
    "middleout", "orbittingvolley", "overflow", "pour", "print", "randomsequence", "rings",
    "scattered", "slice", "slide", "smoke", "spotlights", "spray", "swarm", "sweep",
    "synthgrid", "thunderstorm", "unstable", "vhstape", "waves", "wipe",
];

fn is_ttfx_effect(name: &str) -> bool { TTFX_EFFECTS.contains(&name) }

// Any effect the renderer can actually run: our hand-rolled set or the ttfx catalog.
fn is_valid_effect(name: &str) -> bool { DEFAULT_EFFECTS.contains(&name) || is_ttfx_effect(name) }

// Drive a vendored ttfx effect on our Vte PTY, looping so it runs as a continuous
// background (ttfx effects settle when done; rebuild and replay). Handles PTY resize
// by rebuilding at the new size. Runs after our ASCII intro (same stdout).
// Canvas input for ttfx effects: the configured text rendered as centered ASCII art —
// big enough that the effect animates a real readable word (ttfx effects animate
// text, so they need real content, not sparse noise).
fn ttfx_canvas_input(text: &str, cols: usize, rows: usize) -> String {
    let t = prep(text);
    let n = t.chars().count().max(1);
    // Scale to fill ~70% of the width, capped by ~60% of the height.
    let ws = ((cols as f64 * 0.7) / (6.0 * n as f64)) as usize;
    let hs = ((rows as f64 * 0.6) / 5.0) as usize;
    let scale = ws.min(hs).clamp(1, 40);
    let gap = scale.max(1);
    let art = art_prefix(&t, n, scale, gap);
    let art_h = 5 * scale;
    let art_w = art_width(&t, scale, gap);
    let top = rows.saturating_sub(art_h) / 2;
    let left = cols.saturating_sub(art_w) / 2;
    let mut canvas = vec![vec![' '; cols]; rows];
    for (r, line) in art.iter().enumerate() {
        for (i, ch) in line.chars().enumerate() {
            if ch != ' ' && top + r < rows && left + i < cols {
                canvas[top + r][left + i] = ch;
            }
        }
    }
    canvas.iter().map(|row| row.iter().collect::<String>()).collect::<Vec<_>>().join("\n")
}

// Drive a vendored ttfx effect on our Vte PTY, audio-reactively. The effect runs on
// a VIRTUAL clock and we pace the frames by the live audio level — loud music
// advances the effect faster, quiet slows it — so the whole ttfx catalog reacts to
// the music like the hand-rolled effects do. Loops so it runs as a continuous
// background; rebuilds on PTY resize. Runs after our ASCII intro (same stdout).
fn run_ttfx(effect_name: &str, cols: usize, rows: usize, ttfx_text: &str, audio: &AudioState, audio_enabled: bool, reactivity: i64, use_theme_colors: bool) -> Result<()> {
    log_dbg(&format!("run_ttfx enter: effect={effect_name} {cols}x{rows} ttfx_text={ttfx_text} reactivity={reactivity} audio_enabled={audio_enabled}"));
    use clap::Parser;
    use std::io::Write;
    use ttfx::engine::ctx::{Clock, EngineCtx};
    use ttfx::engine::terminal::TerminalConfig;
    use ttfx::utils::rng::Rng;

    let (mut cols, mut rows) = settle_pty_size(cols, rows);
    let fps = 60i64;
    let frame_secs = 1.0 / fps as f64;
    // reactivity is live-adjustable: the frame loop re-reads state.json so a slider
    // change applies WITHOUT restarting the background (no respawn, no intro replay).
    let mut reactivity = reactivity;
    let mut frames = 0u32;
    let mut cached_theme_colors: Vec<(String, String)> = Vec::new();
    loop {
        // Build the effect with default config via the clap parser (like upstream's
        // --random-effect), then drive it ourselves so we can pace it by audio.
        let mut effect = match ttfx::cli::Cli::try_parse_from(["ttfx", effect_name]) {
            Ok(ttfx::cli::Cli { effect: Some(e), .. }) => e.build_effect(),
            _ => { let m = format!("unknown ttfx effect: {effect_name}"); eprintln!("{m}"); log_dbg(&m); return Ok(()); }
        };
        // Canvas = the configured text as centered ASCII art (rebuilt each pass).
        let input = ttfx_canvas_input(ttfx_text, cols, rows);
        let mut config = TerminalConfig::default();
        config.canvas_width = cols as i64;
        config.canvas_height = rows as i64;
        config.frame_rate = fps;
        let mut ctx = match EngineCtx::new(&input, config, Rng::from_entropy(), Clock::virtual_with_frame_rate(fps)) {
            Ok(c) => c,
            Err(e) => { let m = format!("ttfx ctx error for {effect_name}: {e:?}"); eprintln!("{m}"); log_dbg(&m); return Ok(()); }
        };
        if let Err(e) = effect.build(&mut ctx) { let m = format!("ttfx build error for {effect_name}: {e:?}"); eprintln!("{m}"); log_dbg(&m); return Ok(()); }

        // Audio-paced frame loop: with a virtual clock each next_frame() advances the
        // animation by one tick, so pacing the calls by the live audio level makes the
        // effect speed up on loud passages / beats and slow down when quiet.
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        if ctx.terminal.prep_canvas(&mut out).is_err() { return Ok(()); }
        loop {
            // Audio-reactive color: bass -> hue shift, volume -> brightness. Same
            // reactivity slider that drives speed: 0=off, 3=triple. Applied in the
            // global sgr_color hook so every ttfx effect recolors without per-effect
            // patches. This is the bridge (vendored engine) - no upstream PR needed.
            {
                let vol = audio.volume().clamp(0.0, 1.0);
                let bass = (audio.band_at(0, NBANDS) + audio.band_at(1, NBANDS) * 0.5).clamp(0.0, 1.0);
                let active = audio_enabled && reactivity > 0 && (vol > 0.02 || bass > 0.05);
                if active {
                    // Brightness 1.0..1.5 (reactivity 3 = stronger), hue up to ~60 deg from bass + vol.
                    let bright = 1.0 + vol * 0.45 * (reactivity as f32 / 2.0);
                    let hue_deg = (bass * 45.0 + vol * 18.0) * (reactivity as f32 / 2.0) + if audio.beat() { 10.0 } else { 0.0 };
                    let rad = hue_deg.to_radians();
                    let (c, s) = (rad.cos(), rad.sin());
                    let t = 1.0 - c;
                    let w1 = 0.57735026; // 1/sqrt(3) for hue rotation around gray axis
                    let m = [
                        c + t/3.0,          t/3.0 - w1*s,       t/3.0 + w1*s,
                        t/3.0 + w1*s,       c + t/3.0,          t/3.0 - w1*s,
                        t/3.0 - w1*s,       t/3.0 + w1*s,       c + t/3.0,
                    ];
                    ttfx::utils::ansi::set_audio_color(true, bright, m);
                } else {
                    ttfx::utils::ansi::set_audio_color(false, 1.0, [1.0,0.0,0.0, 0.0,1.0,0.0, 0.0,0.0,1.0]);
                }
            }
            // Live theme color detection: when the user switches Omarchy themes,
            // recolor the ttfx effect on the fly without restarting.
            if use_theme_colors {
                let new_colors = read_theme_colors();
                if !new_colors.is_empty() {
                    let new_key: String = new_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                    let old_key: String = cached_theme_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                    if new_key != old_key {
                        cached_theme_colors = new_colors.clone();
                        // Find the accent color directly from the hex values (not ANSI).
                        let accent_hex = cached_theme_colors.iter().find(|(k, _)| k == "accent").map(|(_, v)| v.clone());
                        if let Some(accent) = accent_hex {
                            if let Some((r, g, b)) = parse_hex_color(&accent) {
                                let bright = 1.0;
                                let rr = r as f32 / 255.0;
                                let gg = g as f32 / 255.0;
                                let bb = b as f32 / 255.0;
                                let m = [
                                    rr, 0.0, 0.0,
                                    0.0, gg, 0.0,
                                    0.0, 0.0, bb,
                                ];
                                ttfx::utils::ansi::set_audio_color(true, bright, m);
                                log_dbg(&format!("ttfx theme colors changed: {effect_name} accent={accent}"));
                            }
                        }
                    }
                }
            }
            // Audio-reactive per-effect hook (e.g. thunderstorm lightning on loud beats).
            // Runs before next_frame so the effect can inject a strike this tick.
            if audio_enabled && reactivity > 0 {
                let vol = audio.volume().clamp(0.0, 1.0);
                let bass = (audio.band_at(0, NBANDS) + audio.band_at(1, NBANDS) * 0.5).clamp(0.0, 1.0);
                let beat = audio.beat();
                effect.on_audio(&mut ctx, vol, bass, beat);
            }
            // Stop if the PTY went away (effect settled / terminal gone).
            let frame = match effect.next_frame(&mut ctx) { Some(f) => f, None => break };
            if ctx.terminal.print_frame(&mut out, &frame).is_err() { let _ = ctx.terminal.restore_cursor(&mut out, ""); return Ok(()); }
            // Live config poll (~every 45 frames): apply reactivity changes without a
            // restart; on a structural change (effect picked / turned off) exit so the
            // controller respawns us with the new effect.
            frames += 1;
            if frames % 45 == 0 {
                let live = read_config();
                reactivity = live.reactivity;
                if live.effect != effect_name || !live.running { let _ = ctx.terminal.restore_cursor(&mut out, ""); return Ok(()); }
            }
            // Pace by the live audio level, scaled by the user's reactivity setting.
            // Louder => smaller delay => faster animation, capped at 3x (triple).
            // reactivity 0 disables it; higher reactivity reaches the cap more easily.
            let vol = audio.volume().clamp(0.0, 1.0);
            let boost = vol * reactivity as f32 + if audio.beat() { 0.5 } else { 0.0 };
            let speed = if audio_enabled && reactivity > 0 { (1.0 + boost).clamp(1.0, 3.0) } else { 1.0 };
            thread::sleep(Duration::from_secs_f64(frame_secs / speed as f64));
        }
        let _ = ctx.terminal.restore_cursor(&mut out, "");
        // Continuous background: loop all ttfx effects for the full rotate_secs
        // without a gap. The 400ms pause caused the visible "para y vuelve".
        // Check if Service already switched effect/off while we were running.
        {
            let live = read_config();
            if live.effect != effect_name || !live.running {
                let _ = ctx.terminal.restore_cursor(&mut out, "\n");
                return Ok(());
            }
        }
        // Tiny settle without blanking — next pass rebuilds immediately.
        let (c2, r2) = settle_pty_size(cols, rows);
        if (c2, r2) != (cols, rows) { cols = c2; rows = r2; }
    }
}

fn run_render(cfg: &Config, effect: &str, cols: usize, rows: usize, intensity: i64, audio: bool, byline: &str, intro_size: i64, cell_aspect: f32, show_fps: bool, with_intro: bool, ttfx_text: &str, reactivity: i64, intro_beat_sync: bool) -> Result<()> {
    log_dbg(&format!("run_render enter: effect={effect} {cols}x{rows} intensity={intensity} audio={audio} ttfx_text={ttfx_text} reactivity={reactivity} with_intro={with_intro} byline={byline} intro_beat_sync={intro_beat_sync}"));
    let intensity = intensity.clamp(0, 10);
    let state = AudioState::start(audio);
    // Use the REAL PTY size, but WAIT for it to settle first (Vte spawns at 80x24).
    let (cols, rows) = settle_pty_size(cols, rows);
    let mut scr = Screen::new(cols, rows, show_fps);

    let palette: Vec<String> = if cfg.use_theme_colors {
        // Use theme colors when user opts in; fall back gracefully if theme unavailable
        let tc = read_theme_colors();
        if !tc.is_empty() {
            theme_palette(&tc, effect).into_iter().collect()
        } else {
            hardcoded_palette(effect)
        }
    } else {
        hardcoded_palette(effect)
    };
    let palette_refs: Vec<&str> = palette.iter().map(|s| s.as_str()).collect();

    // Initialize the thread-local palette so effects can read live theme colors
    if cfg.use_theme_colors {
        set_theme_palette(palette.clone());
    }

    if with_intro {
        show_intro(&mut scr, &palette, byline, effect, intro_size, &state, intro_beat_sync);
    }

    // ttfx effects drive the vendored engine on this same PTY (after our intro).
    if is_ttfx_effect(effect) {
        run_ttfx(effect, cols, rows, ttfx_text, &state, audio, reactivity, cfg.use_theme_colors);
        return Ok(());
    }

    // Each effect returns Ok(()) when it detects a PTY resize; re-dispatch so
    // it restarts with fresh state at the new size (no jump, no stale grid).
    loop {
        let pal: &[String] = &palette;
        let res = match effect {
            "donut" => fx_donut(&mut scr, pal, intensity, &state, cell_aspect, cfg.use_theme_colors, effect),
            "fire" => fx_fire(&mut scr, pal, intensity, &state, cfg.use_theme_colors, effect),
            "starfield" => fx_starfield(&mut scr, pal, intensity, &state, cfg.use_theme_colors, effect),
            "life" => fx_life(&mut scr, pal, intensity, &state, cfg.use_theme_colors, effect),
            "wave" => fx_wave(&mut scr, pal, intensity, &state, cfg.use_theme_colors, effect),
            "bars" => fx_bars(&mut scr, pal, intensity, &state, cfg.use_theme_colors, effect),
            _ => fx_matrix(&mut scr, pal, intensity, &state, effect == "rain", cfg.use_theme_colors, effect),
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
fn fx_matrix(scr: &mut Screen, palette: &[String], intensity: i64, audio: &AudioState, rain: bool, use_theme_colors: bool, effect: &str) -> Result<()> {
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
    let mut prev_head: Vec<i32> = head.clone();
    let mut speed: Vec<i32> = (0..cols).map(|_| rng.gen_range(speed_base..speed_base + 2).max(1)).collect();
    let mut trail: Vec<usize> = (0..cols).map(|_| rng.gen_range(trail_base..trail_base + 12)).collect();
    // When the theme changes, only drops that RESPAWN after the change adopt the
    // new palette; drops already on screen keep their born color until they
    // scroll away. Each column's trail is painted entirely from its born palette
    // so every cell always has a real hue (never the white default foreground).
    let mut cur_pal: Vec<String> = palette.to_vec();
    let mut col_pal: Vec<Vec<String>> = (0..cols).map(|_| palette.to_vec()).collect();
    let mut cached_theme_colors: Vec<(String, String)> = Vec::new();
    loop {
        if scr.maybe_resize() { return Ok(()); }
        // Detect theme changes live and update the palette future drops use.
        if use_theme_colors {
            let new_colors = read_theme_colors();
            if !new_colors.is_empty() {
                let new_key: String = new_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                let old_key: String = cached_theme_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                if new_key != old_key {
                    cached_theme_colors = new_colors.clone();
                    cur_pal = theme_palette(&cached_theme_colors, effect);
                    set_theme_palette(cur_pal.clone());
                    log_dbg(&format!("theme colors changed: {effect} palette updated ({})", cur_pal.len()));
                }
            }
        }
        let vol = audio.volume();
        let spawn_chance = 0.35 + vol * 0.65;
        // A) Erase cells this column's trail vacated since the previous frame.
        for c in 0..cols {
            let h_prev = prev_head[c];
            let h = head[c];
            let vac_top = h_prev - trail[c] as i32 + 1;
            let vac_end = h - trail[c] as i32 + 1; // exclusive
            if vac_end > vac_top {
                for y in vac_top..vac_end.min(rows as i32) {
                    if y >= 0 && y < rows as i32 {
                        scr.erase(c, y as usize);
                    }
                }
            }
            // Column was reset/rested (head negative): clear its entire old trail
            if h < 0 && vac_top >= 0 {
                for y in vac_top..h_prev.min(rows as i32) {
                    if y >= 0 && y < rows as i32 {
                        scr.erase(c, y as usize);
                    }
                }
            }
        }
        let band = |c: usize| audio.band_at(c, cols);
        for c in 0..cols {
            let h = head[c];
            // Paint this column's drop entirely with ITS born palette.
            let pal = &col_pal[c];
            for t in 0..trail[c] {
                let y = h - t as i32;
                if y >= 0 && y < rows as i32 {
                    let ch = chars[rng.gen_range(0..chars.len())];
                    let idx = if t == 0 { 0 } else if t < 4 { 1 } else { 2 };
                    let idx = idx.min(pal.len().saturating_sub(1));
                    // fall back to a neutral theme hue if palette is empty
                    let color = pal.get(idx).cloned().filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "\x1b[32m".to_string());
                    scr.put(c, y as usize, ch, color);
                }
            }
            // Advance drop; on respawn adopt the current theme palette.
            prev_head[c] = h;
            head[c] = h + speed[c] + (band(c) * 2.0) as i32;
            if head[c] - trail[c] as i32 > rows as i32 {
                if rng.gen::<f32>() < spawn_chance {
                    head[c] = -(rng.gen_range(0..30));
                    speed[c] = rng.gen_range(speed_base..speed_base + 2).max(1);
                    trail[c] = rng.gen_range(trail_base..trail_base + 12);
                    col_pal[c] = cur_pal.clone();
                } else {
                    head[c] = -(rows as i32 + 10);
                }
            }
        }
        scr.fps_overlay();
        scr.present();
        thread::sleep(frame_delay(40, intensity, audio));
    }
}

// --- wave: layered sine waves scrolling horizontally ---
fn fx_wave(scr: &mut Screen, palette: &[String], intensity: i64, audio: &AudioState, use_theme_colors: bool, effect: &str) -> Result<()> {
    let mut t = 0f32;
    let mut cur_pal: Vec<String> = palette.to_vec();
    let mut cached_theme_colors: Vec<(String, String)> = Vec::new();
    loop {
        if scr.maybe_resize() { return Ok(()); }
        if use_theme_colors {
            let new_colors = read_theme_colors();
            if !new_colors.is_empty() {
                let new_key: String = new_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                let old_key: String = cached_theme_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                if new_key != old_key {
                    cached_theme_colors = new_colors.clone();
                    cur_pal = theme_palette(&cached_theme_colors, effect);
                    set_theme_palette(cur_pal.clone());
                    log_dbg(&format!("theme colors changed: {effect} palette updated ({})", cur_pal.len()));
                }
            }
        }
        scr.clear_dirty();
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
                    scr.put(x, y as usize, if layer == 0 { '~' } else { '-' }, tint_to_color(layer + 1, &cur_pal));
                }
            }
        }
        scr.fps_overlay();
        scr.present();
        t += 0.12 + lv * 0.25;
        thread::sleep(frame_delay(45, intensity, audio));
    }
}

// --- bars: equalizer driven by the REAL per-band spectrum (each bar = one
// frequency band's energy), smoothed so it follows without flicker. ---
fn fx_bars(scr: &mut Screen, palette: &[String], intensity: i64, audio: &AudioState, use_theme_colors: bool, effect: &str) -> Result<()> {
    let bands = NBANDS;
    let mut heights = vec![0f32; bands];
    let mut cur_pal: Vec<String> = palette.to_vec();
    let mut cached_theme_colors: Vec<(String, String)> = Vec::new();
    loop {
        if scr.maybe_resize() { return Ok(()); }
        if use_theme_colors {
            let new_colors = read_theme_colors();
            if !new_colors.is_empty() {
                let new_key: String = new_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                let old_key: String = cached_theme_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                if new_key != old_key {
                    cached_theme_colors = new_colors.clone();
                    cur_pal = theme_palette(&cached_theme_colors, effect);
                    set_theme_palette(cur_pal.clone());
                    log_dbg(&format!("theme colors changed: {effect} palette updated ({})", cur_pal.len()));
                }
            }
        }
        scr.clear_dirty();
        let (cols, rows) = (scr.cols, scr.rows);
        let maxh = rows as f32 * 0.9;
        let bw = cols / bands.max(1);
        for b in 0..bands {
            let energy = audio.band_at(b * (cols / bands.max(1)), cols);
            let target = (energy * 1.1).min(1.0) * maxh;
            heights[b] += (target - heights[b]) * 0.4;
            let h = heights[b] as usize;
            for y in 0..h.min(rows) {
                let tint = if y as f32 > h as f32 * 0.7 { 1 } else if y as f32 > h as f32 * 0.4 { 2 } else { 3 };
                for x in 0..bw.saturating_sub(1) {
                    scr.put(b * bw + x, rows - 1 - y, '#', tint_to_color(tint, &cur_pal));
                }
            }
        }
        scr.fps_overlay();
        scr.present();
        thread::sleep(frame_delay(50, intensity, audio));
    }
}

// --- donut: classic 3D torus, scaled to the grid. Spin speed follows audio. ---
// Scale follows the original donut.c proportions (30/80 horizontal, 15/22
// vertical), which already bake in the terminal cell aspect. That keeps the
// torus round on any screen; compressing by cell_aspect over-flattened it.
fn fx_donut(scr: &mut Screen, palette: &[String], intensity: i64, audio: &AudioState, cell_aspect: f32, use_theme_colors: bool, effect: &str) -> Result<()> {
    let _ = cell_aspect;
    let mut a = 0f32;
    let mut e = 1f32;
    let (cols, rows) = (scr.cols, scr.rows);
    let cx = cols as f32 / 2.0;
    let cy = rows as f32 / 2.0;
    let sx = cols as f32 * (30.0 / 80.0);
    let sy = rows as f32 * (15.0 / 22.0);
    let mut zbuf = vec![0f32; cols * rows];
    let mut cur_pal: Vec<String> = palette.to_vec();
    let mut cached_theme_colors: Vec<(String, String)> = Vec::new();
    loop {
        if scr.maybe_resize() { return Ok(()); }
        if use_theme_colors {
            let new_colors = read_theme_colors();
            if !new_colors.is_empty() {
                let new_key: String = new_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                let old_key: String = cached_theme_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                if new_key != old_key {
                    cached_theme_colors = new_colors.clone();
                    cur_pal = theme_palette(&cached_theme_colors, effect);
                    set_theme_palette(cur_pal.clone());
                    log_dbg(&format!("theme colors changed: {effect} palette updated ({})", cur_pal.len()));
                }
            }
        }
        scr.clear_dirty();
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
                    scr.put(x as usize, y as usize, ch, tint_to_color(tint, &cur_pal));
                }
                i += 0.02;
            }
            j += 0.07;
        }
        scr.fps_overlay();
        scr.present();
        let spin = 1.0 + lv * 2.0;
        a += 0.04 * spin;
        e += 0.02 * spin;
        thread::sleep(frame_delay(45, intensity, audio));
    }
}

// --- fire: classic doom fire from the bottom row. Height licks with audio. ---
fn fx_fire(scr: &mut Screen, palette: &[String], intensity: i64, audio: &AudioState, use_theme_colors: bool, effect: &str) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let (cols, rows) = (scr.cols, scr.rows);
    let palette_chars: Vec<char> = " .:-=+*#%@".chars().collect();
    let mut heat = vec![vec![0u8; cols]; rows];
    let mut cur_pal: Vec<String> = palette.to_vec();
    let mut cached_theme_colors: Vec<(String, String)> = Vec::new();
    loop {
        if scr.maybe_resize() { return Ok(()); }
        if use_theme_colors {
            let new_colors = read_theme_colors();
            if !new_colors.is_empty() {
                let new_key: String = new_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                let old_key: String = cached_theme_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                if new_key != old_key {
                    cached_theme_colors = new_colors.clone();
                    cur_pal = theme_palette(&cached_theme_colors, effect);
                    set_theme_palette(cur_pal.clone());
                    log_dbg(&format!("theme colors changed: {effect} palette updated ({})", cur_pal.len()));
                }
            }
        }
        let lv = audio.volume();
        let fuel = (28.0 + lv * 8.0 + intensity as f32 * 0.4) as u8;
        for x in 0..cols { heat[rows - 1][x] = fuel.min(36); }
        for y in 0..rows - 1 {
            for x in 0..cols {
                let src_x = (x as i32 + rng.gen_range(-1..=1)).clamp(0, cols as i32 - 1) as usize;
                let decay = rng.gen_range(0..=2);
                heat[y][x] = heat[y + 1][src_x].saturating_sub(decay);
            }
        }
        scr.clear_dirty();
        for y in 0..rows {
            for x in 0..cols {
                let h = heat[y][x] as usize;
                if h > 0 {
                    let ci = (h * palette_chars.len() / 37).min(palette_chars.len() - 1);
                    let tint = if h > 24 { 1 } else if h > 12 { 2 } else { 3 };
                    scr.put(x, y, palette_chars[ci], tint_to_color(tint, &cur_pal));
                }
            }
        }
        scr.fps_overlay();
        scr.present();
        thread::sleep(frame_delay(45, intensity, audio));
    }
}

// --- starfield: stars flying outward from the center. Speed follows audio. ---
fn fx_starfield(scr: &mut Screen, palette: &[String], intensity: i64, audio: &AudioState, use_theme_colors: bool, effect: &str) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let (cols, rows) = (scr.cols, scr.rows);
    let cx = cols as f32 / 2.0;
    let cy = rows as f32 / 2.0;
    let n = 160usize;
    let mut stars: Vec<(f32, f32, f32)> = (0..n).map(|_| (
        rng.gen_range(-1.0..1.0), rng.gen_range(-1.0..1.0), rng.gen_range(0.05..1.0),
    )).collect();
    let mut cur_pal: Vec<String> = palette.to_vec();
    let mut cached_theme_colors: Vec<(String, String)> = Vec::new();
    loop {
        if scr.maybe_resize() { return Ok(()); }
        if use_theme_colors {
            let new_colors = read_theme_colors();
            if !new_colors.is_empty() {
                let new_key: String = new_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                let old_key: String = cached_theme_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                if new_key != old_key {
                    cached_theme_colors = new_colors.clone();
                    cur_pal = theme_palette(&cached_theme_colors, effect);
                    set_theme_palette(cur_pal.clone());
                    log_dbg(&format!("theme colors changed: {effect} palette updated ({})", cur_pal.len()));
                }
            }
        }
        scr.clear_dirty();
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
                scr.put(px as usize, py as usize, ch, tint_to_color(tint, &cur_pal));
            }
        }
        scr.fps_overlay();
        scr.present();
        thread::sleep(frame_delay(40, intensity, audio));
    }
}

// --- life: Conway's Game of Life, reseeded on stagnation ---
fn fx_life(scr: &mut Screen, palette: &[String], intensity: i64, audio: &AudioState, use_theme_colors: bool, effect: &str) -> Result<()> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let (cols, rows) = (scr.cols, scr.rows);
    let mut grid = vec![vec![false; cols]; rows];
    let mut next = vec![vec![false; cols]; rows];
    let mut age = vec![vec![0u8; cols]; rows];
    let density = 0.18 + audio.volume() * 0.1;
    for y in 0..rows { for x in 0..cols { grid[y][x] = rng.gen::<f32>() < density; } }
    let mut stagnant = 0u32;
    let mut cur_pal: Vec<String> = palette.to_vec();
    let mut cached_theme_colors: Vec<(String, String)> = Vec::new();
    loop {
        if scr.maybe_resize() { return Ok(()); }
        if use_theme_colors {
            let new_colors = read_theme_colors();
            if !new_colors.is_empty() {
                let new_key: String = new_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                let old_key: String = cached_theme_colors.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<_>>().join("|");
                if new_key != old_key {
                    cached_theme_colors = new_colors.clone();
                    cur_pal = theme_palette(&cached_theme_colors, effect);
                    set_theme_palette(cur_pal.clone());
                    log_dbg(&format!("theme colors changed: {effect} palette updated ({})", cur_pal.len()));
                }
            }
        }
        scr.clear_dirty();
        for y in 0..rows {
            for x in 0..cols {
                if grid[y][x] {
                    age[y][x] = age[y][x].saturating_add(1);
                    let tint = if age[y][x] > 30 { 1 } else if age[y][x] > 8 { 2 } else { 3 };
                    scr.put(x, y, if age[y][x] > 8 { 'O' } else { 'o' }, tint_to_color(tint, &cur_pal));
                } else {
                    age[y][x] = 0;
                }
            }
        }
        scr.fps_overlay();
        scr.present();
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
