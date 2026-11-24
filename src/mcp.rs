//! What the plugin offers an AI client through Corvus' MCP server, when the
//! operator has allowed it twice: Plugin tools in Settings, Connections, MCP,
//! and "Let an AI client steer the map layer" in the plugin's gear.
//!
//! They change only what the Home map shows (which contact is framed, which
//! kinds are drawn). Reading contacts, sending markers and setting up
//! connections are the core's own tools (`tak_*`), with permissions of their own.

use serde_json::{json, Value};

use super::model::{Filter, AFFILIATIONS, CLASSES};
use super::{Tak, MCP_OPTION};
use corvus_api::plugin::{Host, McpTool};

#[derive(Default)]
pub struct State {
    done: Vec<(u64, Result<Value, String>)>,
}

fn tool(name: &str, title: &str, description: &str, schema: Value) -> McpTool {
    McpTool {
        name: name.into(),
        title: title.into(),
        description: description.into(),
        input_schema: schema,
        confirm: false,
        read_only: false,
    }
}

pub(crate) fn tools_for(h: &Host) -> Vec<McpTool> {
    if h.options().get(MCP_OPTION) != Some(&Value::Bool(true)) {
        return Vec::new();
    }
    vec![
        tool(
            "focus_contact",
            "Focus a TAK contact",
            "Select a contact on the Home map and frame it. contact is its uid or its callsign (a part of it is enough when only one contact has it).",
            json!({"type": "object", "properties": {"contact": {"type": "string"}}, "required": ["contact"]}),
        ),
        tool(
            "fit_contacts",
            "Frame the TAK contacts",
            "Frame every contact the map currently shows, or only the listed ones (uids or callsigns).",
            json!({"type": "object", "properties": {"contacts": {"type": "array", "items": {"type": "string"}}}}),
        ),
        tool(
            "set_map_filter",
            "Choose what the TAK layer shows",
            "Choose which kinds of TAK contact the Home map draws. affiliations narrows people and vehicles to friend, hostile, neutral or unknown; classes to unit, point (spot markers), route, drawing or alert. An empty list hides that kind altogether. What you leave out stays as it was. Returns what is shown now.",
            json!({"type": "object", "properties": {
                "affiliations": {"type": "array", "items": {"type": "string", "enum": AFFILIATIONS}},
                "classes": {"type": "array", "items": {"type": "string", "enum": CLASSES}},
            }}),
        ),
    ]
}

fn string_list(v: &Value) -> Result<Vec<String>, String> {
    v.as_array()
        .ok_or("must be a list of text")?
        .iter()
        .map(|x| {
            x.as_str()
                .map(str::to_string)
                .ok_or_else(|| "must be a list of text".to_string())
        })
        .collect()
}

impl Tak {
    pub(crate) fn mcp_start(&mut self, name: &str, args: &Value, token: u64, h: &mut Host) {
        let result = self.mcp_now(name, args, h);
        self.mcp.done.push((token, result));
    }

    pub(crate) fn mcp_result(&mut self, token: u64) -> Option<Result<Value, String>> {
        let i = self.mcp.done.iter().position(|(t, _)| *t == token)?;
        Some(self.mcp.done.remove(i).1)
    }

    pub(crate) fn mcp_run(&mut self, _h: &mut Host) {
        if self.mcp.done.len() > 32 {
            self.mcp.done.drain(..self.mcp.done.len() - 32);
        }
    }

    fn mcp_now(&mut self, name: &str, args: &Value, h: &mut Host) -> Result<Value, String> {
        if !tools_for(h).iter().any(|t| t.name == name) {
            return Err(format!("The plugin offers no tool {name} right now."));
        }
        match name {
            "focus_contact" => {
                let query = args
                    .get("contact")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or("contact is required")?;
                let c = self.mirror.find(query)?.clone();
                self.select(Some(c.uid.clone()));
                h.fit(vec![[c.lon, c.lat]]);
                Ok(json!({"ok": true, "focused": {"uid": c.uid, "callsign": c.callsign, "lat": c.lat, "lon": c.lon}}))
            }
            "fit_contacts" => {
                let framed = match args.get("contacts") {
                    None | Some(Value::Null) => self.fit(None, h),
                    Some(list) => {
                        let mut uids = Vec::new();
                        for q in string_list(list).map_err(|e| format!("contacts {e}"))? {
                            uids.push(self.mirror.find(&q)?.uid.clone());
                        }
                        self.fit(Some(&uids), h)
                    }
                };
                if framed == 0 {
                    return Err("There is nothing to frame: no contact is shown.".into());
                }
                Ok(json!({"ok": true, "framed": framed}))
            }
            "set_map_filter" => {
                for (key, all, target) in [("affiliations", &AFFILIATIONS[..], 0), ("classes", &CLASSES[..], 1)] {
                    if let Some(list) = args.get(key).filter(|v| !v.is_null()) {
                        string_list(list).map_err(|e| format!("{key} {e}"))?;
                        let cleaned = Filter::clean(list, all).ok_or_else(|| format!("{key} must be a list"))?;
                        if target == 0 {
                            self.filter.affiliations = cleaned;
                        } else {
                            self.filter.classes = cleaned;
                        }
                    }
                }
                self.save_filter(h);
                Ok(json!({"ok": true, "shown": self.filter.to_value()}))
            }
            other => Err(format!("The plugin offers no tool {other}.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_of_text_are_checked() {
        assert_eq!(string_list(&json!(["a", "b"])).unwrap(), ["a", "b"]);
        assert!(string_list(&json!([1])).is_err());
        assert!(string_list(&json!("a")).is_err());
    }

    #[test]
    fn answers_are_taken_once_and_the_pile_stays_small() {
        let mut t = Tak::default();
        t.mcp.done.push((1, Ok(json!(1))));
        t.mcp.done.push((2, Err("no".into())));
        assert_eq!(t.mcp_result(2), Some(Err("no".into())));
        assert_eq!(t.mcp_result(2), None);
        assert_eq!(t.mcp_result(1), Some(Ok(json!(1))));
    }
}
