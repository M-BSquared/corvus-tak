//! TAK: the contacts and markers of ATAK, CivTAK, WinTAK and a TAK server on the
//! Home map, and the aircraft on theirs.
//!
//! The network side is the core's `tak` module (Cursor on Target over UDP
//! multicast, UDP, TCP and TLS, with the routes and the MCP tools under
//! `/api/tak`). This plugin follows its `tak` topic, draws what it hears as
//! markers (the TAK colours for friend, hostile, neutral and unknown), lists the
//! contacts, and lets the operator place a marker, write chat and set up the
//! connections. An AI client can also steer the map layer through the tools in
//! [`mcp`], when the operator allows it in the plugin's gear.

mod mcp;
pub mod model;
mod view;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use self::model::{
    markers_for_entities, plan_entities, Applied, Filter, Mirror, ENTITIES_PLUGIN, ENTITIES_TOPIC, ENTITY_RESEND_S,
    ENTITY_UID_PREFIX,
};
// Under other names: the key check of the program reads the language files of the
// application for every literal handed to `translate(`, and a plugin keeps its own.
use corvus_api::i18n::{tr as translate, tr_fmt as translate_fmt};
use corvus_api::net::stream::Stream;
use corvus_api::plugin::options::OptionSpec;
use corvus_api::plugin::shapes::{hex, Style};
use corvus_api::plugin::{Host, Info, MapEvent, McpTool, Plugin, Reply};

pub const MCP_OPTION: &str = "ai";
const KEY_PREFIX: &str = "c:";

/// The switches of the plugin's gear.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Opts {
    pub labels: bool,
    pub stale: bool,
    pub entities: bool,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            labels: true,
            stale: false,
            entities: false,
        }
    }
}

/// A connection being added or changed.
#[derive(Debug, Clone, Default)]
pub struct Editor {
    /// The id of the connection being changed; `None` for a new one.
    pub id: Option<String>,
    pub name: String,
    pub kind: String,
    pub host: String,
    pub port: String,
    pub send: bool,
    pub ca_file: String,
    pub cert_file: String,
    pub key_file: String,
    pub error: String,
}

/// What the operator has typed and not sent yet.
#[derive(Debug, Clone)]
pub struct Form {
    pub marker_type: String,
    pub marker_name: String,
    pub marker_remarks: String,
    pub placing: bool,
    pub chat: String,
    /// The callsign being typed; `None` while the saved one is shown.
    pub callsign: Option<String>,
}

impl Default for Form {
    fn default() -> Form {
        Form {
            marker_type: "b-m-p-s-p-i".into(),
            marker_name: String::new(),
            marker_remarks: String::new(),
            placing: false,
            chat: String::new(),
            callsign: None,
        }
    }
}

enum Job {
    Contacts(Reply),
    Status(Reply),
    /// A change the operator or an AI client asked for: what it was, and the answer.
    Act(&'static str, Reply),
    /// A Map Entities marker sent or taken back: what it was, the uid, the body sent (as text) and the answer.
    Share(ShareOp, String, String, Reply),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ShareOp {
    Send,
    Delete,
}

/// What the plugin did for Map Entities: the markers on the network, by uid.
#[derive(Default)]
struct Share {
    /// The markers the network was told of, with the body sent.
    pushed: HashMap<String, String>,
    /// What the network should show.
    wanted: BTreeMap<String, Value>,
    /// Whether sharing was on, and the revision of the entities, at the last look.
    seen: (bool, u64),
    /// Connections up at the last look; a new one gets everything again.
    links: usize,
    queue: VecDeque<(ShareOp, String)>,
    dirty: bool,
    force: bool,
    resend_at: f64,
    retry_at: f64,
    failing: bool,
}

const SHARE_IN_FLIGHT: usize = 6;
const SHARE_RETRY_S: f64 = 15.0;

#[derive(Default)]
pub struct Tak {
    pub mirror: Mirror,
    pub filter: Filter,
    pub opts: Opts,
    pub selected: Option<String>,
    pub form: Form,
    pub editor: Option<Editor>,
    pub aircraft: Option<(f64, f64)>,
    stream: Option<Stream>,
    inbox: Arc<Mutex<Vec<Value>>>,
    jobs: Vec<Job>,
    share: Share,
    /// Contacts handed to Map Entities this session.
    pub handed: HashSet<String>,
    drawn: HashSet<String>,
    dirty: bool,
    started: bool,
    pub open: bool,
    mcp: mcp::State,
}

impl Plugin for Tak {
    fn info(&self) -> Info {
        Info {
            id: "tak",
            name: "TAK",
            icon: "radio-tower",
            description: "ATAK, CivTAK and WinTAK contacts and markers on the Home map, and the aircraft on theirs",
            version: "2.0.0",
            order: 36,
            can_tab: true,
        }
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::toggle("labels", "Show callsigns on the map", true),
            OptionSpec::toggle("stale", "Show contacts that stopped reporting", false),
            OptionSpec::toggle("entities", "Send Map Entities to TAK", false).hint(
                "Sends the gates, trees and symbols of the Map Entities plugin to the connected TAK clients as markers, and takes them back when you delete them or switch this off. It needs Share with other plugins in the gear of Map Entities. Without Map Entities nothing is sent.",
            ),
            OptionSpec::toggle(MCP_OPTION, "Let an AI client steer the map layer", false)
                .hint("Adds tools to the MCP server that focus a contact and choose what the map shows. It also needs Plugin tools switched on in Settings, Connections, MCP."),
        ]
    }

    fn start(&mut self, h: &mut Host) {
        if self.started {
            return;
        }
        self.started = true;
        self.filter = Filter::from_settings(&h.settings());
        self.read_options(h);
        let inbox = Arc::clone(&self.inbox);
        self.stream = Some(h.events("tak", move |name, data| {
            if name != "error" && data.is_object() {
                if let Ok(mut q) = inbox.lock() {
                    if q.len() < 512 {
                        q.push(data.clone());
                    }
                }
            }
        }));
    }

    fn tick(&mut self, h: &mut Host) {
        self.aircraft = aircraft_of(h.state);
        let events: Vec<Value> = self
            .inbox
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default();
        for e in &events {
            if self.mirror.apply(e) == Applied::Gap {
                self.jobs.push(Job::Contacts(h.get("/api/tak/tracks?limit=1000")));
                self.jobs.push(Job::Status(h.get("/api/tak")));
            }
            self.dirty = true;
        }
        self.poll_jobs(h);
        self.mcp_run(h);
        self.sync_entities(h);
        if self.dirty {
            self.dirty = false;
            self.draw(h);
        }
    }

    fn open(&mut self, _h: &mut Host) {
        self.open = true;
    }

    fn close(&mut self, h: &mut Host) {
        self.open = false;
        self.editor = None;
        self.stop_placing(h);
    }

    fn options_changed(&mut self, h: &mut Host) {
        self.read_options(h);
        self.share.dirty = true;
        self.dirty = true;
    }

    fn map_event(&mut self, e: MapEvent, h: &mut Host) {
        match e {
            MapEvent::Click(key) => {
                if let Some(uid) = key.strip_prefix(KEY_PREFIX) {
                    self.select(Some(uid.to_string()));
                }
            }
            MapEvent::Picked(at) => self.place(at, h),
            MapEvent::PickCancelled => self.form.placing = false,
            MapEvent::Drag(..) | MapEvent::DragEnd(..) => {}
        }
    }

    fn show(&mut self, ui: &mut corvus_api::egui::Ui, area: corvus_api::egui::Rect, h: &mut Host) -> f32 {
        view::show(self, ui, area, h)
    }

    fn mcp_tools(&self, h: &Host) -> Vec<McpTool> {
        mcp::tools_for(h)
    }

    fn mcp_call(&mut self, name: &str, args: &Value, token: u64, h: &mut Host) {
        self.mcp_start(name, args, token, h);
    }

    fn mcp_take(&mut self, token: u64) -> Option<Result<Value, String>> {
        self.mcp_result(token)
    }
}

/// `[lat, lon]` of the aircraft, when it is connected and has a fix.
fn aircraft_of(state: &Value) -> Option<(f64, f64)> {
    if state.get("connected") != Some(&Value::Bool(true)) {
        return None;
    }
    let pos = state.get("position")?.as_array()?;
    let (lon, lat) = (pos.first()?.as_f64()?, pos.get(1)?.as_f64()?);
    (lat.is_finite() && lon.is_finite() && !(lat == 0.0 && lon == 0.0)).then_some((lat, lon))
}

impl Tak {
    fn read_options(&mut self, h: &Host) {
        let on = |k: &str, d: bool| h.options().get(k).and_then(Value::as_bool).unwrap_or(d);
        self.opts = Opts {
            labels: on("labels", true),
            stale: on("stale", false),
            entities: on("entities", false),
        };
    }

    pub fn select(&mut self, uid: Option<String>) {
        self.selected = uid;
        self.dirty = true;
    }

    pub fn redraw(&mut self) {
        self.dirty = true;
    }

    /// Saves what the map shows.
    pub fn save_filter(&mut self, h: &mut Host) {
        let _ = h.save_settings(&json!({"filter": self.filter.to_value()}), false);
        self.dirty = true;
    }

    /// Asks the core for a change; the answer says what to tell the operator.
    pub fn act(&mut self, what: &'static str, path: &str, body: Value, h: &mut Host) {
        self.jobs.push(Job::Act(what, h.post(path, body)));
    }

    fn poll_jobs(&mut self, h: &mut Host) {
        let mut keep = Vec::new();
        for job in std::mem::take(&mut self.jobs) {
            match job {
                Job::Contacts(r) => match r.take() {
                    None => keep.push(Job::Contacts(r)),
                    Some(Ok(v)) => {
                        self.mirror.replace(v.get("contacts").unwrap_or(&Value::Null));
                        self.dirty = true;
                    }
                    Some(Err(_)) => {}
                },
                Job::Status(r) => match r.take() {
                    None => keep.push(Job::Status(r)),
                    Some(Ok(v)) => {
                        if v.is_object() {
                            self.mirror.status = v;
                        }
                    }
                    Some(Err(_)) => {}
                },
                Job::Share(op, uid, body, r) => match r.take() {
                    None => keep.push(Job::Share(op, uid, body, r)),
                    Some(Ok(_)) => {
                        match op {
                            ShareOp::Send => self.share.pushed.insert(uid, body),
                            ShareOp::Delete => self.share.pushed.remove(&uid),
                        };
                        self.share.failing = false;
                    }
                    Some(Err(why)) => self.share_failed(&why, h),
                },
                Job::Act(what, r) => match r.take() {
                    None => keep.push(Job::Act(what, r)),
                    Some(Ok(v)) => {
                        if let Some(s) = v.get("status").filter(|s| s.is_object()) {
                            self.mirror.status = s.clone();
                        }
                        if what == "marker" {
                            let n = v.get("sent_to").and_then(Value::as_u64).unwrap_or(0);
                            h.notify(
                                "info",
                                &translate_fmt("Marker sent to {n} connection(s).", &[("n", &n.to_string())]),
                            );
                        }
                    }
                    Some(Err(e)) => h.notify("warning", &e),
                },
            }
        }
        self.jobs = keep;
    }

    // ---- working with Map Entities ----------------------------------------------

    /// The network could not take a marker: try again in a while, and say so once.
    fn share_failed(&mut self, why: &str, h: &mut Host) {
        self.share.queue.clear();
        self.share.dirty = true;
        self.share.retry_at = h.now + SHARE_RETRY_S;
        if !self.share.failing {
            self.share.failing = true;
            h.notify(
                "warning",
                &translate_fmt(
                    "Map Entities could not be sent to TAK: {why} Trying again.",
                    &[("why", why)],
                ),
            );
        }
    }

    /// Keeps the markers on the network in step with what Map Entities shares:
    /// only while the option is on and that plugin runs, and everything it sent is
    /// taken back when either stops. Nothing here is needed for the rest of the plugin.
    fn sync_entities(&mut self, h: &mut Host) {
        let on = self.opts.entities && h.present(ENTITIES_PLUGIN);
        let rev = h.revision(ENTITIES_TOPIC);
        let links = self.mirror.connections_on();
        if (on, rev) != self.share.seen {
            self.share.seen = (on, rev);
            self.share.wanted = if on {
                h.read(ENTITIES_TOPIC).map(markers_for_entities).unwrap_or_default()
            } else {
                BTreeMap::new()
            };
            self.share.dirty = true;
        }
        if links > self.share.links {
            self.share.force = true;
        }
        self.share.links = links;
        if on && h.now >= self.share.resend_at {
            self.share.resend_at = h.now + ENTITY_RESEND_S;
            self.share.force = true;
        }
        if !on && self.share.pushed.is_empty() && self.share.queue.is_empty() {
            self.share.force = false;
            self.share.dirty = false;
            return;
        }
        if links == 0 {
            return;
        }
        if (self.share.dirty || self.share.force) && h.now >= self.share.retry_at && self.share.queue.is_empty() {
            let (send, gone) = plan_entities(&self.share.wanted, &self.share.pushed, self.share.force);
            self.share.dirty = false;
            self.share.force = false;
            self.share.queue.extend(send.into_iter().map(|u| (ShareOp::Send, u)));
            self.share.queue.extend(gone.into_iter().map(|u| (ShareOp::Delete, u)));
        }
        let in_flight = self.jobs.iter().filter(|j| matches!(j, Job::Share(..))).count();
        for _ in in_flight..SHARE_IN_FLIGHT {
            let Some((op, uid)) = self.share.queue.pop_front() else {
                break;
            };
            match op {
                ShareOp::Send => {
                    let Some(body) = self.share.wanted.get(&uid).cloned() else {
                        continue;
                    };
                    let text = body.to_string();
                    let reply = h.post("/api/tak/marker", body);
                    self.jobs.push(Job::Share(op, uid, text, reply));
                }
                ShareOp::Delete => {
                    let reply = h.post("/api/tak/delete", json!({"uid": uid}));
                    self.jobs.push(Job::Share(op, uid, String::new(), reply));
                }
            }
        }
        if !self.share.queue.is_empty() {
            h.ctx.request_repaint();
        }
    }

    /// Hands a contact to Map Entities, which places it as a point of its own.
    /// False when that plugin is not running.
    pub fn hand_to_entities(&mut self, c: &model::Contact, h: &mut Host) -> bool {
        let sent = h.send(ENTITIES_PLUGIN, model::entity_for_contact(c));
        if sent {
            self.handed.insert(c.uid.clone());
        }
        sent
    }

    pub fn busy(&self) -> bool {
        !self.jobs.is_empty()
    }

    // ---- the map ------------------------------------------------------------

    fn draw(&mut self, h: &mut Host) {
        let mut wanted = HashSet::new();
        for c in self.mirror.contacts.values() {
            if !self.filter.shows(c, self.opts.stale) {
                continue;
            }
            if c.uid.starts_with(ENTITY_UID_PREFIX) && h.present(ENTITIES_PLUGIN) {
                continue;
            }
            let key = format!("{KEY_PREFIX}{}", c.uid);
            let mut style = Style::color(hex(&c.color_hex()).unwrap_or(corvus_api::egui::Color32::WHITE));
            if c.stale {
                style = style.opacity(0.45);
            }
            let label = if self.opts.labels { c.callsign.as_str() } else { "" };
            let title = format!("{} ({})", c.callsign, c.kind);
            let selected = self.selected.as_deref() == Some(c.uid.as_str());
            h.map().marker(
                &key,
                [c.lon, c.lat],
                c.icon(),
                label,
                &title,
                style,
                Some(26.0),
                selected,
                false,
                true,
            );
            wanted.insert(key);
        }
        for gone in self.drawn.difference(&wanted) {
            h.map().remove(gone);
        }
        self.drawn = wanted;
    }

    /// Frames these contacts, or all that are drawn.
    pub fn fit(&self, uids: Option<&[String]>, h: &mut Host) -> usize {
        let pts: Vec<[f64; 2]> = self
            .mirror
            .contacts
            .values()
            .filter(|c| match uids {
                Some(list) => list.contains(&c.uid),
                None => self.filter.shows(c, self.opts.stale),
            })
            .map(|c| [c.lon, c.lat])
            .collect();
        let n = pts.len();
        if n > 0 {
            h.fit(pts);
        }
        n
    }

    // ---- placing a marker -----------------------------------------------------

    pub fn start_placing(&mut self, h: &mut Host) {
        self.form.placing = true;
        h.pick_point();
        h.notify(
            "info",
            &translate("Click the map to place the marker. Escape or a right click cancels."),
        );
    }

    pub fn stop_placing(&mut self, h: &mut Host) {
        if self.form.placing {
            self.form.placing = false;
            h.cancel_pick();
        }
    }

    fn place(&mut self, at: [f64; 2], h: &mut Host) {
        self.form.placing = false;
        let name = self.form.marker_name.trim();
        let body = json!({
            "lat": at[1], "lon": at[0],
            "type": self.form.marker_type,
            "callsign": if name.is_empty() { "Marker" } else { name },
            "remarks": self.form.marker_remarks.trim(),
        });
        self.act("marker", "/api/tak/marker", body, h);
    }
}

/// The language files of this plugin, for the translation layer.
pub const LANG: &[(&str, &str)] = &[
    ("de", include_str!("../lang/de.json")),
    ("fr", include_str!("../lang/fr.json")),
    ("dl", include_str!("../lang/dl.json")),
];

/// One instance of the plugin, for the registry.
pub fn plugin() -> Box<dyn Plugin> {
    Box::new(Tak::default())
}

corvus_api::export_plugin!(plugin, LANG);

#[cfg(test)]
mod tests {
    use super::model::{AFFILIATIONS, CLASSES};
    use super::*;

    #[test]
    fn the_manifest_says_what_the_plugin_says_of_itself() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!("../plugin.json")).expect("plugin.json");
        let i = Tak::default().info();
        for (key, want) in [
            ("id", i.id),
            ("name", i.name),
            ("description", i.description),
            ("icon", i.icon),
            ("version", i.version),
        ] {
            assert_eq!(
                manifest[key], want,
                "plugin.json says something else than the plugin for {key}"
            );
        }
        assert_eq!(manifest["order"], i.order);
        assert_eq!(manifest["can_tab"], i.can_tab);
    }

    #[test]
    fn the_aircraft_needs_a_link_and_a_fix() {
        assert_eq!(
            aircraft_of(&json!({"connected": true, "position": [11.6, 48.0]})),
            Some((48.0, 11.6))
        );
        assert_eq!(
            aircraft_of(&json!({"connected": false, "position": [11.6, 48.0]})),
            None
        );
        assert_eq!(aircraft_of(&json!({"connected": true, "position": [0.0, 0.0]})), None);
        assert_eq!(aircraft_of(&json!({"connected": true})), None);
    }

    #[test]
    fn the_filter_names_are_the_cores() {
        assert_eq!(AFFILIATIONS, ["friend", "hostile", "neutral", "unknown"]);
        assert_eq!(CLASSES.len(), 5);
    }

    #[test]
    fn a_new_plugin_shows_callsigns_and_hides_stale_contacts() {
        assert_eq!(Tak::default().opts, Opts::default());
        assert!(Opts::default().labels && !Opts::default().stale);
    }
}
