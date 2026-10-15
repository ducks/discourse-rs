//! preferences/profile.gjs, with UserField and UppyImageUploader.

use serde_json::{Map, Value, json};

use super::{Combo, Prefs, button, checkbox, combo, save_attrs, save_controls, value_string};
use crate::topic_list_view::{ListContext, emoji_unescape, escape, icon, t};
use crate::{AppError, Unsupported};

/// What the profile tab reads beside the user.
pub struct Profile<'a> {
    /// `site.user_fields`
    pub site_user_fields: &'a [Value],
    /// `currentUser.needs_required_fields_check`
    pub needs_required_fields_check: bool,
    /// The moment.js and moment-timezone scripts the timezone list and
    /// Use Current Timezone load.
    pub moment: (String, String),
}

/// UserFieldBase's `customFieldClass`.
fn custom_field_class(name: &str) -> String {
    let dashed = name.split_whitespace().collect::<Vec<_>>().join("-");
    let kept: String = dashed
        .chars()
        .filter(|c| !"!\"#$%&'()*+,./:;<=>?@[\\]^`{|}~".contains(*c))
        .collect::<String>()
        .to_lowercase();
    if kept.is_empty() {
        String::new()
    } else {
        format!(" user-field-{kept}")
    }
}

/// UserField: the component its field_type names.
fn user_field(cx: &ListContext, field: &Value, value: &Value) -> Result<String, AppError> {
    let id = field["id"].as_i64().unwrap_or_default();
    let name = field["name"].as_str().unwrap_or_default();
    let kind = field["field_type"].as_str().unwrap_or_default();
    let required = field["required"] == true;
    let input_id = format!("user-field-{id}-input");
    let label_text = if required {
        escape(name)
    } else {
        format!("{}{}", escape(name), escape(&t(cx, "user_fields.optional")))
    };
    let description = field["description"].as_str().unwrap_or_default();
    let instructions = format!("<div class=\"instructions\">{description}</div>");
    let text_value = match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    };
    let inner = match kind {
        "text" | "date" => format!(
            "<div class=\"controls\"><input id=\"{input_id}\"{} class=\"ember-text-field\" type=\"{}\" value=\"{}\" data-user-field=\"{id}\"><label class=\"control-label alt-placeholder\" for=\"{input_id}\">{label_text}</label>{instructions}</div>",
            if kind == "text" {
                " maxlength=\"2048\""
            } else {
                ""
            },
            if kind == "text" { "text" } else { "date" },
            escape(&text_value)
        ),
        "textarea" => format!(
            "<div class=\"controls\"><textarea class=\"form-template-field__textarea ember-text-area\" id=\"{input_id}\" maxlength=\"2048\" data-user-field=\"{id}\">{}</textarea><label class=\"control-label alt-placeholder\" for=\"{input_id}\">{label_text}</label>{instructions}</div>",
            escape(&text_value)
        ),
        "confirm" => {
            let checked = value == &json!(true)
                || crate::user_fields::CUSTOM_FIELD_TRUE.contains(&text_value.as_str());
            let mut out = String::new();
            if !name.is_empty() {
                out.push_str(&format!(
                    "<label class=\"control-label\">{label_text}</label>"
                ));
            }
            out.push_str(&format!(
                "<div class=\"controls\"><label class=\"control-label checkbox-label\"><input id=\"{input_id}\" class=\"ember-checkbox\" type=\"checkbox\"{} data-user-field=\"{id}\"> <span>{description}{}</span></label></div>",
                if checked { " checked" } else { "" },
                if name.is_empty() && required {
                    "<span class=\"required\">*</span>"
                } else {
                    ""
                }
            ));
            out
        }
        "dropdown" => {
            let rows: Vec<(String, String)> = field["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|o| o.as_str())
                .map(|o| (o.to_string(), o.to_string()))
                .collect();
            let select = combo(
                cx,
                &Combo {
                    id: Some(&input_id),
                    component_class: "",
                    class: "",
                    rows,
                    value: (!text_value.is_empty()).then(|| text_value.clone()),
                    none: Some(t(cx, "user_fields.none")),
                    key: None,
                    filterable: false,
                },
            )
            .replacen("<details", &format!("<details data-user-field=\"{id}\""), 1);
            format!(
                "<label class=\"control-label alt-placeholder\" for=\"{input_id}\">{label_text}</label><div class=\"controls\">{select}{instructions}</div>"
            )
        }
        "multiselect" => {
            return Err(
                Unsupported("a multiselect user field in preferences (MultiSelect, 196)").into(),
            );
        }
        _ => return Err(Unsupported("a user field of an unknown type").into()),
    };
    Ok(format!(
        "<div class=\"control-group\" data-setting-name=\"user-user-fields\"><div class=\"user-field {kind}{}\">{inner}</div></div>",
        custom_field_class(name)
    ))
}

/// UppyImageUploader: the image with its change and delete controls, or
/// the file picker.
fn image_uploader(cx: &ListContext, id: &str, kind: &str, url: Option<&str>, key: &str) -> String {
    let picker = format!(
        "{}<div class=\"pick-files-button\"><input accept=\"image/*\" id=\"{id}__input\" type=\"file\"></div>",
        icon("upload", None)
    );
    let preview = match url {
        Some(url) => format!(
            "<div class=\"file-uploader__preview input-xxlarge\" style=\"background-image: url({u})\"><a class=\"lightbox\" href=\"{u}\" rel=\"nofollow ugc noopener\"><div class=\"meta\"><span class=\"informations\">x </span></div></a><div class=\"expand-overlay\">{}</div></div><div class=\"file-uploader__controls\"><label class=\"btn btn-default btn-small\" for=\"{id}__input\" tabindex=\"0\">{picker} {}</label>{}</div>",
            button(
                "btn-default btn-small image-uploader-lightbox-btn",
                Some("discourse-expand"),
                None,
                &format!(" title=\"{}\"", escape(&t(cx, "expand")))
            ),
            escape(&t(cx, "upload_selector.change")),
            button(
                "btn-danger btn-small file-uploader__delete",
                Some("trash-can"),
                Some(&t(cx, "upload_selector.delete")),
                ""
            ),
            u = escape(url),
        ),
        None => format!(
            "<div class=\"file-uploader__preview input-xxlarge\" style=\"background-image: url()\"><div class=\"file-uploader__controls\"><label class=\"btn btn-transparent\" for=\"{id}__input\" tabindex=\"0\">{picker} {}</label><div class=\"file-uploader__progress-status hidden\"><div aria-label=\"{u} 0%\" class=\"progress-bar-container\" role=\"progressbar\"><div class=\"progress-bar\" style=\"width: 0%\"></div></div><span>{u} 0%</span></div></div></div>",
            escape(&t(cx, "upload_selector.select_file")),
            u = escape(&t(cx, "upload_selector.uploading")),
        ),
    };
    format!(
        "<div class=\"file-uploader {}\" id=\"{id}\" data-upload-type=\"{kind}\" data-pref=\"{key}\">{preview}</div>",
        if url.is_some() {
            "has-image"
        } else {
            "no-image"
        },
    )
}

/// preferences/profile.gjs
pub fn render(p: &Prefs, pr: &Profile) -> Result<(String, Map<String, Value>), AppError> {
    let cx = p.cx;
    let u = p.u;
    let mut out = String::new();
    let enforced = pr.needs_required_fields_check;
    if enforced {
        out.push_str(&format!(
            "<div class=\"alert alert-error\">{}</div>",
            escape(&t(cx, "user.preferences.profile.enforced_required_fields"))
        ));
    }
    if !enforced {
        if p.setting("allow_users_to_hide_profile")? {
            out.push_str(&format!(
                "<div class=\"control-group user-hide-profile\" data-setting-name=\"user-hide-profile\">{}</div>",
                checkbox(
                    "pref-hide-profile",
                    Some("user-hide-profile"),
                    p.option("hide_profile") == true,
                    &t(cx, "user.hide_profile"),
                    Some("hide_profile")
                )
            ));
        }
        if u["can_change_bio"] == true {
            out.push_str(&format!(
                "<div class=\"control-group pref-bio\" data-setting-name=\"user-bio\"><label class=\"control-label\">{}</label><div class=\"controls bio-composer input-xxlarge\"><div class=\"d-editor\"><div class=\"d-editor-container --markdown-editor-enabled\"><div class=\"d-editor-textarea-column\"><div class=\"d-editor-textarea-wrapper\"><div class=\"d-overflow-controls d-editor-button-bar__wrap\"><div class=\"d-overflow-controls__content d-editor-button-bar\" role=\"toolbar\" data-d-scroll-axis=\"horizontal\">{}</div></div><textarea autocomplete=\"off\" class=\"d-editor-input\" data-pref=\"bio_raw\">{}</textarea></div></div><div class=\"d-editor-preview-wrapper\"><div class=\"d-editor-preview\">{}</div></div></div></div></div></div>",
                escape(&t(cx, "user.bio")),
                crate::composer_view::toolbar(cx, None, None),
                escape(u["bio_raw"].as_str().unwrap_or_default()),
                u["bio_cooked"].as_str().unwrap_or_default(),
            ));
        }
        let timezone = p.option("timezone").as_str().map(str::to_string);
        out.push_str(&format!(
            "<div class=\"control-group pref-timezone\" data-setting-name=\"user-timezone\" data-moment=\"{}\" data-moment-timezone=\"{}\"><label class=\"control-label\">{}</label>{}{}</div>",
            escape(&pr.moment.0),
            escape(&pr.moment.1),
            escape(&t(cx, "user.timezone")),
            combo(
                cx,
                &Combo {
                    id: None,
                    component_class: "timezone-input",
                    class: "input-xxlarge",
                    rows: timezone.iter().map(|z| (z.clone(), z.clone())).collect(),
                    value: timezone.clone(),
                    none: None,
                    key: Some("timezone"),
                    filterable: true,
                }
            ),
            button(
                "btn-default use-current-timezone",
                Some("globe"),
                Some(&t(cx, "user.use_current_timezone")),
                ""
            )
        ));
        for (allowed, class, id, label, key) in [
            (
                "can_change_location",
                "pref-location",
                "edit-location",
                "user.location",
                "location",
            ),
            (
                "can_change_website",
                "pref-website",
                "edit-website",
                "user.website",
                "website",
            ),
        ] {
            if u[allowed] != true {
                continue;
            }
            out.push_str(&format!(
                "<div class=\"control-group {class}\" data-setting-name=\"user-{key}\"><label class=\"control-label\" for=\"{id}\">{}</label><div class=\"controls\"><input id=\"{id}\" class=\"ember-text-field input-xxlarge\" type=\"text\" value=\"{}\" data-pref=\"{key}\"></div></div>",
                escape(&t(cx, label)),
                escape(u[key].as_str().unwrap_or_default())
            ));
        }
    }

    // userFields: the site's, editable ones unless staff, by position; with
    // the notice, the required ones the user lacks, empty.
    let mut fields: Vec<&Value> = pr.site_user_fields.iter().collect();
    let user_values = &u["user_fields"];
    if enforced {
        fields.retain(|f| {
            f["requirement"] == "for_all_users"
                && !crate::user_fields::present(&user_values[f["id"].to_string()])
        });
    } else {
        if !p.viewer_staff {
            fields.retain(|f| f["editable"] == true);
        }
        fields.sort_by_key(|f| f["position"].as_i64());
    }
    for f in &fields {
        let value = if enforced {
            Value::Null
        } else {
            user_values[f["id"].to_string()].clone()
        };
        out.push_str(&user_field(cx, f, &value)?);
    }
    out.push_str("<div class=\"clearfix\"></div>");

    if !enforced {
        if p.setting("allow_profile_backgrounds")? {
            for (allowed, setting, title, id, kind, key, instructions) in [
                (
                    "can_upload_profile_header",
                    "user-profile-bg",
                    "user.change_profile_background.title",
                    "profile-background-uploader",
                    "profile_background",
                    "profile_background_upload_url",
                    "user.change_profile_background.instructions",
                ),
                (
                    "can_upload_user_card_background",
                    "user-card-bg",
                    "user.change_card_background.title",
                    "profile-card-background-uploader",
                    "card_background",
                    "card_background_upload_url",
                    "user.change_card_background.instructions",
                ),
            ] {
                if u[allowed] != true {
                    continue;
                }
                out.push_str(&format!(
                    "<div class=\"control-group pref-profile-bg\" data-setting-name=\"{setting}\"><label class=\"control-label\">{}</label><div class=\"controls\">{}</div><div class=\"instructions\">{}</div></div>",
                    escape(&t(cx, title)),
                    image_uploader(cx, id, kind, u[key].as_str().filter(|s| !s.is_empty()), key),
                    escape(&t(cx, instructions))
                ));
            }
        }
        if p.setting("allow_featured_topic_on_user_profiles")? {
            out.push_str(&format!(
                "<div class=\"control-group\" data-setting-name=\"user-featured-topic\"><label class=\"control-label\">{}</label>",
                escape(&t(cx, "user.featured_topic"))
            ));
            let featured = &u["featured_topic"];
            if featured.is_object() {
                out.push_str(&format!(
                    "<label class=\"featured-topic-link\"><a href=\"{}/t/{}/{}\">{}</a></label>",
                    cx.base_path,
                    escape(featured["slug"].as_str().unwrap_or_default()),
                    featured["id"],
                    emoji_unescape(
                        featured["fancy_title"].as_str().unwrap_or_default(),
                        &cx.settings.emojis,
                        cx.base_path
                    )
                ));
            }
            out.push_str("<div>");
            out.push_str(&button(
                "btn-default feature-topic-on-profile-btn",
                None,
                Some(&t(cx, "user.feature_topic_on_profile.open_search")),
                " data-unported=\"featuring a topic on the profile (156)\"",
            ));
            if featured.is_object() {
                out.push_str(&button(
                    "btn-danger clear-feature-topic-on-profile-btn",
                    None,
                    Some(&t(cx, "user.feature_topic_on_profile.clear.title")),
                    " data-unported=\"clearing the featured topic (156)\"",
                ));
            }
            out.push_str(&format!(
                "</div><div class=\"instructions\">{}</div></div>",
                escape(&t(cx, "user.change_featured_topic.instructions"))
            ));
        }
        // canChangeDefaultCalendar
        if p.option("default_calendar") != "none_selected" {
            let rows = ["google", "outlook", "apple", "ics"]
                .iter()
                .map(|v| (v.to_string(), t(cx, &format!("download_calendar.{v}"))))
                .collect();
            out.push_str(&format!(
                "<div class=\"control-group\" data-setting-name=\"user-default-calendar\"><label class=\"control-label\">{}</label><div>{}</div><div class=\"instructions\">{}</div></div>",
                escape(&t(cx, "download_calendar.default_calendar")),
                combo(
                    cx,
                    &Combo {
                        id: Some("user-default-calendar"),
                        component_class: "",
                        class: "",
                        rows,
                        value: value_string(p.option("default_calendar")),
                        none: None,
                        key: Some("default_calendar"),
                        filterable: false,
                    }
                ),
                escape(&t(cx, "download_calendar.default_calendar_instruction"))
            ));
        }
    }
    out.push_str(&save_controls(cx));

    let attrs = save_attrs(
        u,
        &[
            "bio_raw",
            "website",
            "location",
            "custom_fields",
            "user_fields",
            "profile_background_upload_url",
            "card_background_upload_url",
            "date_of_birth",
            "timezone",
            "default_calendar",
            "hide_profile",
        ],
        &[],
        &[],
    );
    Ok((out, attrs))
}
