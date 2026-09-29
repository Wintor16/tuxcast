use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;

use crate::core::platform::GameWindow;
use crate::core::types::{PxPoint, PxRect, WindowInfo};

/// Finds and focuses the game window on KDE Plasma / Wayland by driving
/// KWin's D-Bus scripting interface (`org.kde.kwin.Scripting`).
///
/// KWin scripts can't return values over D-Bus, so each script writes a
/// single marker-prefixed JSON line via `print()`, which KWin forwards to
/// its own stdout/journal; we read it back with `journalctl` right after
/// the (synchronous) `run()` call returns.
///
/// Each query loads a fresh, uniquely-named script rather than reusing one
/// loaded once at startup: a script kept loaded and `run()` repeatedly
/// observably stops seeing live changes to `workspace.windowList()` after
/// a window migrates to a different output (verified directly — a brand
/// new script sees the move immediately, a long-lived one calling `run()`
/// again does not), which looks like KWin's JS engine memoizing something
/// per loaded-script-instance across runs. Reloading avoids relying on
/// that undocumented behavior. The load/unload overhead is a few ms,
/// comfortably inside the bot loop's polling budget.
pub struct KWinWindow {
    match_js: String,
    conn: Mutex<Option<zbus::blocking::Connection>>,
    call_id: AtomicU64,
}

impl KWinWindow {
    pub fn new(_title: &str) -> Self {
        // "Roblox" itself never runs natively on Linux; players use a
        // compatibility client, overwhelmingly Sober (resourceClass
        // "org.vinegarhq.Sober"). Matching is on resourceClass only — a
        // stable application identifier, not user-visible — never on the
        // window's caption/title: an earlier version also matched the
        // caption, which risked matching an unrelated window whose *title*
        // happened to contain "sober" or "roblox" (a browser tab, a Discord
        // channel name, ...).
        let needles = ["roblox", "sober"];
        let checks: Vec<String> =
            needles.iter().map(|n| format!("rc.indexOf({n:?}) !== -1")).collect();
        Self {
            match_js: checks.join(" || "),
            conn: Mutex::new(None),
            call_id: AtomicU64::new(0),
        }
    }

    fn conn(&self) -> Option<zbus::blocking::Connection> {
        let mut guard = self.conn.lock();
        if guard.is_none() {
            *guard = zbus::blocking::Connection::session().ok();
        }
        guard.clone()
    }

    /// A name unique to this call, this process, and this run of the
    /// process — never reused, so there's nothing for KWin to have
    /// stale-cached from a previous load.
    fn unique_name(&self, kind: &str) -> String {
        let id = self.call_id.fetch_add(1, Ordering::Relaxed);
        format!("gpoaf_{kind}_{}_{id}", std::process::id())
    }

    fn find_matching_js(&self, body: &str) -> String {
        format!(
            r#"
var list = workspace.windowList();
var found = null;
for (var i = 0; i < list.length; i++) {{
    var w = list[i];
    var rc = (w.resourceClass || "").toLowerCase();
    if ({match_js}) {{
        found = w;
        break;
    }}
}}
{body}
"#,
            match_js = self.match_js,
        )
    }
}

fn load_script(conn: &zbus::blocking::Connection, name: &str, source: &str) -> Option<String> {
    let path = std::env::temp_dir().join(format!("{name}.js"));
    let mut f = std::fs::File::create(&path).ok()?;
    f.write_all(source.as_bytes()).ok()?;
    drop(f);

    let reply = conn
        .call_method(
            Some("org.kde.KWin"),
            "/Scripting",
            Some("org.kde.kwin.Scripting"),
            "loadScript",
            &(path.to_str()?, name),
        )
        .ok()?;
    let id: i32 = reply.body().deserialize().ok()?;
    Some(format!("/Scripting/Script{id}"))
}

fn unload_script(conn: &zbus::blocking::Connection, name: &str) {
    let _ = conn.call_method(
        Some("org.kde.KWin"),
        "/Scripting",
        Some("org.kde.kwin.Scripting"),
        "unloadScript",
        &(name,),
    );
}

/// Loads `source` under a fresh name, runs it once, reads back the line
/// printed with `marker`, then unloads it. Not reused across calls — see
/// the `KWinWindow` doc comment for why.
fn run_once(conn: &zbus::blocking::Connection, name: &str, source: &str, marker: &str) -> Option<String> {
    let script_path = load_script(conn, name, source)?;
    let result = (|| {
        conn.call_method(
            Some("org.kde.KWin"),
            script_path.as_str(),
            Some("org.kde.kwin.Script"),
            "run",
            &(),
        )
        .ok()?;

        let out = Command::new("journalctl")
            .args(["--user", "-t", "kwin_wayland", "-n", "200", "--no-pager", "-o", "cat"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines().rev().find_map(|l| l.strip_prefix(marker)).map(|s| s.to_string())
    })();
    unload_script(conn, name);
    result
}

impl KWinWindow {
    fn find_raw(&self) -> Option<(WindowInfo, Option<String>)> {
        let conn = self.conn()?;
        let name = self.unique_name("find");
        let marker = format!("GPOAF_WINDOW_{name} ");
        let src = self.find_matching_js(&format!(
            r#"
if (found) {{
    print("{marker}" + JSON.stringify({{
        x: found.x, y: found.y, width: found.width, height: found.height,
        active: found.active, minimized: found.minimized,
        output: found.output ? found.output.name : null
    }}));
}} else {{
    print("{marker}null");
}}
"#
        ));
        let payload = run_once(&conn, &name, &src, &marker)?;
        if payload == "null" {
            return None;
        }
        let v: serde_json::Value = serde_json::from_str(&payload).ok()?;
        let client = PxRect {
            x: v.get("x")?.as_i64()? as i32,
            y: v.get("y")?.as_i64()? as i32,
            w: v.get("width")?.as_i64()? as i32,
            h: v.get("height")?.as_i64()? as i32,
        };
        let active = v.get("active")?.as_bool()?;
        let minimized = v.get("minimized")?.as_bool()?;
        if client.is_empty() && !minimized {
            return None;
        }
        let output = v.get("output").and_then(|o| o.as_str()).map(str::to_string);
        Some((WindowInfo { client, is_foreground: active, visible: !minimized, dpi: 96 }, output))
    }

    /// The name of the physical output (monitor) the window currently sits
    /// on (e.g. "eDP-1"), used to detect when it's dragged to a different
    /// screen so the portal capture pipeline can be reconnected — KWin's
    /// window-scoped PipeWire stream doesn't reliably keep following a
    /// window across an output change on its own.
    pub fn current_output(&self) -> Option<String> {
        self.find_raw()?.1
    }

    /// `find()` plus the output name, in one KWin round-trip — for a
    /// caller (capture's low-frequency cache refresh) that needs both and
    /// shouldn't pay for two separate queries.
    pub fn find_with_output(&self) -> Option<(WindowInfo, Option<String>)> {
        self.find_raw()
    }
}

impl GameWindow for KWinWindow {
    fn find(&self) -> Option<WindowInfo> {
        self.find_raw().map(|(info, _)| info)
    }

    fn focus(&self) -> bool {
        let Some(conn) = self.conn() else { return false };
        let name = self.unique_name("focus");
        let marker = format!("GPOAF_FOCUS_{name} ");
        let src = self.find_matching_js(&format!(
            r#"
if (found) {{
    if (found.minimized) {{ found.minimized = false; }}
    workspace.activeWindow = found;
    print("{marker}" + (workspace.activeWindow === found ? "ok" : "fail"));
}} else {{
    print("{marker}notfound");
}}
"#
        ));
        run_once(&conn, &name, &src, &marker).as_deref() == Some("ok")
    }
}

/// One-shot query of the real OS cursor position via KWin scripting
/// (`workspace.cursorPos`). Not currently used in the shipped input path
/// (see the module-level note on why absolute portal motion was kept
/// instead of relative), kept for diagnostics/tooling.
pub fn cursor_pos() -> Option<PxPoint> {
    let conn = zbus::blocking::Connection::session().ok()?;
    let name = format!("gpoaf_cursor_{}", std::process::id());
    let marker = format!("GPOAF_CURSOR_{name} ");
    let src = format!(r#"print({marker:?} + JSON.stringify(workspace.cursorPos));"#);
    let payload = run_once(&conn, &name, &src, &marker)?;
    let v: serde_json::Value = serde_json::from_str(&payload).ok()?;
    Some(PxPoint { x: v.get("x")?.as_i64()? as i32, y: v.get("y")?.as_i64()? as i32 })
}

/// The primary monitor's geometry, per KDE's own display configuration
/// (`kscreen-doctor`'s `priority: 1` output) — not necessarily output 0 or
/// the leftmost one. Wayland gives clients no "center on the primary
/// screen" placement of their own, so windows without a game window to
/// anchor to (see `center_panel_on_primary_screen`) need this to land
/// somewhere predictable instead of wherever the compositor happens to
/// place new surfaces.
fn primary_screen_rect() -> Option<PxRect> {
    let out = std::process::Command::new("kscreen-doctor").arg("--json").arg("-o").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // kscreen-doctor prints one JSON object followed by a human-readable
    // dump; only the JSON prefix parses.
    let root: serde_json::Value =
        serde_json::Deserializer::from_str(&text).into_iter().next()?.ok()?;
    let outputs = root.get("outputs")?.as_array()?;
    let primary = outputs
        .iter()
        .filter(|o| o.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false))
        .min_by_key(|o| o.get("priority").and_then(|v| v.as_i64()).unwrap_or(i64::MAX))?;
    let pos = primary.get("pos")?;
    let size = primary.get("size")?;
    Some(PxRect {
        x: pos.get("x")?.as_i64()? as i32,
        y: pos.get("y")?.as_i64()? as i32,
        w: size.get("width")?.as_i64()? as i32,
        h: size.get("height")?.as_i64()? as i32,
    })
}

/// Centers our panel window on the primary monitor. Only meaningful when
/// there's no game window to anchor to instead (windows.rs already
/// positions the panel relative to Roblox/Sober whenever one is found);
/// called once when the panel is about to be shown with no such anchor,
/// not on a timer — unlike `keepAbove`, a `frameGeometry` set this way
/// sticks until something else moves the window.
pub fn center_panel_on_primary_screen() {
    let Some(primary) = primary_screen_rect() else { return };
    let Some(conn) = zbus::blocking::Connection::session().ok() else { return };
    let src = format!(
        r#"
var list = workspace.windowList();
for (var i = 0; i < list.length; i++) {{
    var w = list[i];
    var rc = (w.resourceClass || "").toLowerCase();
    if (rc.indexOf("gpo-autofish") !== -1 && w.caption === "TuxCast") {{
        var px = {x} + Math.round(({pw} - w.width) / 2);
        var py = {y} + Math.round(({ph} - w.height) / 2);
        // Reassigning frameGeometry even to its current value can cause a
        // restack (observed stealing focus from the game every time this
        // runs), so only touch it when the window is actually elsewhere.
        if (w.x !== px || w.y !== py) {{
            w.frameGeometry = {{ x: px, y: py, width: w.width, height: w.height }};
        }}
    }}
}}
"#,
        x = primary.x,
        y = primary.y,
        pw = primary.w,
        ph = primary.h,
    );
    let name = format!("gpoaf_center_{}", std::process::id());
    unload_script(&conn, &name);
    let Some(script_path) = load_script(&conn, &name, &src) else { return };
    let _ = conn.call_method(
        Some("org.kde.KWin"),
        script_path.as_str(),
        Some("org.kde.kwin.Script"),
        "run",
        &(),
    );
    unload_script(&conn, &name);
}

/// Forces `keepAbove` on all of our own windows (panel/HUD/overlay/guide,
/// which all share the `gpo-autofish` window class) via KWin scripting.
///
/// Tauri's own `alwaysOnTop` (set in tauri.conf.json) is a no-op on
/// Wayland — clients can't request stacking order for themselves there,
/// only the compositor can grant it — so this is the actual mechanism
/// that makes it work on KDE. Safe/cheap to call repeatedly (e.g. every
/// time a window is (re)shown): each call is a few ms.
pub fn pin_app_windows_above() {
    let Some(conn) = zbus::blocking::Connection::session().ok() else { return };
    let src = r#"
var list = workspace.windowList();
for (var i = 0; i < list.length; i++) {
    var w = list[i];
    var rc = (w.resourceClass || "").toLowerCase();
    // Reassigning keepAbove even to its current value can cause a
    // restack (observed stealing focus from the game every time this
    // runs), so only touch it when it's actually not set yet.
    if (rc.indexOf("gpo-autofish") !== -1 && !w.keepAbove) {
        w.keepAbove = true;
    }
}
"#;
    let name = format!("gpoaf_pin_{}", std::process::id());
    unload_script(&conn, &name);
    let Some(script_path) = load_script(&conn, &name, src) else { return };
    let _ = conn.call_method(
        Some("org.kde.KWin"),
        script_path.as_str(),
        Some("org.kde.kwin.Script"),
        "run",
        &(),
    );
    unload_script(&conn, &name);
}
