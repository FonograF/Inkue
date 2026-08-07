//! NSKeyedArchiver graph resolution.
//!
//! A `.qlab4` / `.qlab5` file is **not** a database: it is a binary property
//! list written by Apple's `NSKeyedArchiver`, which stores a flat `$objects`
//! array whose entries reference each other by `Uid` index. Nothing here is
//! specific to QLab — this is Apple's container format, and the `plist` crate
//! reads the bytes; resolving the object graph back into a tree is ours.
//!
//! The result is a [`serde_json::Value`] tree where each archived object
//! becomes an object carrying a `"__class__"` key, which is what the mapping
//! layer matches on. JSON is the right shape here because the mapping's output
//! is JSON too, so one representation carries the whole pipeline.

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use plist::{Uid, Value as Plist};
use serde_json::{json, Map, Value};

/// Deepest nesting resolved before a branch is cut short.
///
/// The graph is cyclic by nature (a cue points at its parent, which lists the
/// cue), and the memo below breaks true cycles — this is the second belt, for
/// the pathologically deep container nesting some workspaces produce.
const MAX_DEPTH: usize = 48;

/// Resolve a whole NSKeyedArchiver plist into a JSON tree.
pub fn resolve(bytes: &[u8]) -> Result<Value> {
    let root: Plist =
        plist::from_bytes(bytes).context("not a property list (expected a QLab workspace)")?;
    let objects = root
        .as_dictionary()
        .and_then(|d| d.get("$objects"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("no $objects array — not an NSKeyedArchiver plist"))?;

    let mut resolver = Resolver { objects, memo: HashMap::new() };
    // Index 1 is the archive root: index 0 is always the "$null" placeholder.
    Ok(resolver.value(&Plist::Uid(Uid::new(1)), 0))
}

struct Resolver<'a> {
    objects: &'a [Plist],
    /// Index → already-resolved value. Also the cycle guard: an index being
    /// resolved maps to `Null` until it completes, so a back-reference
    /// terminates instead of recursing forever.
    memo: HashMap<u64, Value>,
}

impl Resolver<'_> {
    fn value(&mut self, plist: &Plist, depth: usize) -> Value {
        if depth > MAX_DEPTH {
            return Value::Null;
        }
        match plist {
            Plist::Uid(uid) => self.follow(uid.get(), depth),
            // "$null" is the archiver's way of writing an absent value.
            Plist::String(s) if s == "$null" => Value::Null,
            Plist::String(s) => Value::String(s.clone()),
            Plist::Boolean(b) => Value::Bool(*b),
            Plist::Integer(i) => i
                .as_signed()
                .map(|n| json!(n))
                .or_else(|| i.as_unsigned().map(|n| json!(n)))
                .unwrap_or(Value::Null),
            Plist::Real(f) => json!(f),
            Plist::Array(items) => {
                Value::Array(items.iter().map(|i| self.value(i, depth + 1)).collect())
            }
            Plist::Dictionary(_) => self.object(plist, depth),
            // Binary blobs matter: the cue tree itself is one (see `mod.rs`).
            Plist::Data(bytes) => json!({ "__data__": bytes.clone() }),
            _ => Value::Null,
        }
    }

    fn follow(&mut self, index: u64, depth: usize) -> Value {
        if let Some(done) = self.memo.get(&index) {
            return done.clone();
        }
        let Some(entry) = self.objects.get(index as usize).cloned() else {
            return Value::Null;
        };
        // Placeholder first: anything that points back here while we resolve
        // gets Null rather than recursing.
        self.memo.insert(index, Value::Null);
        let resolved = self.value(&entry, depth);
        self.memo.insert(index, resolved.clone());
        resolved
    }

    /// Resolve one archived object, unwrapping the Foundation containers into
    /// plain JSON and keeping everything else as a `"__class__"`-tagged object.
    fn object(&mut self, plist: &Plist, depth: usize) -> Value {
        let Some(dict) = plist.as_dictionary() else {
            return Value::Null;
        };
        let class = self.class_name(dict.get("$class"));

        match class.as_deref() {
            Some("NSString" | "NSMutableString") => {
                return dict
                    .get("NS.string")
                    .map(|v| self.value(v, depth + 1))
                    .unwrap_or(Value::Null);
            }
            // Keep the words, drop the styling: a Titles cue's text lives here.
            Some("NSAttributedString" | "NSMutableAttributedString") => {
                return dict
                    .get("NSString")
                    .map(|v| self.value(v, depth + 1))
                    .unwrap_or(Value::Null);
            }
            Some("NSArray" | "NSMutableArray" | "NSSet" | "NSMutableSet") => {
                let items = dict.get("NS.objects").and_then(|v| v.as_array());
                return Value::Array(
                    items
                        .map(|a| a.iter().map(|i| self.value(i, depth + 1)).collect())
                        .unwrap_or_default(),
                );
            }
            Some("NSDictionary" | "NSMutableDictionary") => {
                return self.dictionary(dict, depth);
            }
            Some("NSMutableData" | "NSData") => {
                return dict
                    .get("NS.data")
                    .map(|v| self.value(v, depth + 1))
                    .unwrap_or(Value::Null);
            }
            Some("NSURL") => {
                // QLab 4 media target.
                let relative = dict.get("NS.relative").map(|v| self.value(v, depth + 1));
                return json!({
                    "__class__": "NSURL",
                    "relative": relative.unwrap_or(Value::Null),
                });
            }
            _ => {}
        }

        // A custom class (AudioCue, F53Alias, …): keep every plain-named
        // property. QLab's own properties are plainly named, which is what
        // makes this format tractable at all.
        let mut out = Map::new();
        if let Some(name) = class {
            out.insert("__class__".into(), Value::String(name));
        }
        for (key, value) in dict.iter() {
            if key.starts_with('$') {
                continue;
            }
            out.insert(key.clone(), self.value(value, depth + 1));
        }
        Value::Object(out)
    }

    /// An `NSDictionary` stores its keys and values as two parallel arrays.
    fn dictionary(&mut self, dict: &plist::Dictionary, depth: usize) -> Value {
        let keys = dict.get("NS.keys").and_then(|v| v.as_array());
        let values = dict.get("NS.objects").and_then(|v| v.as_array());
        let (Some(keys), Some(values)) = (keys, values) else {
            return Value::Object(Map::new());
        };
        let mut out = Map::new();
        for (key, value) in keys.iter().zip(values.iter()) {
            let key = match self.value(key, depth + 1) {
                Value::String(s) => s,
                other => other.to_string(),
            };
            out.insert(key, self.value(value, depth + 1));
        }
        Value::Object(out)
    }

    /// The `$classname` an object's `$class` reference points at.
    fn class_name(&self, class_ref: Option<&Plist>) -> Option<String> {
        let uid = class_ref?.as_uid()?;
        self.objects
            .get(uid.get() as usize)?
            .as_dictionary()?
            .get("$classname")?
            .as_string()
            .map(str::to_string)
    }
}

/// The bytes of a nested archive stored as `NS.data`.
///
/// A QLab workspace holds its whole cue tree as a *second* archive inside the
/// first — this pulls those bytes back out of the resolved tree.
pub fn nested_data(value: &Value) -> Option<Vec<u8>> {
    let bytes = value.get("__data__")?.as_array()?;
    bytes
        .iter()
        .map(|b| b.as_u64().map(|n| n as u8))
        .collect::<Option<Vec<u8>>>()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_is_not_a_plist_is_rejected() {
        assert!(resolve(b"this is not a plist at all").is_err());
    }

    #[test]
    fn a_plist_without_objects_is_rejected() {
        // Valid XML plist, but not an NSKeyedArchiver.
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>hello</key><string>world</string></dict></plist>"#;
        let err = resolve(xml).unwrap_err().to_string();
        assert!(err.contains("$objects"), "unexpected error: {err}");
    }

    #[test]
    fn nested_data_reads_bytes_back_out() {
        let value = json!({ "__data__": [1, 2, 255] });
        assert_eq!(nested_data(&value), Some(vec![1, 2, 255]));
        assert_eq!(nested_data(&json!({})), None);
    }
}
