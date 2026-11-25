//! The plugin's view: what is on the network, the connections and who Corvus
//! is on them, what the map shows, the contacts, a marker to place and chat.

// Under other names: the key check of the program reads the language files of the
// application for every literal handed to `translate(`, and a plugin keeps its own.
use corvus_api::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use corvus_api::i18n::{tr as translate, tr_fmt as translate_fmt};
use corvus_api::plugin::shapes::{hex, paint_glyph};
use corvus_api::plugin::Host;
use corvus_api::ui::card;
use corvus_api::ui::kit::{self, Button, Choice, Variant};
use corvus_api::ui::listrow::{self, Mark};
use corvus_api::ui::style::{self, Weight};
use corvus_api::ui::tip::HoverTr;
use corvus_api::ui::{formfield, icons};
use serde_json::{json, Value};

use super::model::{age_text, distance_m, distance_text, Contact, Filter, AFFILIATIONS, CLASSES, ROLES, TEAMS};
use super::model::{ENTITIES_PLUGIN, ENTITY_UID_PREFIX};
use super::{Editor, Tak};

const GAP: f32 = 10.0;
const ROW_GAP: f32 = 5.0;
const MAX_ROWS: usize = 40;
const MAX_CHAT_SHOWN: usize = 6;

pub fn show(p: &mut Tak, ui: &mut egui::Ui, area: Rect, h: &mut Host) -> f32 {
    let (x, w) = (area.left(), area.width());
    let mut y = area.top();
    y = summary(p, ui, x, y, w) + GAP;
    y = connections(p, ui, x, y, w, h) + GAP;
    if p.editor.is_some() {
        y = editor(p, ui, x, y, w, h) + GAP;
    }
    y = identity(p, ui, x, y, w, h) + GAP;
    y = map_filter(p, ui, x, y, w, h) + GAP;
    y = contacts(p, ui, x, y, w, h) + GAP;
    y = marker_form(p, ui, x, y, w, h) + GAP;
    y = chat(p, ui, x, y, w, h);
    y - area.top()
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) * (1.0 - t) + f32::from(y) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// A 26 px disc tinted with the contact's colour and its symbol in it.
fn badge(ui: &egui::Ui, at: Pos2, icon: &str, color: Color32, stale: bool) {
    let r = Rect::from_min_size(at, Vec2::splat(26.0));
    let color = if stale { color.gamma_multiply(0.5) } else { color };
    let p = ui.painter();
    p.circle_filled(r.center(), 13.0, mix(style::surface_1(), color, 0.16));
    p.circle_stroke(r.center(), 12.25, Stroke::new(1.5, mix(color, style::text_1(), 0.42)));
    paint_glyph(
        ui,
        icon,
        Rect::from_center_size(r.center(), Vec2::splat(15.0)),
        mix(color, style::text_1(), 0.52),
        2.0,
    );
}

/// A small caps title over a hairline, tools at the right.
fn head<T>(
    ui: &mut egui::Ui,
    x: f32,
    y: f32,
    w: f32,
    title: &str,
    tools: impl FnOnce(&mut egui::Ui, f32) -> T,
) -> (f32, T) {
    let g = style::galley(ui, &title.to_uppercase(), 10.0, Weight::W400, style::text_3(), 0.9);
    let row_h = g.size().y.max(24.0);
    ui.painter()
        .galley(Pos2::new(x, y + (row_h - g.size().y) / 2.0), g, style::text_3());
    let out = tools(ui, x + w);
    let line = y + row_h + 3.0;
    ui.painter().hline(
        egui::Rangef::new(x, x + w),
        line + 0.5,
        Stroke::new(1.0, style::border_soft()),
    );
    (line + 1.0, out)
}

/// A square icon button at the right edge, `slot` places from it.
fn tool(ui: &mut egui::Ui, right: f32, y: f32, slot: usize, icon: &'static str, title: &str) -> bool {
    let r = Rect::from_min_size(Pos2::new(right - (slot + 1) as f32 * 25.0 + 1.0, y), Vec2::splat(24.0));
    icon_button(ui, r, icon, title, ("tak-tool", icon, slot))
}

fn icon_button(
    ui: &mut egui::Ui,
    r: Rect,
    icon: &'static str,
    title: &str,
    id: impl std::hash::Hash + std::fmt::Debug,
) -> bool {
    let resp = ui
        .interact(r, ui.scope_id().with(id), egui::Sense::click())
        .on_hover_tr(title);
    let (fill, ink) = if resp.hovered() {
        (style::surface_hover(), style::text_1())
    } else {
        (Color32::TRANSPARENT, style::text_3())
    };
    ui.painter().rect_filled(r, style::RADIUS_BTN, fill);
    icons::paint(ui, icon, Rect::from_center_size(r.center(), Vec2::splat(14.0)), ink);
    resp.clicked()
}

/// One line of text clipped to `w`; returns its height.
#[allow(clippy::too_many_arguments)]
fn line(ui: &egui::Ui, x: f32, y: f32, w: f32, text: &str, size: f32, weight: Weight, color: Color32) -> f32 {
    let g = style::galley(ui, text, size, weight, color, 0.0);
    let h = g.size().y;
    let clip = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w.max(0.0), h + 2.0));
    ui.painter_at(clip).galley(Pos2::new(x, y), g, color);
    h
}

/// Wrapped text; returns its height.
fn note(ui: &egui::Ui, x: f32, y: f32, w: f32, text: &str, color: Color32) -> f32 {
    let g = kit::wrapped_weight(ui, text, 11.0, Weight::W400, color, w, 1.4);
    let h = g.size().y;
    ui.painter().galley(Pos2::new(x, y), g, color);
    h
}

fn state_color(state: &str) -> Color32 {
    match state {
        "connected" | "listening" => style::healthy(),
        "connecting" => style::warning(),
        "error" => style::critical(),
        _ => style::text_3(),
    }
}

fn state_label(state: &str) -> &'static str {
    match state {
        "connected" => "Connected",
        "listening" => "Listening",
        "connecting" => "Connecting",
        "error" => "Not connected",
        _ => "Off",
    }
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "multicast" => "Local network",
        "udp" => "UDP",
        "tcp" => "TAK Server (TCP)",
        "tls" => "TAK Server (TLS)",
        _ => "Connection",
    }
}

fn summary(p: &Tak, ui: &egui::Ui, x: f32, y: f32, w: f32) -> f32 {
    let shown = p
        .mirror
        .contacts
        .values()
        .filter(|c| p.filter.shows(c, p.opts.stale))
        .count();
    let conns = p.mirror.connections();
    let sharing = p.mirror.status.get("share_position") == Some(&Value::Bool(true));
    let cells = [
        ("Contacts", format!("{shown} / {}", p.mirror.contacts.len()), ""),
        (
            "Connected",
            format!("{} / {}", p.mirror.connections_on(), conns.len()),
            "",
        ),
        (
            "Sharing",
            (if sharing { "On" } else { "Off" }).to_string(),
            if sharing { "warn" } else { "" },
        ),
    ];
    let cw = (w - 2.0 * ROW_GAP) / 3.0;
    for (i, (label, value, level)) in cells.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(x + i as f32 * (cw + ROW_GAP), y), Vec2::new(cw, 44.0));
        let (edge, ink) = if *level == "warn" {
            (mix(style::border_soft(), style::warning(), 0.55), style::warning())
        } else {
            (style::border_soft(), style::text_1())
        };
        card::paint(
            ui.painter(),
            r,
            card::Look::custom(style::surface_2(), edge, style::RADIUS_BTN),
        );
        let lg = style::galley(ui, &label.to_uppercase(), 9.5, Weight::W400, style::text_2(), 0.7);
        ui.painter()
            .galley(Pos2::new(r.left() + 10.0, r.top() + 7.0), lg, style::text_2());
        let vg = style::galley_mono(ui, &translate(value), 13.0, ink);
        ui.painter_at(r.shrink2(Vec2::new(6.0, 0.0)))
            .galley(Pos2::new(r.left() + 10.0, r.top() + 22.0), vg, ink);
    }
    y + 44.0
}

// ---- connections --------------------------------------------------------------

fn connections(p: &mut Tak, ui: &mut egui::Ui, x: f32, y: f32, w: f32, h: &mut Host) -> f32 {
    let (mut bottom, add) = head(ui, x, y, w, "Connections", |ui, right| {
        tool(ui, right, y, 0, "plus", "Add a connection")
    });
    bottom += ROW_GAP;
    if add && p.editor.is_none() {
        p.editor = Some(Editor {
            kind: "tcp".into(),
            port: "8087".into(),
            send: true,
            ..Editor::default()
        });
    }
    let rows: Vec<Value> = p.mirror.connections().to_vec();
    if rows.is_empty() {
        bottom += note(
            ui,
            x + 2.0,
            bottom + 2.0,
            w - 4.0,
            "No connection yet. Add the local network to hear ATAK and WinTAK on the same Wi-Fi, or a TAK Server.",
            style::text_3(),
        ) + 6.0;
        return bottom;
    }
    for c in &rows {
        let id = c["id"].as_str().unwrap_or("").to_string();
        let on = c["enabled"].as_bool().unwrap_or(false);
        let state = c["state"].as_str().unwrap_or("off");
        let r = Rect::from_min_size(Pos2::new(x, bottom), Vec2::new(w, 42.0));
        card::paint(
            ui.painter(),
            r,
            card::Look::custom(style::surface_2(), style::border_soft(), style::RADIUS_BTN),
        );
        kit::dot(
            ui.painter(),
            Pos2::new(r.left() + 14.0, r.center().y),
            state_color(state),
            on,
        );
        let room = w - 28.0 - 38.0 - 24.0 - 26.0;
        let name = c["name"].as_str().unwrap_or("");
        line(
            ui,
            r.left() + 28.0,
            r.top() + 6.0,
            room,
            name,
            12.0,
            Weight::W600,
            style::text_1(),
        );
        let sub = format!(
            "{} {}:{} \u{b7} {}",
            translate(kind_label(c["kind"].as_str().unwrap_or(""))),
            c["host"].as_str().unwrap_or(""),
            c["port"].as_u64().unwrap_or(0),
            translate(state_label(state)),
        );
        line(
            ui,
            r.left() + 28.0,
            r.top() + 23.0,
            room,
            &sub,
            10.5,
            Weight::W400,
            style::text_3(),
        );
        let sw = Pos2::new(r.right() - 10.0 - 38.0, r.center().y - 11.0);
        if kit::switch(ui, sw, on, true, ("tak-conn", &id)) {
            p.act(
                "connection",
                "/api/tak/connection",
                json!({"id": id, "enabled": !on}),
                h,
            );
        }
        let pen = Rect::from_min_size(Pos2::new(sw.x - 6.0 - 24.0, r.center().y - 12.0), Vec2::splat(24.0));
        if icon_button(ui, pen, "pencil", "Change this connection", ("tak-edit", &id)) {
            p.editor = Some(editor_of(c));
        }
        bottom = r.bottom() + ROW_GAP;
        let detail = c["detail"].as_str().unwrap_or("");
        if state == "error" && !detail.is_empty() {
            bottom += note(ui, x + 4.0, bottom, w - 8.0, detail, style::critical()) + ROW_GAP;
        }
    }
    bottom - ROW_GAP
}

fn editor_of(c: &Value) -> Editor {
    let text = |k: &str| c[k].as_str().unwrap_or("").to_string();
    Editor {
        id: Some(text("id")),
        name: text("name"),
        kind: text("kind"),
        host: text("host"),
        port: c["port"].as_u64().unwrap_or(0).to_string(),
        send: c["send"].as_bool().unwrap_or(true),
        ca_file: text("ca_file"),
        cert_file: text("cert_file"),
        key_file: text("key_file"),
        error: String::new(),
    }
}

fn default_port(kind: &str) -> &'static str {
    match kind {
        "multicast" => "6969",
        "tls" => "8089",
        "udp" => "4242",
        _ => "8087",
    }
}

fn editor(p: &mut Tak, ui: &mut egui::Ui, x: f32, y: f32, w: f32, h: &mut Host) -> f32 {
    let Some(mut ed) = p.editor.take() else { return y };
    let title = if ed.id.is_some() {
        "Change the connection"
    } else {
        "New connection"
    };
    let (mut bottom, ()) = head(ui, x, y, w, title, |_, _| ());
    bottom += GAP;
    bottom = formfield::field(ui, x, bottom, w, "Name", "", |ui, cy| {
        formfield::input(ui, x, cy, w, &mut ed.name, "TAK Server", false, false, "tak-ed-name");
        formfield::control_h()
    }) + GAP;
    let kinds = [
        Choice::new("multicast", translate("Local network (multicast)").into_owned()),
        Choice::new("udp", "UDP"),
        Choice::new("tcp", translate("TAK Server (TCP)").into_owned()),
        Choice::new("tls", translate("TAK Server (TLS)").into_owned()),
    ];
    let mut picked = None;
    bottom = formfield::field(ui, x, bottom, w, "Kind", "", |ui, cy| {
        picked = formfield::select(ui, x, cy, w, &kinds, &ed.kind, "tak-ed-kind");
        formfield::control_h()
    }) + GAP;
    if let Some(k) = picked {
        if ed.port.trim().is_empty() || ed.port == default_port(&ed.kind) {
            ed.port = default_port(&k).to_string();
        }
        if k == "multicast" && ed.host.trim().is_empty() {
            ed.host = "239.2.3.1".into();
        }
        ed.kind = k;
    }
    let port_w = 84.0;
    let host_w = w - port_w - ROW_GAP;
    let host_hint = if ed.kind == "multicast" {
        "239.2.3.1"
    } else {
        "tak.example.org"
    };
    let cap = formfield::caption(ui, x, bottom, "Host", "", "tak-ed-host-cap");
    formfield::caption(ui, x + host_w + ROW_GAP, bottom, "Port", "", "tak-ed-port-cap");
    let iy = bottom + cap + formfield::GAP;
    formfield::input(ui, x, iy, host_w, &mut ed.host, host_hint, true, false, "tak-ed-host");
    formfield::input(
        ui,
        x + host_w + ROW_GAP,
        iy,
        port_w,
        &mut ed.port,
        "8087",
        true,
        false,
        "tak-ed-port",
    );
    bottom = iy + formfield::control_h() + GAP;
    if ed.kind == "tls" {
        for (cap, hint, slot) in [
            (
                "CA file",
                "PEM file of the server's CA, if the system does not know it",
                0,
            ),
            ("Client certificate", "PEM file, if the server asks for one", 1),
            ("Client key", "PEM file of the key that belongs to it", 2),
        ] {
            let field = match slot {
                0 => &mut ed.ca_file,
                1 => &mut ed.cert_file,
                _ => &mut ed.key_file,
            };
            bottom = formfield::field(ui, x, bottom, w, cap, "", |ui, cy| {
                formfield::input(ui, x, cy, w, field, hint, true, false, ("tak-ed-file", slot));
                formfield::control_h()
            }) + GAP;
        }
    }
    let mut send = ed.send;
    if kit::checkbox(
        ui,
        Pos2::new(x, bottom),
        w,
        "Send to this connection",
        &mut send,
        false,
        "tak-ed-send",
    ) {
        ed.send = send;
    }
    bottom += 22.0;
    if !ed.error.is_empty() {
        bottom += note(ui, x, bottom, w, &ed.error, style::critical()) + 6.0;
    }
    let save = Button::new("Save", Variant::Primary).small();
    let cancel = Button::new("Cancel", Variant::Secondary).small();
    let remove = Button::new("Remove", Variant::DangerOutline).small();
    let (sw, cw, rw) = (save.width(ui), cancel.width(ui), remove.width(ui));
    let bh = save.height();
    let by = bottom + 4.0;
    let mut keep = true;
    if save.show(
        ui,
        Rect::from_min_size(Pos2::new(x, by), Vec2::new(sw, bh)),
        "tak-ed-save",
    ) {
        match editor_body(&ed) {
            Ok(body) => {
                p.act("connection", "/api/tak/connection", body, h);
                keep = false;
            }
            Err(e) => ed.error = e,
        }
    }
    if cancel.show(
        ui,
        Rect::from_min_size(Pos2::new(x + sw + ROW_GAP, by), Vec2::new(cw, bh)),
        "tak-ed-cancel",
    ) {
        keep = false;
    }
    if let Some(id) = ed.id.clone() {
        let r = Rect::from_min_size(Pos2::new(x + w - rw, by), Vec2::new(rw, bh));
        if remove.show(ui, r, "tak-ed-remove") {
            p.act("connection", "/api/tak/connection/remove", json!({"id": id}), h);
            keep = false;
        }
    }
    if keep {
        p.editor = Some(ed);
    }
    by + bh
}

fn editor_body(ed: &Editor) -> Result<Value, String> {
    let port: u16 = ed
        .port
        .trim()
        .parse()
        .ok()
        .filter(|p| *p != 0)
        .ok_or_else(|| translate("The port is a number from 1 to 65535.").into_owned())?;
    if ed.host.trim().is_empty() {
        return Err(translate("Enter a host name or an address.").into_owned());
    }
    let mut body = json!({
        "name": ed.name.trim(), "kind": ed.kind, "host": ed.host.trim(), "port": port, "send": ed.send,
        "ca_file": ed.ca_file.trim(), "cert_file": ed.cert_file.trim(), "key_file": ed.key_file.trim(),
    });
    if let (Some(id), Some(o)) = (&ed.id, body.as_object_mut()) {
        o.insert("id".into(), id.clone().into());
    }
    Ok(body)
}

// ---- who Corvus is ------------------------------------------------------------

fn identity(p: &mut Tak, ui: &mut egui::Ui, x: f32, y: f32, w: f32, h: &mut Host) -> f32 {
    let (mut bottom, ()) = head(ui, x, y, w, "Corvus on TAK", |_, _| ());
    bottom += GAP;
    let st = p.mirror.status.clone();
    let saved = st["callsign"].as_str().unwrap_or("Corvus").to_string();
    let share = st["share_position"].as_bool().unwrap_or(false);
    let mut text = p.form.callsign.clone().unwrap_or_else(|| saved.clone());
    let apply = Button::new("Apply", Variant::Secondary).small();
    let aw = apply.width(ui);
    let changed = text.trim() != saved && !text.trim().is_empty();
    let field_w = if changed { w - aw - ROW_GAP } else { w };
    let cap = formfield::caption(ui, x, bottom, "Callsign", "", "tak-callsign-cap");
    let iy = bottom + cap + formfield::GAP;
    let resp = formfield::input(ui, x, iy, field_w, &mut text, "Corvus", false, false, "tak-callsign");
    if resp.changed() {
        p.form.callsign = Some(text.clone());
    }
    if changed {
        let ar = Rect::from_min_size(
            Pos2::new(
                x + field_w + ROW_GAP,
                iy + (formfield::control_h() - apply.height()) / 2.0,
            ),
            Vec2::new(aw, apply.height()),
        );
        if apply.show(ui, ar, "tak-callsign-apply") {
            p.act("settings", "/api/tak/settings", json!({"callsign": text.trim()}), h);
            p.form.callsign = None;
        }
    }
    bottom = iy + formfield::control_h() + GAP;
    let half = (w - ROW_GAP) / 2.0;
    let teams: Vec<Choice> = TEAMS.iter().map(|t| Choice::new(*t, *t)).collect();
    let roles: Vec<Choice> = ROLES
        .iter()
        .map(|r| Choice::new(*r, translate(r).into_owned()))
        .collect();
    let (mut team, mut role) = (None, None);
    let cap = formfield::caption(ui, x, bottom, "Team", "", "tak-team-cap");
    formfield::caption(ui, x + half + ROW_GAP, bottom, "Role", "", "tak-role-cap");
    let iy = bottom + cap + formfield::GAP;
    team = team.or(formfield::select(
        ui,
        x,
        iy,
        half,
        &teams,
        st["team"].as_str().unwrap_or("Cyan"),
        "tak-team",
    ));
    role = role.or(formfield::select(
        ui,
        x + half + ROW_GAP,
        iy,
        half,
        &roles,
        st["role"].as_str().unwrap_or("Team Member"),
        "tak-role",
    ));
    if let Some(t) = team {
        p.act("settings", "/api/tak/settings", json!({"team": t}), h);
    }
    if let Some(r) = role {
        p.act("settings", "/api/tak/settings", json!({"role": r}), h);
    }
    bottom = iy + formfield::control_h() + GAP;
    let label = line(
        ui,
        x,
        bottom + 1.0,
        w - 50.0,
        "Share the aircraft's position",
        12.0,
        Weight::W600,
        style::text_1(),
    );
    let hint = note(
        ui,
        x,
        bottom + label + 3.0,
        w - 50.0,
        "Sends the live position as a friendly drone to everybody on the connections that are switched on and allowed to send.",
        style::text_3(),
    );
    if kit::switch(ui, Pos2::new(x + w - 38.0, bottom + 2.0), share, true, "tak-share") {
        p.act("settings", "/api/tak/settings", json!({"share_position": !share}), h);
    }
    bottom + label + 3.0 + hint
}

// ---- what the map shows -------------------------------------------------------

fn class_label(class: &str) -> &'static str {
    match class {
        "unit" => "People and vehicles",
        "point" => "Markers",
        "route" => "Routes",
        "drawing" => "Drawings",
        _ => "Alerts",
    }
}

fn affiliation_label(a: &str) -> &'static str {
    match a {
        "friend" => "Friend",
        "hostile" => "Hostile",
        "neutral" => "Neutral",
        _ => "Unknown",
    }
}

fn map_filter(p: &mut Tak, ui: &mut egui::Ui, x: f32, y: f32, w: f32, h: &mut Host) -> f32 {
    let (mut bottom, frame) = head(ui, x, y, w, "On the map", |ui, right| {
        tool(ui, right, y, 0, "maximize", "Show all contacts on the map")
    });
    bottom += GAP;
    if frame && p.fit(None, h) == 0 {
        h.notify("info", &translate("No contact is shown."));
    }
    let half = w / 2.0;
    let mut changed = false;
    for (i, a) in AFFILIATIONS.iter().enumerate() {
        let mut on = p.filter.affiliations.iter().any(|n| n == a);
        let at = Pos2::new(x + (i % 2) as f32 * half, bottom + (i / 2) as f32 * 22.0);
        if kit::checkbox(ui, at, half - 6.0, affiliation_label(a), &mut on, false, ("tak-aff", i)) {
            Filter::toggle(&mut p.filter.affiliations, a, &AFFILIATIONS);
            changed = true;
        }
    }
    bottom += 2.0 * 22.0 + 4.0;
    for (i, c) in CLASSES.iter().enumerate() {
        let mut on = p.filter.classes.iter().any(|n| n == c);
        let at = Pos2::new(x + (i % 2) as f32 * half, bottom + (i / 2) as f32 * 22.0);
        if kit::checkbox(ui, at, half - 6.0, class_label(c), &mut on, false, ("tak-class", i)) {
            Filter::toggle(&mut p.filter.classes, c, &CLASSES);
            changed = true;
        }
    }
    bottom += CLASSES.len().div_ceil(2) as f32 * 22.0;
    if changed {
        p.save_filter(h);
    }
    bottom
}

// ---- contacts -----------------------------------------------------------------

fn contacts(p: &mut Tak, ui: &mut egui::Ui, x: f32, y: f32, w: f32, h: &mut Host) -> f32 {
    let (mut bottom, ()) = head(ui, x, y, w, "Contacts", |_, _| ());
    bottom += ROW_GAP;
    let stale = p.opts.stale;
    let list: Vec<Contact> = p
        .mirror
        .sorted(p.aircraft)
        .into_iter()
        .filter(|c| p.filter.shows(c, stale))
        .cloned()
        .collect();
    if list.is_empty() {
        let text = if p.mirror.connections_on() == 0 {
            "Nothing is connected. Switch a connection on to hear ATAK, CivTAK and WinTAK."
        } else {
            "No contact yet. They show up as soon as somebody on the network reports."
        };
        return bottom + note(ui, x + 2.0, bottom + 2.0, w - 4.0, text, style::text_3()) + 4.0;
    }
    let mut pick: Option<String> = None;
    for c in list.iter().take(MAX_ROWS) {
        let r = Rect::from_min_size(Pos2::new(x, bottom), Vec2::new(w, 38.0));
        let selected = p.selected.as_deref() == Some(c.uid.as_str());
        let resp = ui.interact(r, ui.scope_id().with(("tak-contact", &c.uid)), egui::Sense::click());
        listrow::paint(ui.painter(), r, style::RADIUS_BTN, Mark::of(selected, resp.hovered()));
        badge(
            ui,
            Pos2::new(r.left() + 6.0, r.center().y - 13.0),
            c.icon(),
            hex(&c.color_hex()).unwrap_or(Color32::WHITE),
            c.stale,
        );
        let room = w - 44.0 - 8.0;
        line(
            ui,
            r.left() + 40.0,
            r.top() + 5.0,
            room,
            &c.callsign,
            12.0,
            Weight::W600,
            style::text_1(),
        );
        let mut sub = Vec::new();
        if let Some(a) = p.aircraft {
            sub.push(distance_text(distance_m(a, (c.lat, c.lon))));
        }
        sub.push(if c.stale {
            translate("stale").into_owned()
        } else {
            age_text(c.age_s)
        });
        if let Some(b) = c.battery {
            sub.push(format!("{b:.0} %"));
        }
        line(
            ui,
            r.left() + 40.0,
            r.top() + 21.0,
            room,
            &sub.join(" \u{b7} "),
            10.5,
            Weight::W400,
            style::text_3(),
        );
        if resp.clicked() {
            pick = Some(c.uid.clone());
        }
        bottom = r.bottom() + 2.0;
    }
    if list.len() > MAX_ROWS {
        let more = translate_fmt("{n} more on the map", &[("n", &(list.len() - MAX_ROWS).to_string())]);
        bottom += line(
            ui,
            x + 4.0,
            bottom + 2.0,
            w - 8.0,
            &more,
            10.5,
            Weight::W400,
            style::text_3(),
        ) + 6.0;
    }
    if let Some(uid) = pick {
        if p.selected.as_deref() == Some(uid.as_str()) {
            p.select(None);
        } else {
            p.fit(Some(std::slice::from_ref(&uid)), h);
            p.select(Some(uid));
        }
    }
    if let Some(c) = p.selected.as_ref().and_then(|u| p.mirror.contacts.get(u)).cloned() {
        bottom = detail(p, ui, x, bottom + GAP, w, &c, h);
    }
    bottom
}

fn detail(p: &mut Tak, ui: &mut egui::Ui, x: f32, y: f32, w: f32, c: &Contact, h: &mut Host) -> f32 {
    let mut rows: Vec<(&str, String)> = vec![
        ("Type", format!("{} ({})", c.kind, c.class)),
        ("Position", format!("{:.6}, {:.6}", c.lat, c.lon)),
    ];
    if let Some(a) = c.hae {
        rows.push(("Height", format!("{a:.0} m")));
    }
    if let Some(s) = c.speed {
        rows.push((
            "Movement",
            format!("{:.1} m/s, {:.0}\u{b0}", s, c.course.unwrap_or(0.0)),
        ));
    }
    if let Some(t) = &c.team {
        rows.push(("Team", t.clone()));
    }
    if !c.remarks.is_empty() {
        rows.push(("Remarks", c.remarks.clone()));
    }
    rows.push(("Heard on", c.source.clone()));
    rows.push(("Uid", c.uid.clone()));
    let slot = card::reserve(ui.painter());
    let mut cy = y + 8.0;
    let top = y;
    let label_w = 76.0;
    for (label, value) in &rows {
        let g = kit::wrapped_weight(ui, value, 11.5, Weight::W400, style::text_1(), w - label_w - 20.0, 1.3);
        let hgt = g.size().y.max(14.0);
        line(
            ui,
            x + 10.0,
            cy,
            label_w,
            &label.to_uppercase(),
            9.5,
            Weight::W400,
            style::text_3(),
        );
        ui.painter()
            .galley(Pos2::new(x + 10.0 + label_w, cy), g, style::text_1());
        cy += hgt + 4.0;
    }
    let hand = (h.present(ENTITIES_PLUGIN) && !c.uid.starts_with(ENTITY_UID_PREFIX)).then(|| {
        let done = p.handed.contains(&c.uid);
        Button::new(
            if done { "In Map Entities" } else { "Add to Map Entities" },
            Variant::Secondary,
        )
        .small()
        .icon("map-pin")
        .enabled(!done)
        .title(
            "Places this contact as a point in the Map Entities plugin. It is a copy: it does not follow the contact.",
        )
    });
    let frame = Button::new("Show on the map", Variant::Secondary)
        .small()
        .icon("locate-fixed");
    let delete = Button::new("Delete for everyone", Variant::DangerOutline).small();
    let (fw, dw, bh) = (frame.width(ui), delete.width(ui), frame.height());
    cy += 2.0;
    if frame.show(
        ui,
        Rect::from_min_size(Pos2::new(x + 10.0, cy), Vec2::new(fw, bh)),
        "tak-detail-frame",
    ) {
        h.fit(vec![[c.lon, c.lat]]);
    }
    let delete_at = Rect::from_min_size(Pos2::new(x + 10.0 + fw + ROW_GAP, cy), Vec2::new(dw, bh));
    if delete
        .title("Tells every TAK client on the connections that may send to remove it")
        .show(ui, delete_at, "tak-detail-delete")
    {
        p.act("delete", "/api/tak/delete", json!({"uid": c.uid}), h);
        p.select(None);
    }
    let mut end = cy + bh + 8.0;
    if let Some(hand) = hand {
        let hw = hand.width(ui);
        let at = Rect::from_min_size(Pos2::new(x + 10.0, cy + bh + ROW_GAP), Vec2::new(hw, bh));
        if hand.show(ui, at, "tak-detail-hand") {
            p.hand_to_entities(c, h);
        }
        end += bh + ROW_GAP;
    }
    let r = Rect::from_min_size(Pos2::new(x, top), Vec2::new(w, end - top));
    card::fill(
        ui.painter(),
        slot,
        r,
        card::Look::custom(style::surface_2(), style::border_soft(), style::RADIUS_BTN),
    );
    end
}

// ---- a marker -----------------------------------------------------------------

fn marker_form(p: &mut Tak, ui: &mut egui::Ui, x: f32, y: f32, w: f32, h: &mut Host) -> f32 {
    let (mut bottom, ()) = head(ui, x, y, w, "Place a marker", |_, _| ());
    bottom += GAP;
    let kinds = [
        Choice::new("b-m-p-s-p-i", translate("Spot marker").into_owned()),
        Choice::new("a-f-G-U-C", translate("Friendly ground unit").into_owned()),
        Choice::new("a-h-G", translate("Hostile ground").into_owned()),
        Choice::new("a-n-G", translate("Neutral ground").into_owned()),
        Choice::new("a-u-G", translate("Unknown ground").into_owned()),
    ];
    let mut picked = None;
    let current = p.form.marker_type.clone();
    bottom = formfield::field(ui, x, bottom, w, "Type", "", |ui, cy| {
        picked = formfield::select(ui, x, cy, w, &kinds, &current, "tak-marker-type");
        formfield::control_h()
    }) + GAP;
    if let Some(k) = picked {
        p.form.marker_type = k;
    }
    let name = &mut p.form.marker_name;
    bottom = formfield::field(ui, x, bottom, w, "Name", "", |ui, cy| {
        formfield::input(ui, x, cy, w, name, "Rally point", false, false, "tak-marker-name");
        formfield::control_h()
    }) + GAP;
    let remarks = &mut p.form.marker_remarks;
    bottom = formfield::field(ui, x, bottom, w, "Remarks", "", |ui, cy| {
        formfield::input(ui, x, cy, w, remarks, "", false, false, "tak-marker-remarks");
        formfield::control_h()
    }) + GAP;
    let placing = p.form.placing;
    let button = if placing {
        Button::new("Cancel", Variant::Secondary).small()
    } else {
        Button::new("Place on the map", Variant::Primary)
            .small()
            .icon("map-pin")
    };
    let (bw, bh) = (button.width(ui), button.height());
    if button.show(
        ui,
        Rect::from_min_size(Pos2::new(x, bottom), Vec2::new(bw, bh)),
        "tak-marker-place",
    ) {
        if placing {
            p.stop_placing(h);
        } else {
            p.start_placing(h);
        }
    }
    bottom += bh;
    if placing {
        bottom += 6.0
            + note(
                ui,
                x,
                bottom + 6.0,
                w,
                "Click the map to place the marker. Escape or a right click cancels.",
                style::text_2(),
            );
    }
    bottom
}

// ---- chat ---------------------------------------------------------------------

fn chat(p: &mut Tak, ui: &mut egui::Ui, x: f32, y: f32, w: f32, h: &mut Host) -> f32 {
    let (mut bottom, ()) = head(ui, x, y, w, "Chat", |_, _| ());
    bottom += ROW_GAP;
    let lines: Vec<_> = p.mirror.chat.iter().rev().take(MAX_CHAT_SHOWN).rev().cloned().collect();
    if lines.is_empty() {
        bottom += note(ui, x + 2.0, bottom + 2.0, w - 4.0, "No chat yet.", style::text_3()) + 6.0;
    }
    for m in &lines {
        let g = kit::wrapped_weight(
            ui,
            &format!("{}: {}", m.from, m.text),
            11.5,
            Weight::W400,
            style::text_1(),
            w - 8.0,
            1.35,
        );
        let gh = g.size().y;
        ui.painter()
            .galley(Pos2::new(x + 4.0, bottom + 2.0), g, style::text_1());
        bottom += gh + 5.0;
    }
    let send = Button::new("Send", Variant::Secondary).small().icon("send");
    let sw = send.width(ui);
    let field_w = w - sw - ROW_GAP;
    let resp = formfield::input(
        ui,
        x,
        bottom,
        field_w,
        &mut p.form.chat,
        "Message to everybody",
        false,
        false,
        "tak-chat",
    );
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let sr = Rect::from_min_size(
        Pos2::new(
            x + field_w + ROW_GAP,
            bottom + (formfield::control_h() - send.height()) / 2.0,
        ),
        Vec2::new(sw, send.height()),
    );
    let clicked = send.show(ui, sr, "tak-chat-send");
    if (clicked || enter) && !p.form.chat.trim().is_empty() {
        let text = p.form.chat.trim().to_string();
        p.act("chat", "/api/tak/chat", json!({"text": text}), h);
        p.form.chat.clear();
    }
    bottom + formfield::control_h()
}
