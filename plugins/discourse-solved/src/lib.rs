//! discourse-solved's topic list, as a third-party plugin compiled to
//! WebAssembly (docs/plugin-wasm.md), ported from the Luau spike
//! (branch spike/plugin-runtime, plugins/discourse-solved/plugin.luau)
//! and the bundled port's topic_list_keys (src/plugins/solved):
//!
//! - after_load: each topic's accepted answerers (the users of its answer
//!   posts), loaded with the page (register_topic_list_preload_user_ids);
//! - before_serialize: the answerers join the posters second, described
//!   "Accepted Answer", and an answerer who posted last keeps their place
//!   (TopicPostersSummaryExtension);
//! - after_serialize: has_accepted_answer, and can_have_answer where the
//!   category shows an empty box (TopicAnswerMixin).

use std::collections::{BTreeMap, HashMap, HashSet};

use discourse_plugin::{Input, core_rows, rows, setting, t};
use serde_json::{Map, Value, json};

discourse_plugin::phases! {
    "topic_list.after_load" => after_load,
    "topic_list.before_serialize" => before_serialize,
    "topic_list.after_serialize" => after_serialize,
}

fn int(v: &Value) -> Option<i64> {
    v.as_i64()
}

/// Ruby truthiness of a setting's value.
fn truthy(v: &Value) -> bool {
    !matches!(v, Value::Null | Value::Bool(false))
}

fn topic_ids(topics: &[Value]) -> Vec<i64> {
    topics.iter().filter_map(|t| int(&t["id"])).collect()
}

/// topic id => the users of its accepted answers' posts, in answer
/// order (Rails' join on posts keeps deleted ones); answered topics with
/// no such user are present with none.
fn answerers(topic_ids: &[i64]) -> BTreeMap<i64, Vec<i64>> {
    let solved = rows("discourse_solved_solved_topics", "topic_id", topic_ids);
    let solved_topic: HashMap<i64, i64> = solved
        .iter()
        .filter_map(|r| Some((int(&r["id"])?, int(&r["topic_id"])?)))
        .collect();
    let solved_ids: Vec<i64> = solved_topic.keys().copied().collect();
    let mut answers = rows(
        "discourse_solved_topic_answers",
        "solved_topic_id",
        &solved_ids,
    );
    answers.sort_by_key(|r| int(&r["id"]));
    let post_ids: Vec<i64> = answers
        .iter()
        .filter_map(|r| int(&r["answer_post_id"]))
        .collect();
    let authors: HashMap<i64, i64> = core_rows("posts", "id", &post_ids)
        .iter()
        .filter_map(|p| Some((int(&p["id"])?, int(&p["user_id"])?)))
        .collect();
    let mut out: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for answer in &answers {
        let Some(topic_id) = int(&answer["solved_topic_id"]).and_then(|s| solved_topic.get(&s))
        else {
            continue;
        };
        let users = out.entry(*topic_id).or_default();
        let author = int(&answer["answer_post_id"]).and_then(|p| authors.get(&p));
        if let Some(user_id) = author
            && !users.contains(user_id)
        {
            users.push(*user_id);
        }
    }
    out
}

fn after_load(input: Input) -> Result<Value, String> {
    let topics = input.payload.as_array().ok_or("payload: not a list")?;
    let mut user_ids = Vec::new();
    let mut data = Map::new();
    for (topic_id, users) in answerers(&topic_ids(topics)) {
        user_ids.extend(users.iter().copied());
        data.insert(
            topic_id.to_string(),
            json!({ "answered": true, "answerers": users }),
        );
    }
    Ok(json!({ "user_ids": user_ids, "topics": data }))
}

fn before_serialize(input: Input) -> Result<Value, String> {
    let topics = input.payload.as_object().ok_or("payload: not an object")?;
    let accepted = t("accepted_answer");
    let mut out = Map::new();
    for (id, topic) in topics {
        let users: Vec<i64> = topic["data"]["answerers"]
            .as_array()
            .map(|a| a.iter().filter_map(int).collect())
            .unwrap_or_default();
        if users.is_empty() {
            continue;
        }
        let mut posters = topic["posters"].clone();
        let user_ids = posters["user_ids"]
            .as_array_mut()
            .ok_or("posters.user_ids: not a list")?;
        // The answerers right after the author.
        let at = user_ids.len().min(1);
        for (i, user_id) in users.iter().enumerate() {
            user_ids.insert(at + i, json!(user_id));
        }
        let descriptions = posters["descriptions"]
            .as_object_mut()
            .ok_or("posters.descriptions: not an object")?;
        for user_id in &users {
            let list = descriptions
                .entry(user_id.to_string())
                .or_insert_with(|| json!([]));
            if let Some(list) = list.as_array_mut() {
                list.push(json!(accepted));
            }
        }
        // last_poster_is_topic_creator? also holds for an answerer.
        if int(&topic["last_post_user_id"]).is_some_and(|last| users.contains(&last)) {
            posters["last_poster_stays"] = json!(true);
        }
        out.insert(id.clone(), json!({ "posters": posters }));
    }
    Ok(Value::Object(out))
}

/// `category.custom_fields[name]` for the page's categories, the first
/// row by id as Rails finds it.
struct CategoryFields(Vec<Value>);

impl CategoryFields {
    fn load(topics: &[Value]) -> Self {
        let mut ids: Vec<i64> = topics
            .iter()
            .filter_map(|t| int(&t["category_id"]))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        CategoryFields(core_rows("category_custom_fields", "category_id", &ids))
    }

    fn is_true(&self, category_id: Option<i64>, name: &str) -> bool {
        let Some(category_id) = category_id else {
            return false;
        };
        self.0
            .iter()
            .find(|r| int(&r["category_id"]) == Some(category_id) && r["name"] == name)
            .is_some_and(|r| r["value"] == "true")
    }
}

/// What `allow_accepted_answers?` reads beyond the topic, loaded once
/// for the page when some topic needs it.
struct Allowed {
    all_topics: bool,
    fields: CategoryFields,
    /// topic id => its tags' names, when enable_solved_tags is set.
    tags: HashMap<i64, Vec<String>>,
    solved_tags: Vec<String>,
    /// private message id => its allowed groups among allow_solved_in_groups.
    pm_groups: HashSet<i64>,
}

impl Allowed {
    fn load(topics: &[Value], fields: CategoryFields) -> Self {
        let all_topics = truthy(&setting("allow_solved_on_all_topics"));
        let solved_tags: Vec<String> = setting("enable_solved_tags")
            .as_str()
            .unwrap_or("")
            .split('|')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        let mut tags: HashMap<i64, Vec<String>> = HashMap::new();
        if !solved_tags.is_empty() {
            let topic_tags = core_rows("topic_tags", "topic_id", &topic_ids(topics));
            let tag_ids: Vec<i64> = topic_tags
                .iter()
                .filter_map(|r| int(&r["tag_id"]))
                .collect();
            let names: HashMap<i64, String> = core_rows("tags", "id", &tag_ids)
                .iter()
                .filter_map(|r| Some((int(&r["id"])?, r["name"].as_str()?.to_string())))
                .collect();
            for r in &topic_tags {
                if let (Some(topic_id), Some(name)) = (
                    int(&r["topic_id"]),
                    int(&r["tag_id"]).and_then(|id| names.get(&id)),
                ) {
                    tags.entry(topic_id).or_default().push(name.clone());
                }
            }
        }
        let groups: Vec<i64> = match setting("allow_solved_in_groups") {
            Value::String(s) => s.split('|').filter_map(|g| g.parse().ok()).collect(),
            Value::Number(n) => n.as_i64().into_iter().collect(),
            _ => Vec::new(),
        };
        let pms: Vec<i64> = topics
            .iter()
            .filter(|t| t["archetype"] == "private_message")
            .filter_map(|t| int(&t["id"]))
            .collect();
        let pm_groups = if groups.is_empty() {
            HashSet::new()
        } else {
            core_rows("topic_allowed_groups", "topic_id", &pms)
                .iter()
                .filter(|r| int(&r["group_id"]).is_some_and(|g| groups.contains(&g)))
                .filter_map(|r| int(&r["topic_id"]))
                .collect()
        };
        Allowed {
            all_topics,
            fields,
            tags,
            solved_tags,
            pm_groups,
        }
    }

    /// `allow_accepted_answers?(topic)`
    fn allows(&self, topic: &Value) -> bool {
        if self.all_topics {
            return true;
        }
        let id = int(&topic["id"]).unwrap_or_default();
        if topic["archetype"] == "private_message" {
            return self.pm_groups.contains(&id);
        }
        // solved_enabled_for_category?: a solved tag, or the category's field.
        let tagged = self
            .tags
            .get(&id)
            .is_some_and(|names| names.iter().any(|n| self.solved_tags.contains(n)));
        tagged
            || self
                .fields
                .is_true(int(&topic["category_id"]), "enable_accepted_answers")
    }
}

fn after_serialize(input: Input) -> Result<Value, String> {
    let topics = input.payload.as_array().ok_or("payload: not a list")?;
    let fields = CategoryFields::load(topics);
    let empty_box: Vec<&Value> = topics
        .iter()
        .filter(|t| fields.is_true(int(&t["category_id"]), "empty_box_on_unsolved"))
        .collect();
    let allowed = (!empty_box.is_empty()).then(|| Allowed::load(topics, fields));
    let mut out = Map::new();
    for topic in topics {
        let Some(id) = int(&topic["id"]) else {
            continue;
        };
        let mut keys = Map::new();
        keys.insert(
            "has_accepted_answer".into(),
            json!(topic["data"]["answered"] == true),
        );
        if let Some(allowed) = &allowed
            && empty_box.iter().any(|t| int(&t["id"]) == Some(id))
        {
            let can_have = if allowed.all_topics {
                true
            } else if topic["archetype"] != "private_message"
                && (topic["closed"] == true || topic["archived"] == true)
            {
                false
            } else {
                allowed.allows(topic)
            };
            keys.insert("can_have_answer".into(), json!(can_have));
        }
        out.insert(id.to_string(), Value::Object(keys));
    }
    Ok(Value::Object(out))
}
