//! `PostRevision#modifications`: a YAML-serialized HashWithIndifferentAccess
//! of field name to [before, after]. Rails reads any YAML carrying the
//! hash's tag, so this writes it with serde_yaml_ng rather than matching
//! Psych's quoting byte for byte; tests/writes.rs compares the column
//! parsed.

use serde_json::Value;
use serde_yaml_ng::value::{Tag, TaggedValue};

use crate::Unsupported;

const TAG: &str = "ruby/hash:ActiveSupport::HashWithIndifferentAccess";

/// The column's text for these changes, in field order.
pub fn dump(fields: &[(&str, [Value; 2])]) -> Result<String, Unsupported> {
    let mut map = serde_yaml_ng::Mapping::new();
    for (key, values) in fields {
        let pair = values
            .iter()
            .map(serde_yaml_ng::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Unsupported("revision values YAML cannot hold"))?;
        map.insert((*key).into(), serde_yaml_ng::Value::Sequence(pair));
    }
    let tagged = serde_yaml_ng::Value::Tagged(Box::new(TaggedValue {
        tag: Tag::new(TAG),
        value: serde_yaml_ng::Value::Mapping(map),
    }));
    serde_yaml_ng::to_string(&tagged).map_err(|_| Unsupported("revision values YAML cannot hold"))
}

/// A stored revision's changes, field to [before, after], in stored order.
pub fn load(yaml: &str) -> Result<Vec<(String, [Value; 2])>, Unsupported> {
    let parsed: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(yaml).map_err(|_| Unsupported("unreadable revision YAML"))?;
    let map = match parsed {
        serde_yaml_ng::Value::Tagged(t) => t.value,
        other => other,
    };
    let serde_yaml_ng::Value::Mapping(map) = map else {
        return Err(Unsupported("revision modifications that are not a hash"));
    };
    let mut out = Vec::new();
    for (k, v) in map {
        let (serde_yaml_ng::Value::String(key), serde_yaml_ng::Value::Sequence(items)) = (k, v)
        else {
            return Err(Unsupported(
                "revision modifications that are not field pairs",
            ));
        };
        let [before, after]: [serde_yaml_ng::Value; 2] = items
            .try_into()
            .map_err(|_| Unsupported("revision modifications that are not field pairs"))?;
        let json = |v: serde_yaml_ng::Value| {
            serde_json::to_value(v).map_err(|_| Unsupported("revision values JSON cannot hold"))
        };
        out.push((key, [json(before)?, json(after)?]));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_what_rails_wrote() {
        let rails = "--- !ruby/hash:ActiveSupport::HashWithIndifferentAccess\nraw:\n- Reply one.\n- Reply two.\ncooked:\n- \"<p>Reply one.</p>\"\n- \"<p>Reply two.</p>\"\nedit_reason:\n-\n- fixing it up\n";
        let fields = load(rails).unwrap();
        assert_eq!(
            fields[0],
            ("raw".into(), [json!("Reply one."), json!("Reply two.")])
        );
        assert_eq!(
            fields[2],
            ("edit_reason".into(), [Value::Null, json!("fixing it up")])
        );
    }

    #[test]
    fn loads_what_it_dumps() {
        let fields = [
            ("raw", [json!("a: b"), json!("two\nlines")]),
            ("edit_reason", [Value::Null, json!("why")]),
            ("wiki", [json!(false), json!(true)]),
        ];
        let yaml = dump(&fields).unwrap();
        assert!(yaml.starts_with("!ruby/hash:ActiveSupport::HashWithIndifferentAccess\n"));
        let loaded = load(&yaml).unwrap();
        for (i, (k, v)) in fields.iter().enumerate() {
            assert_eq!(loaded[i], (k.to_string(), v.clone()));
        }
    }
}
