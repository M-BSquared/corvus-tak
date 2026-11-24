//! What the plugin knows of the TAK network: the contacts and chat the core
//! pushes, a mirror kept in step by the core's `tak` topic, and what the map
//! shows of it. Nothing here draws or calls; it is plain data and tests.

use std::collections::{HashMap, VecDeque};

use serde_json::{json, Value};

pub const AFFILIATIONS: [&str; 4] = ["friend", "hostile", "neutral", "unknown"];
pub const CLASSES: [&str; 5] = ["unit", "point", "route", "drawing", "alert"];
pub const TEAMS: [&str; 14] = [
    "White",
    "Yellow",
    "Orange",
    "Magenta",
    "Red",
    "Maroon",
    "Purple",
    "Dark Blue",
    "Blue",
    "Cyan",
    "Teal",
    "Green",
    "Dark Green",
    "Brown",
];
pub const ROLES: [&str; 8] = [
    "Team Member",
    "Team Lead",
    "HQ",
    "Sniper",
    "Medic",
    "Forward Observer",
    "RTO",
    "K9",
];
pub const MAX_CHAT: usize = 100;
pub const MAX_CONTACTS: usize = 1000;

#[derive(Debug, Clone, PartialEq)]
pub struct Contact {
    pub uid: String,
    pub callsign: String,
    pub kind: String,
    pub class: String,
    pub affiliation: String,
    pub dimension: String,
    pub lat: f64,
    pub lon: f64,
    pub hae: Option<f64>,
    pub course: Option<f64>,
    pub speed: Option<f64>,
    pub battery: Option<f64>,
    pub team: Option<String>,
    pub color: Option<String>,
    pub remarks: String,
    pub stale: bool,
    pub source: String,
    pub age_s: f64,
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64).filter(|n| n.is_finite())
}

impl Contact {
    /// A contact from the core's JSON; `None` when it has no uid or place.
    pub fn from_json(v: &Value) -> Option<Contact> {
        let uid = text(v, "uid");
        let (lat, lon) = (num(v, "lat")?, num(v, "lon")?);
        if uid.is_empty() || !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return None;
        }
        let callsign = match text(v, "callsign") {
            c if c.is_empty() => uid.clone(),
            c => c,
        };
        Some(Contact {
            callsign,
            kind: text(v, "type"),
            class: text(v, "class"),
            affiliation: text(v, "affiliation"),
            dimension: text(v, "dimension"),
            hae: num(v, "hae"),
            course: num(v, "course"),
            speed: num(v, "speed"),
            battery: num(v, "battery"),
            team: v.get("team").and_then(Value::as_str).map(str::to_string),
            color: v.get("color").and_then(Value::as_str).map(str::to_string),
            remarks: text(v, "remarks"),
            stale: v.get("stale").and_then(Value::as_bool).unwrap_or(false),
            source: text(v, "source"),
            age_s: num(v, "age_s").unwrap_or(0.0),
            uid,
            lat,
            lon,
        })
    }

    /// The colour the map gives it, `#RRGGBB`: the TAK convention for the
    /// four affiliations, the contact's own colour for a marker that has one.
    pub fn color_hex(&self) -> String {
        if self.class == "unit" {
            return match self.affiliation.as_str() {
                "friend" => "#4DA3FF",
                "hostile" => "#FF5A5A",
                "neutral" => "#5AD27A",
                _ => "#F2D04D",
            }
            .to_string();
        }
        self.color.clone().unwrap_or_else(|| "#E6EDF3".to_string())
    }

    /// The symbol of `corvus_api::plugin::shapes::ICONS` for it.
    pub fn icon(&self) -> &'static str {
        match self.class.as_str() {
            "point" => "pin",
            "alert" => "hazard",
            "route" | "drawing" => "flag",
            _ => match self.dimension.as_str() {
                "air" if self.kind.contains("-Q") => "drone",
                "air" => "plane",
                "sea" | "subsurface" => "boat",
                "space" => "target",
                _ if self.kind.contains("-E-V") || self.kind.contains("-E-S") => "vehicle",
                _ => "person",
            },
        }
    }
}

/// Great circle distance in metres between `[lat, lon]` pairs.
pub fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, la2) = (a.0.to_radians(), b.0.to_radians());
    let (dlat, dlon) = ((b.0 - a.0).to_radians(), (b.1 - a.1).to_radians());
    let h = (dlat / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlon / 2.0).sin().powi(2);
    6_371_000.0 * 2.0 * h.sqrt().min(1.0).asin()
}

/// "850 m" or "2.4 km".
pub fn distance_text(m: f64) -> String {
    if m < 1000.0 {
        format!("{} m", m.round() as i64)
    } else {
        format!("{:.1} km", m / 1000.0)
    }
}

/// "12 s", "4 min", "3 h".
pub fn age_text(s: f64) -> String {
    let s = s.max(0.0);
    if s < 90.0 {
        format!("{} s", s.round() as i64)
    } else if s < 5400.0 {
        format!("{} min", (s / 60.0).round() as i64)
    } else {
        format!("{} h", (s / 3600.0).round() as i64)
    }
}

/// What the map shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    pub affiliations: Vec<String>,
    pub classes: Vec<String>,
}

impl Default for Filter {
    fn default() -> Filter {
        Filter {
            affiliations: AFFILIATIONS.iter().map(|s| s.to_string()).collect(),
            classes: CLASSES.iter().map(|s| s.to_string()).collect(),
        }
    }
}

fn pick(settings: &Value, key: &str, all: &[&str]) -> Vec<String> {
    match settings
        .get("filter")
        .and_then(|f| f.get(key))
        .and_then(Value::as_array)
    {
        Some(list) => all
            .iter()
            .filter(|a| list.iter().any(|v| v.as_str() == Some(**a)))
            .map(|a| a.to_string())
            .collect(),
        None => all.iter().map(|a| a.to_string()).collect(),
    }
}

impl Filter {
    pub fn from_settings(settings: &Value) -> Filter {
        Filter {
            affiliations: pick(settings, "affiliations", &AFFILIATIONS),
            classes: pick(settings, "classes", &CLASSES),
        }
    }

    pub fn to_value(&self) -> Value {
        json!({"affiliations": self.affiliations, "classes": self.classes})
    }

    /// A list of names, cleaned to the ones that exist (in their usual order).
    pub fn clean(list: &Value, all: &[&str]) -> Option<Vec<String>> {
        let list = list.as_array()?;
        Some(
            all.iter()
                .filter(|a| list.iter().any(|v| v.as_str() == Some(**a)))
                .map(|a| a.to_string())
                .collect(),
        )
    }

    pub fn shows(&self, c: &Contact, show_stale: bool) -> bool {
        (show_stale || !c.stale)
            && self.classes.contains(&c.class)
            && (c.class != "unit" || self.affiliations.contains(&c.affiliation))
    }

    pub fn toggle(list: &mut Vec<String>, name: &str, all: &[&str]) {
        if let Some(i) = list.iter().position(|n| n == name) {
            list.remove(i);
        } else {
            list.push(name.to_string());
            list.sort_by_key(|n| all.iter().position(|a| a == n).unwrap_or(usize::MAX));
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chat {
    pub time: String,
    pub from: String,
    pub room: String,
    pub text: String,
}

impl Chat {
    fn from_json(v: &Value) -> Option<Chat> {
        let text = text(v, "text");
        (!text.is_empty()).then(|| Chat {
            time: text_of(v, "time"),
            from: text_of(v, "from"),
            room: text_of(v, "room"),
            text,
        })
    }
}

fn text_of(v: &Value, key: &str) -> String {
    text(v, key)
}

/// What an event did to the mirror.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Applied {
    Changed,
    /// An update was missed: ask the core for everything again.
    Gap,
}

/// The core's picture of the network, as far as it has been pushed.
#[derive(Default)]
pub struct Mirror {
    pub contacts: HashMap<String, Contact>,
    pub status: Value,
    pub chat: VecDeque<Chat>,
    pub seq: u64,
    /// Set when a snapshot or an update arrived at least once.
    pub live: bool,
}

impl Mirror {
    pub fn apply(&mut self, event: &Value) -> Applied {
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        let seq = event.get("seq").and_then(Value::as_u64).unwrap_or(0);
        match kind {
            "snapshot" => {
                self.contacts.clear();
                for c in event.get("contacts").and_then(Value::as_array).into_iter().flatten() {
                    self.put(c);
                }
                self.chat.clear();
                for m in event.get("chat").and_then(Value::as_array).into_iter().flatten() {
                    self.add_chat(m);
                }
                self.take_status(event);
                self.seq = seq;
                self.live = true;
                Applied::Changed
            }
            "update" => {
                let gap = self.live && seq != self.seq + 1;
                for c in event.get("upsert").and_then(Value::as_array).into_iter().flatten() {
                    self.put(c);
                }
                for u in event.get("remove").and_then(Value::as_array).into_iter().flatten() {
                    if let Some(u) = u.as_str() {
                        self.contacts.remove(u);
                    }
                }
                for m in event.get("chat").and_then(Value::as_array).into_iter().flatten() {
                    self.add_chat(m);
                }
                self.take_status(event);
                self.seq = seq;
                self.live = true;
                if gap {
                    Applied::Gap
                } else {
                    Applied::Changed
                }
            }
            _ => Applied::Changed,
        }
    }

    /// Replaces the contacts with a list the core answered with.
    pub fn replace(&mut self, list: &Value) {
        self.contacts.clear();
        for c in list.as_array().into_iter().flatten() {
            self.put(c);
        }
    }

    fn put(&mut self, v: &Value) {
        if let Some(c) = Contact::from_json(v) {
            if self.contacts.len() < MAX_CONTACTS || self.contacts.contains_key(&c.uid) {
                self.contacts.insert(c.uid.clone(), c);
            }
        }
    }

    fn add_chat(&mut self, v: &Value) {
        if let Some(m) = Chat::from_json(v) {
            if self.chat.len() >= MAX_CHAT {
                self.chat.pop_front();
            }
            self.chat.push_back(m);
        }
    }

    fn take_status(&mut self, event: &Value) {
        if let Some(s) = event.get("status").filter(|s| s.is_object()) {
            self.status = s.clone();
        }
    }

    pub fn connections(&self) -> &[Value] {
        self.status
            .get("connections")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice)
    }

    pub fn connections_on(&self) -> usize {
        self.connections()
            .iter()
            .filter(|c| matches!(c["state"].as_str(), Some("connected" | "listening")))
            .count()
    }

    /// The contacts nearest to `from` first (by callsign without one).
    pub fn sorted(&self, from: Option<(f64, f64)>) -> Vec<&Contact> {
        let mut list: Vec<&Contact> = self.contacts.values().collect();
        list.sort_by(|a, b| match from {
            Some(p) => distance_m(p, (a.lat, a.lon))
                .total_cmp(&distance_m(p, (b.lat, b.lon)))
                .then_with(|| a.uid.cmp(&b.uid)),
            None => a
                .callsign
                .to_lowercase()
                .cmp(&b.callsign.to_lowercase())
                .then_with(|| a.uid.cmp(&b.uid)),
        });
        list
    }

    /// A contact by uid, or by its callsign (exact first, then a part when
    /// only one contact has it).
    pub fn find(&self, query: &str) -> Result<&Contact, String> {
        if let Some(c) = self.contacts.get(query) {
            return Ok(c);
        }
        let q = query.to_lowercase();
        let exact: Vec<&Contact> = self
            .contacts
            .values()
            .filter(|c| c.callsign.to_lowercase() == q)
            .collect();
        let part: Vec<&Contact> = self
            .contacts
            .values()
            .filter(|c| c.callsign.to_lowercase().contains(&q))
            .collect();
        match (exact.len(), part.len()) {
            (1, _) => Ok(exact[0]),
            (0, 1) => Ok(part[0]),
            (0, 0) => Err(format!("No contact matches {query}.")),
            _ => Err(format!("More than one contact matches {query}; use its uid.")),
        }
    }
}

// ---- working with Map Entities ------------------------------------------------

/// The topic Map Entities publishes while it shares its entities.
pub const ENTITIES_TOPIC: &str = "map-entities/shared";
/// The plugin a contact can be handed to.
pub const ENTITIES_PLUGIN: &str = "map-entities";
/// The uid prefix of a marker sent for an entity: `corvus.me.<entity id>`.
pub const ENTITY_UID_PREFIX: &str = "corvus.me.";
/// How long a TAK client keeps a marker that is not renewed.
pub const ENTITY_STALE_S: f64 = 3600.0;
/// How often every marker is sent again, so a client that joined later sees it.
pub const ENTITY_RESEND_S: f64 = 1200.0;

fn number(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64).filter(|n| n.is_finite())
}

fn metres(m: f64) -> String {
    if m >= 10.0 {
        format!("{} m", m.round() as i64)
    } else {
        format!("{:.1} m", m)
    }
}

/// The CoT type a Map Entities kind is sent as: people and vehicles as what they
/// are, a target as unknown, everything else as a spot point.
pub fn cot_type_for(kind: &str) -> &'static str {
    match kind {
        "person" => "a-f-G-U-C",
        "vehicle" => "a-f-G-E-V",
        "target" => "a-u-G",
        _ => "b-m-p-s-p-i",
    }
}

/// The body of `POST /api/tak/marker` for one shared entity, with its uid; `None`
/// for an entity without a place or an id.
pub fn marker_for_entity(e: &Value) -> Option<(String, Value)> {
    let id = e.get("id").and_then(Value::as_str).filter(|s| !s.is_empty())?;
    let uid = format!("{ENTITY_UID_PREFIX}{id}");
    let kind = e.get("kind").and_then(Value::as_str).unwrap_or("flag");
    let label = e.get("label").and_then(Value::as_str).unwrap_or(kind);
    let (lng, lat) = (number(e, "lng")?, number(e, "lat")?);
    let mut parts = vec![label.to_string()];
    let mut note = String::new();
    let (mut at_lng, mut at_lat) = (lng, lat);
    if let (Some(lng2), Some(lat2)) = (number(e, "lng2"), number(e, "lat2")) {
        at_lng = (lng + lng2) / 2.0;
        at_lat = (lat + lat2) / 2.0;
        parts.push(format!("{} wide", metres(distance_m((lat, lng), (lat2, lng2)))));
        if let Some(h) = number(e, "height").filter(|h| *h > 0.0) {
            parts.push(format!("{} high", metres(h)));
        }
    } else if kind == "tree" {
        if let Some(r) = number(e, "radius").filter(|r| *r > 0.0) {
            parts.push(format!("crown radius {}", metres(r)));
        }
        if let Some(h) = number(e, "height").filter(|h| *h > 0.0) {
            parts.push(format!("{} high", metres(h)));
        }
    } else if let Some(r) = number(e, "radius").filter(|r| *r > 0.0) {
        parts.push(format!("radius {}", metres(r)));
    }
    if let Some(n) = e.get("note").and_then(Value::as_str).filter(|n| !n.is_empty()) {
        note = format!(". {n}");
    }
    let name = e
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .unwrap_or(label);
    let body = json!({
        "uid": uid, "lat": at_lat, "lon": at_lng, "type": cot_type_for(kind),
        "callsign": name, "remarks": format!("{}{note}", parts.join(", ")), "stale_s": ENTITY_STALE_S,
    });
    Some((uid, body))
}

/// What to put on the network for the entities in `list`: one marker body per uid.
pub fn markers_for_entities(list: &Value) -> std::collections::BTreeMap<String, Value> {
    list.as_array()
        .map(|a| a.iter().filter_map(marker_for_entity).collect())
        .unwrap_or_default()
}

/// What has to be sent and what has to be taken back so the network shows
/// `wanted`, given what was `pushed` (uid to the body sent, as text). `force`
/// sends everything again.
pub fn plan_entities(
    wanted: &std::collections::BTreeMap<String, Value>,
    pushed: &HashMap<String, String>,
    force: bool,
) -> (Vec<String>, Vec<String>) {
    let send = wanted
        .iter()
        .filter(|(uid, body)| force || pushed.get(*uid).map(String::as_str) != Some(body.to_string().as_str()))
        .map(|(uid, _)| uid.clone())
        .collect();
    let mut gone: Vec<String> = pushed.keys().filter(|u| !wanted.contains_key(*u)).cloned().collect();
    gone.sort();
    (send, gone)
}

/// The message that asks Map Entities to place a contact.
pub fn entity_for_contact(c: &Contact) -> Value {
    let kind = match (c.class.as_str(), c.affiliation.as_str(), c.icon()) {
        (_, "hostile", _) => "target",
        ("unit", _, "vehicle") => "vehicle",
        ("unit", _, _) => "person",
        ("alert", ..) => "hazard",
        _ => "flag",
    };
    let note = if c.remarks.is_empty() {
        format!("From TAK, {}", c.kind)
    } else {
        format!("From TAK, {}. {}", c.kind, c.remarks)
    };
    json!({"op": "add", "kind": kind, "name": c.callsign, "lng": c.lon, "lat": c.lat, "note": note})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(uid: &str, call: &str, kind: &str, class: &str, aff: &str, dim: &str, lat: f64) -> Value {
        json!({"uid": uid, "callsign": call, "type": kind, "class": class, "affiliation": aff, "dimension": dim,
            "lat": lat, "lon": 11.0, "stale": false, "source": "s", "age_s": 3.0})
    }

    fn entity(extra: Value) -> Value {
        let mut e = json!({"id": "e1", "kind": "radar", "label": "Radar", "name": "North radar", "lng": 11.0, "lat": 48.0, "note": "", "radius": 500.0});
        if let (Some(m), Some(x)) = (e.as_object_mut(), extra.as_object()) {
            m.extend(x.clone());
        }
        e
    }

    #[test]
    fn an_entity_becomes_a_marker_with_a_stable_uid() {
        let (uid, body) = marker_for_entity(&entity(json!({"note": "Keep clear"}))).unwrap();
        assert_eq!(uid, "corvus.me.e1");
        assert_eq!(body["type"], "b-m-p-s-p-i");
        assert_eq!(body["callsign"], "North radar");
        assert_eq!(body["remarks"], "Radar, radius 500 m. Keep clear");
        assert_eq!(body["stale_s"], ENTITY_STALE_S);
        assert!(marker_for_entity(&json!({"kind": "radar", "lng": 1, "lat": 2})).is_none());
        assert!(marker_for_entity(&json!({"id": "x", "kind": "radar"})).is_none());
    }

    #[test]
    fn a_gate_is_sent_at_the_middle_of_its_posts() {
        let gate = entity(
            json!({"id": "g", "kind": "gate", "label": "Gate", "name": "Gate 1", "radius": null,
            "lng": 11.0, "lat": 48.0, "lng2": 11.0, "lat2": 48.0002, "height": 5.0}),
        );
        let (_, body) = marker_for_entity(&gate).unwrap();
        assert!((body["lat"].as_f64().unwrap() - 48.0001).abs() < 1e-9);
        let remarks = body["remarks"].as_str().unwrap();
        assert!(remarks.starts_with("Gate, 22 m wide, 5.0 m high"), "{remarks}");
    }

    #[test]
    fn people_and_vehicles_keep_their_meaning_on_the_network() {
        assert_eq!(cot_type_for("person"), "a-f-G-U-C");
        assert_eq!(cot_type_for("vehicle"), "a-f-G-E-V");
        assert_eq!(cot_type_for("target"), "a-u-G");
        assert_eq!(cot_type_for("tree"), "b-m-p-s-p-i");
    }

    #[test]
    fn only_what_changed_is_sent_and_what_vanished_is_taken_back() {
        let list = json!([entity(json!({})), entity(json!({"id": "e2", "name": "Second"}))]);
        let wanted = markers_for_entities(&list);
        assert_eq!(wanted.len(), 2);
        let mut pushed: HashMap<String, String> = wanted.iter().map(|(u, b)| (u.clone(), b.to_string())).collect();
        assert_eq!(plan_entities(&wanted, &pushed, false), (vec![], vec![]));
        assert_eq!(plan_entities(&wanted, &pushed, true).0.len(), 2);
        pushed.insert("corvus.me.old".into(), "x".into());
        pushed.insert("corvus.me.e2".into(), "stale body".into());
        let (send, gone) = plan_entities(&wanted, &pushed, false);
        assert_eq!(send, vec!["corvus.me.e2".to_string()]);
        assert_eq!(gone, vec!["corvus.me.old".to_string()]);
        assert_eq!(
            plan_entities(&markers_for_entities(&Value::Null), &pushed, false)
                .1
                .len(),
            3
        );
    }

    #[test]
    fn a_contact_is_handed_over_as_a_point_map_entities_knows() {
        let mut hostile = Contact::from_json(&c("h", "HOSTILE 1", "a-h-G", "unit", "hostile", "ground", 48.0)).unwrap();
        assert_eq!(entity_for_contact(&hostile)["kind"], "target");
        hostile.affiliation = "friend".into();
        hostile.kind = "a-f-G-E-V-C".into();
        let m = entity_for_contact(&hostile);
        assert_eq!((m["op"].as_str(), m["kind"].as_str()), (Some("add"), Some("vehicle")));
        assert_eq!(m["name"], "HOSTILE 1");
        let spot = Contact::from_json(&c("p", "Spot", "b-m-p-s-p-i", "point", "unknown", "", 48.0)).unwrap();
        assert_eq!(entity_for_contact(&spot)["kind"], "flag");
    }

    #[test]
    fn a_snapshot_then_updates_keep_the_mirror_in_step() {
        let mut m = Mirror::default();
        let snap = json!({"type": "snapshot", "seq": 4, "contacts": [
            c("a", "ALPHA", "a-f-G-U-C", "unit", "friend", "ground", 48.0),
            c("b", "BRAVO", "a-h-G", "unit", "hostile", "ground", 48.1)],
            "chat": [{"text": "hi", "from": "ALPHA", "room": "All", "time": "t"}],
            "status": {"connections": [{"id": "x", "state": "connected"}, {"id": "y", "state": "off"}]}});
        assert_eq!(m.apply(&snap), Applied::Changed);
        assert_eq!(
            (m.contacts.len(), m.seq, m.chat.len(), m.connections_on()),
            (2, 4, 1, 1)
        );
        let up = json!({"type": "update", "seq": 5, "upsert": [c("a", "ALPHA", "a-f-G-U-C", "unit", "friend", "ground", 48.5)],
            "remove": ["b"], "chat": []});
        assert_eq!(m.apply(&up), Applied::Changed);
        assert_eq!(m.contacts.len(), 1);
        assert_eq!(m.contacts["a"].lat, 48.5);
        let lost = json!({"type": "update", "seq": 9, "upsert": [], "remove": [], "chat": []});
        assert_eq!(m.apply(&lost), Applied::Gap);
        assert_eq!(m.apply(&json!({"type": "update", "seq": 10})), Applied::Changed);
    }

    #[test]
    fn damaged_contacts_are_left_out() {
        let mut m = Mirror::default();
        m.apply(&json!({"type": "snapshot", "seq": 1, "contacts": [
            {"uid": "", "lat": 1, "lon": 1}, {"uid": "x", "lat": 91, "lon": 0}, {"uid": "y"}, 7,
            {"uid": "z", "lat": 1.5, "lon": 2.5}]}));
        assert_eq!(m.contacts.len(), 1);
        assert_eq!(
            m.contacts["z"].callsign, "z",
            "the uid stands in for a missing callsign"
        );
    }

    #[test]
    fn colours_and_symbols_follow_the_type() {
        let get = |kind: &str, class: &str, aff: &str, dim: &str| {
            Contact::from_json(&c("u", "U", kind, class, aff, dim, 1.0)).unwrap()
        };
        let friend = get("a-f-G-U-C", "unit", "friend", "ground");
        assert_eq!((friend.color_hex().as_str(), friend.icon()), ("#4DA3FF", "person"));
        assert_eq!(get("a-h-G-E-V", "unit", "hostile", "ground").icon(), "vehicle");
        assert_eq!(get("a-h-G-E-V", "unit", "hostile", "ground").color_hex(), "#FF5A5A");
        assert_eq!(get("a-f-A-M-F-Q", "unit", "friend", "air").icon(), "drone");
        assert_eq!(get("a-n-A-C-F", "unit", "neutral", "air").icon(), "plane");
        assert_eq!(get("a-u-S", "unit", "unknown", "sea").icon(), "boat");
        let spot =
            Contact::from_json(&json!({"uid": "p", "lat": 1, "lon": 1, "class": "point", "color": "#FF00FF"})).unwrap();
        assert_eq!((spot.icon(), spot.color_hex().as_str()), ("pin", "#FF00FF"));
        assert_eq!(get("b-m-p-s-p-i", "point", "other", "other").color_hex(), "#E6EDF3");
    }

    #[test]
    fn the_filter_decides_what_is_drawn() {
        let mut f = Filter::default();
        let unit = Contact::from_json(&c("a", "A", "a-h-G", "unit", "hostile", "ground", 1.0)).unwrap();
        let mut stale = unit.clone();
        stale.stale = true;
        assert!(f.shows(&unit, false) && !f.shows(&stale, false) && f.shows(&stale, true));
        Filter::toggle(&mut f.affiliations, "hostile", &AFFILIATIONS);
        assert!(!f.shows(&unit, false));
        Filter::toggle(&mut f.affiliations, "hostile", &AFFILIATIONS);
        assert_eq!(f.affiliations, AFFILIATIONS);
        let point = Contact::from_json(&json!({"uid": "p", "lat": 1, "lon": 1, "class": "point"})).unwrap();
        Filter::toggle(&mut f.affiliations, "friend", &AFFILIATIONS);
        assert!(f.shows(&point, false), "a marker has no affiliation to hide it by");
        assert_eq!(Filter::from_settings(&json!({"filter": f.to_value()})), f);
        assert_eq!(
            Filter::from_settings(&json!({"filter": {"classes": ["unit", "bogus"]}})).classes,
            ["unit"]
        );
        assert_eq!(Filter::from_settings(&json!({})), Filter::default());
    }

    #[test]
    fn the_list_is_nearest_first_and_a_contact_is_found_by_name() {
        let mut m = Mirror::default();
        m.apply(&json!({"type": "snapshot", "seq": 1, "contacts": [
            c("far", "Zulu", "a-f-G", "unit", "friend", "ground", 49.0),
            c("near", "Alpha", "a-f-G", "unit", "friend", "ground", 48.001),
            c("mid", "Alpha Two", "a-f-G", "unit", "friend", "ground", 48.5)]}));
        let order: Vec<&str> = m.sorted(Some((48.0, 11.0))).iter().map(|c| c.uid.as_str()).collect();
        assert_eq!(order, ["near", "mid", "far"]);
        let by_name: Vec<&str> = m.sorted(None).iter().map(|c| c.callsign.as_str()).collect();
        assert_eq!(by_name, ["Alpha", "Alpha Two", "Zulu"]);
        assert_eq!(m.find("far").unwrap().callsign, "Zulu");
        assert_eq!(
            m.find("alpha").unwrap().uid,
            "near",
            "an exact callsign wins over a part of one"
        );
        assert_eq!(m.find("zul").unwrap().uid, "far");
        assert!(m.find("two").is_ok());
        assert!(m.find("a").unwrap_err().contains("More than one"));
        assert!(m.find("nobody").unwrap_err().contains("No contact"));
    }

    #[test]
    fn distances_and_ages_read_plainly() {
        let d = distance_m((48.0, 11.0), (48.0, 11.001));
        assert!((d - 74.5).abs() < 1.0, "{d}");
        assert_eq!(distance_text(850.2), "850 m");
        assert_eq!(distance_text(2440.0), "2.4 km");
        assert_eq!(
            (
                age_text(12.0).as_str(),
                age_text(240.0).as_str(),
                age_text(10_800.0).as_str()
            ),
            ("12 s", "4 min", "3 h")
        );
    }
}
