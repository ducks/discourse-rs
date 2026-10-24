// The admin's site settings page, and a config area's settings
// (AdminAreaSettings), in the browser: the filter
// (AdminSiteSettingsFilterControls: the filter after INPUT_DELAY, Clear,
// Only show overridden, the mobile view's category menu), each setting's
// editor and its editing (SiteSetting: the save and cancel controls while
// its value is changed, undo to the default, a setting hidden by a
// dependency shown once the dependency is met) and the changes banner
// that saves or discards every changed setting. A setting saves through
// PUT /admin/site_settings/:name, the banner through bulk_update,
// confirming first when one requires it. The route's filters are the
// URL's query; the page for them is swapped in as a link's is. The list
// settings' editors: ListSetting's MultiSelect (compact, file types, host
// and group lists), ValueList (list, url and value lists), the category
// list's CategorySelector and the tag list's TagChooser. Not ported, and
// saying so when used: the editors the page says so of, the other
// pickers' dropdowns, uploading, the emoji picker and the backfill question.
(function () {
  "use strict";

  if (!Discourse.once("site-settings")) {
    return;
  }

  var INPUT_DELAY = 250;
  var timer = null;
  var strings = {};

  // Saving one asks whether to update the existing users.
  var DEFAULT_USER_PREFERENCES = [
    "default_email_digest_frequency",
    "default_include_tl0_in_digests",
    "default_email_level",
    "default_email_messages_level",
    "default_email_mailing_list_mode",
    "default_email_mailing_list_mode_frequency",
    "default_email_previous_replies",
    "default_email_in_reply_to",
    "default_hide_profile",
    "default_hide_presence",
    "default_other_new_topic_duration_minutes",
    "default_other_auto_track_topics_after_msecs",
    "default_other_notification_level_when_replying",
    "default_other_external_links_in_new_tab",
    "default_other_enable_quoting",
    "default_other_enable_smart_lists",
    "default_other_dynamic_favicon",
    "default_other_like_notification_frequency",
    "default_other_skip_new_user_tips",
    "default_other_enable_markdown_monospace_font",
    "default_topics_automatic_unpin",
    "default_categories_watching",
    "default_categories_tracking",
    "default_categories_muted",
    "default_categories_watching_first_post",
    "default_categories_normal",
    "default_tags_watching",
    "default_tags_tracking",
    "default_tags_muted",
    "default_tags_watching_first_post",
    "default_text_size",
    "default_title_count_mode",
    "default_navigation_menu_categories",
    "default_navigation_menu_tags",
    "default_sidebar_link_to_filtered_list",
    "default_sidebar_show_count_of_new_items",
    "default_composition_mode",
    "default_watched_precedence_over_muted",
  ];

  function t(key, args) {
    var s = strings[key];
    if (s === undefined) {
      return "[en." + key + "]";
    }
    Object.keys(args || {}).forEach(function (name) {
      s = s.split("%{" + name + "}").join(String(args[name]));
    });
    return s;
  }

  function tCount(key, count) {
    return t(key + "." + (count === 1 ? "one" : "other"), { count: count });
  }

  // The site settings page, or a config area's settings
  // (AdminAreaSettings): the admin contents holding the filter controls.
  function page() {
    var controls = document.querySelector(".admin-site-settings-filter-controls");
    if (controls) {
      return controls.closest(".admin-contents");
    }
    // A theme's page: its settings, site settings, translations and
    // relatives, each saved by the page's saver (Discourse.settingSavers).
    return document.querySelector(".admin-contents .admin-customize-themes-show");
  }

  // A setting's row; a theme translation's has no setting name.
  var ROW = ".row.setting[data-setting], .row.setting[data-translation-key]";

  function base() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function csrf() {
    var meta = document.querySelector("meta[name='csrf-token']");
    return meta ? meta.content : "";
  }

  function unsupported(what) {
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = "Not ported yet: " + what + ".";
      error.hidden = false;
    }
  }

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  // The page for the route's query, swapped in as a link's is.
  function go(params, keepFocus) {
    var query = params.toString();
    var url = location.pathname + (query ? "?" + query : "");
    if (!window.htmx) {
      location.replace(url);
      return;
    }
    history.replaceState(history.state, "", url);
    Discourse.siteSettingsRefocus = !!keepFocus;
    Discourse.siteSettingsFromControls = true;
    var link = document.createElement("a");
    link.href = url;
    link.hidden = true;
    link.setAttribute("hx-push-url", "false");
    document.getElementById("main-outlet").appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  // The setting's value as the editor holds it, as the server takes it.
  function currentValue(row) {
    var type = row.dataset.type;
    var checkbox = row.querySelector(
      ".setting-value input.form-kit__control-checkbox, .setting-value > label.checkbox-label > input[type='checkbox']"
    );
    if (checkbox) {
      return checkbox.checked ? "true" : "false";
    }
    var select = row.querySelector(".setting-value select");
    if (select) {
      return select.value === "__NONE__" ? "" : select.value;
    }
    var list = row.querySelector(".simple-list, .emoji-list");
    if (list) {
      return listValues(list)
        .filter(function (v) {
          return v !== "";
        })
        .join("|");
    }
    var combo = row.querySelector(".setting-value details.combo-box[data-options]:not(.file-size-unit-selector)");
    if (combo) {
      return combo.querySelector("summary").dataset.value || "";
    }
    var uploader = row.querySelector(".setting-value .file-uploader");
    if (uploader) {
      return uploader.dataset.value !== undefined ? uploader.dataset.value : row.dataset.value;
    }
    var picker = row.querySelector(".setting-value .file-size-picker");
    if (picker) {
      return picker.dataset.kb !== undefined ? picker.dataset.kb : row.dataset.value;
    }
    var color = row.querySelector(".setting-value .color-picker");
    if (color) {
      return color.dataset.value !== undefined ? color.dataset.value : row.dataset.value;
    }
    var chooser = row.querySelector(".setting-value details.multi-select[data-list-value]");
    if (chooser) {
      return chooser.dataset.listValue;
    }
    var valueList = row.querySelector(".setting-value .value-list[data-delimiter]");
    if (valueList) {
      return valueListValues(valueList)
        .filter(function (v) {
          return v !== "";
        })
        .join(valueList.dataset.delimiter);
    }
    var input = row.querySelector(
      ".setting-value input.form-kit__control-input, .setting-value input.form-kit__control-password, .setting-value textarea, .setting-value input.input-setting-string, .setting-value input.input-setting-integer"
    );
    if (input) {
      var value = input.value;
      if (type === "integer" && value !== "") {
        value = String(Math.trunc(Number(value)));
      }
      return value;
    }
    return row.dataset.value;
  }

  function setValue(row, value) {
    var checkbox = row.querySelector(
      ".setting-value input.form-kit__control-checkbox, .setting-value > label.checkbox-label > input[type='checkbox']"
    );
    if (checkbox) {
      checkbox.checked = value === "true";
      return;
    }
    var select = row.querySelector(".setting-value select");
    if (select) {
      select.value = value === "" && select.querySelector("option[value='__NONE__']") ? "__NONE__" : value;
      return;
    }
    var list = row.querySelector(".simple-list, .emoji-list");
    if (list) {
      renderList(list, value.split("|").filter(Boolean));
      return;
    }
    var uploader = row.querySelector(".setting-value .file-uploader");
    if (uploader) {
      setUpload(row, uploader, value);
      return;
    }
    var chooser = row.querySelector(".setting-value details.multi-select[data-list-value]");
    if (chooser) {
      setChooser(chooser, value);
      return;
    }
    var valueList = row.querySelector(".setting-value .value-list[data-delimiter]");
    if (valueList) {
      renderList(
        valueList,
        value.split(valueList.dataset.delimiter).filter(Boolean)
      );
      return;
    }
    var colorPicker = row.querySelector(".setting-value .color-picker");
    if (colorPicker) {
      colorPicker.dataset.value = value;
      colorPicker.querySelector(".hex-input").value = value;
      syncColorPicker(colorPicker, value);
      return;
    }
    var picker = row.querySelector(".setting-value .file-size-picker");
    if (picker) {
      picker.dataset.kb = value;
      var kb = Number(value);
      var unit = kb >= 1024 * 1024 ? "gb" : kb >= 1024 ? "mb" : "kb";
      var unitCombo = picker.querySelector(".file-size-unit-selector");
      var unitOption = JSON.parse(unitCombo.dataset.options).filter(function (o) {
        return o.id === unit;
      })[0];
      setCombo(unitCombo, unit, unitOption.name);
      showFileSize(picker);
      return;
    }
    var combo = row.querySelector(".setting-value details.combo-box[data-options]:not(.file-size-unit-selector)");
    if (combo) {
      var options = JSON.parse(combo.dataset.options);
      var found = options.filter(function (o) {
        return String(o.id) === value;
      })[0];
      setCombo(combo, found ? String(found.id) : "", found ? found.name : "");
      return;
    }
    var input = row.querySelector(
      ".setting-value input.form-kit__control-input, .setting-value input.form-kit__control-password, .setting-value textarea, .setting-value input.input-setting-string, .setting-value input.input-setting-integer"
    );
    if (input) {
      input.value = value;
    }
  }

  function dirtyRows() {
    var root = page();
    if (!root) {
      return [];
    }
    return Array.prototype.filter.call(root.querySelectorAll(".row.setting[data-setting]"), function (row) {
      // A row saved on its own (trackChanges off) is not the banner's.
      return row.classList.contains("--dirty") && !row.dataset.saver;
    });
  }

  // The row's controls: save and cancel while changed, undo while
  // overridden, nothing otherwise.
  function syncRow(row) {
    var value = currentValue(row);
    // SiteSetting#preview with the buffered value.
    var preview = row.querySelector(".setting-value > .preview[data-template]");
    if (preview) {
      preview.innerHTML = preview.dataset.template.replace(/\{\{value\}\}/g, value);
    }
    var dirty = value !== row.dataset.value;
    var overridden = value !== row.dataset.default;
    row.classList.toggle("--dirty", dirty);
    row.classList.toggle("overridden", row.dataset.value !== row.dataset.default);
    var controls = row.querySelector(":scope > .setting-controls");
    var undo = row.querySelector(":scope > .setting-controls__undo");
    if (dirty) {
      if (undo) {
        undo.remove();
      }
      if (!controls) {
        controls = document.createElement("div");
        controls.className = "setting-controls";
        controls.innerHTML =
          '<button aria-label="' +
          t("admin.settings.save") +
          '" class="btn no-text btn-icon ok setting-controls__ok" type="button">' +
          icon("check") +
          '<span aria-hidden="true"> &#8203; </span></button><button aria-label="' +
          t("admin.settings.cancel") +
          '" class="btn no-text btn-icon cancel setting-controls__cancel" type="button">' +
          icon("xmark") +
          '<span aria-hidden="true"> &#8203; </span></button>';
        row.appendChild(controls);
      }
    } else {
      if (controls) {
        controls.remove();
      }
      if (overridden && !undo && !row.classList.contains("disabled")) {
        undo = document.createElement("button");
        undo.className = "btn btn-icon-text btn-default undo setting-controls__undo";
        undo.type = "button";
        undo.innerHTML = icon("arrow-rotate-left") + '<span class="d-button-label">' + t("admin.settings.reset") + "</span>";
        row.appendChild(undo);
      } else if (!overridden && undo) {
        undo.remove();
      }
    }
    syncDependents(row);
    syncBanner();
  }

  // AdminSiteSettingStore: a setting hidden behind others shows while each
  // of them is on.
  function syncDependents(row) {
    var name = row.dataset.setting;
    var root = page();
    root.querySelectorAll(".row.setting[data-depends-on]").forEach(function (dependent) {
      var parents = dependent.dataset.dependsOn.split(",");
      if (parents.indexOf(name) === -1) {
        return;
      }
      var met = parents.every(function (parentName) {
        var parent = root.querySelector('.row.setting[data-setting="' + parentName + '"]');
        return !parent || currentValue(parent) === "true";
      });
      dependent.hidden = !met;
    });
  }

  // AdminSiteSettingsChangesBanner
  function syncBanner() {
    var root = page();
    if (!root) {
      return;
    }
    var count = dirtyRows().length;
    var banner = document.querySelector(".admin-changes-banner");
    document.body.classList.toggle("has-admin-changes-banner", count > 0);
    if (count === 0) {
      if (banner) {
        banner.remove();
      }
      return;
    }
    if (!banner) {
      banner = document.createElement("div");
      banner.className = "admin-changes-banner";
      root.appendChild(banner);
    }
    // Only on a new count: a button rebuilt between mousedown and mouseup
    // (the input's change event) loses its click.
    if (banner.dataset.count === String(count)) {
      return;
    }
    banner.dataset.count = count;
    banner.innerHTML =
      '<span class="admin-changes-banner__main-label">' +
      tCount("admin.site_settings.dirty_banner", count) +
      '</span><div class="controls"><button class="btn btn-text btn-default btn-small banner-discard" type="button"><span class="d-button-label">' +
      tCount("admin.site_settings.discard", count) +
      '</span></button><button class="btn btn-text btn-primary btn-small banner-save" type="button"><span class="d-button-label">' +
      tCount("admin.site_settings.save", count) +
      "</span></button></div>";
  }

  function showError(row, message) {
    var error = row.querySelector(".validation-error");
    if (!error) {
      return;
    }
    error.classList.toggle("hidden", !message);
    var svg = error.querySelector("svg");
    error.textContent = "";
    if (svg) {
      error.appendChild(svg);
    }
    if (message) {
      error.appendChild(document.createTextNode(" " + message));
    }
  }

  function needsConfirmation(row, value) {
    switch (row.dataset.requiresConfirmation) {
      case "simple":
        return true;
      case "simple_on_enable":
        return value === "true";
      case "simple_on_disable":
        return value === "false";
      default:
        return false;
    }
  }

  // SiteSettingChangeTracker#confirmChanges
  function confirm(row) {
    var name = row.dataset.setting;
    return new Promise(function (resolve) {
      Discourse.dialog.confirm({
        message: strings["confirm." + name + ".prompt"],
        confirmLabel: strings["confirm." + name + ".confirm"],
        cancelLabel: strings.no_value,
        didConfirm: function () {
          resolve(true);
        },
        didCancel: function () {
          resolve(false);
        },
      });
    });
  }

  function put(path, body) {
    return fetch(base() + path, {
      method: "PUT",
      credentials: "same-origin",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        "X-Requested-With": "XMLHttpRequest",
        "X-CSRF-Token": csrf(),
      },
      body: body.toString(),
    }).then(function (response) {
      if (response.ok) {
        return response.status === 204 ? null : response.json();
      }
      return response.text().then(function (text) {
        var message = strings.generic_error;
        try {
          var json = JSON.parse(text);
          if (json.errors) {
            message = json.errors[0];
          }
        } catch (_) {}
        throw new Error(message);
      });
    });
  }

  // SiteSettingChangeTracker#configureBackfill: a default user preference
  // asks whether to update the users it would change, if any.
  function configureBackfill(row) {
    var name = row.dataset.setting;
    var body = new URLSearchParams();
    body.set(name, currentValue(row));
    return put("/admin/site_settings/" + encodeURIComponent(name) + "/user_count.json", body).then(function (result) {
      if (result && result.user_count > 0) {
        throw new Error("Not ported yet: the question whether to update existing users.");
      }
    });
  }

  // SiteSetting#update for one setting, bulkUpdate for several or from the
  // banner, after the confirmations and backfill questions.
  function saveRows(rows, bulk) {
    var values = rows.map(currentValue);
    var chain = Promise.resolve(true);
    rows.forEach(function (row, i) {
      if (needsConfirmation(row, values[i])) {
        chain = chain.then(function (ok) {
          return ok ? confirm(row) : false;
        });
      }
    });
    rows.forEach(function (row) {
      if (DEFAULT_USER_PREFERENCES.indexOf(row.dataset.setting) !== -1) {
        chain = chain.then(function (ok) {
          return ok ? configureBackfill(row).then(function () { return true; }) : false;
        });
      }
    });
    return chain
      .then(function (ok) {
        if (!ok) {
          return false;
        }
        var saver = rows.length === 1 && rows[0].dataset.saver && (Discourse.settingSavers || {})[rows[0].dataset.saver];
        if (saver) {
          return saver(rows[0], values[0]).then(function () {
            rows[0].dataset.value = values[0];
            showError(rows[0], null);
            syncRow(rows[0]);
            return true;
          });
        }
        var body = new URLSearchParams();
        var path;
        if (rows.length === 1 && !bulk) {
          path = "/admin/site_settings/" + encodeURIComponent(rows[0].dataset.setting);
          body.set(rows[0].dataset.setting, values[0]);
        } else {
          path = "/admin/site_settings/bulk_update.json";
          rows.forEach(function (row, i) {
            body.set("settings[" + row.dataset.setting + "][value]", values[i]);
            body.set("settings[" + row.dataset.setting + "][backfill]", "false");
          });
        }
        return put(path, body).then(function () {
          rows.forEach(function (row, i) {
            row.dataset.value = values[i];
            showError(row, null);
            syncRow(row);
          });
          return true;
        });
      })
      .catch(function (e) {
        if (bulk) {
          Discourse.dialog.alert(e.message);
        } else {
          rows.forEach(function (row) {
            showError(row, e.message);
          });
        }
        return false;
      });
  }

  function save(row) {
    return saveRows([row], false);
  }

  function cancel(row) {
    setValue(row, row.dataset.value);
    showError(row, null);
    syncRow(row);
  }

  // SimpleList: the values, each with remove and, with more than one,
  // shift buttons; a predefined list's ComboBox offers the choices left.
  function listValues(list) {
    if (list.classList.contains("emoji-list")) {
      // A fixed entry the value lacks (data-added) is not part of it.
      return Array.prototype.map.call(list.querySelectorAll(".values .value:not([data-added]) .emoji-name"), function (name) {
        return name.textContent;
      });
    }
    return Array.prototype.map.call(list.querySelectorAll(".values input.value-input"), function (input) {
      return input.value;
    });
  }

  // EmojiValueList's list, its images from the page's data-emoji-urls.
  function renderEmojiList(list, values) {
    var urls = JSON.parse(list.dataset.emojiUrls || "{}");
    var old = list.querySelector(".values");
    if (old) {
      old.remove();
    }
    // data-fixed (discourse-reactions' like reaction): first, not
    // editable, added when the value lacks it.
    var fixed = list.dataset.fixed;
    var added = fixed && values.indexOf(fixed) === -1;
    var collection = added ? [fixed].concat(values) : values;
    if (!collection.length) {
      return;
    }
    var ul = document.createElement("ul");
    ul.className = "values emoji-value-list";
    collection.forEach(function (value, index) {
      var item = document.createElement("li");
      var editable = value !== fixed;
      item.className = "value";
      item.dataset.index = index;
      if (added && !editable) {
        item.dataset.added = "";
      }
      item.innerHTML =
        smallButton("remove-value-btn", "xmark").replace(' type="button"', editable ? ' type="button"' : ' disabled="" type="button"') +
        ' <div class="value-input emoji-details' +
        (editable ? " can-edit" : "") +
        '" role="button"> <img class="emoji-list-emoji" height="15px" width="15px"> <span class="emoji-name"></span> </div>' +
        (collection.length > 1
          ? " " + smallButton("shift-up-value-btn", "arrow-up") + " " + smallButton("shift-down-value-btn", "arrow-down")
          : "");
      item.querySelector("img").src = urls[value] || "";
      item.querySelector(".emoji-name").textContent = value;
      ul.appendChild(item);
    });
    list.insertBefore(ul, list.firstChild);
  }

  function smallButton(cls, name) {
    return (
      '<button class="btn no-text btn-icon btn-default ' +
      cls +
      ' btn-small" type="button">' +
      icon(name) +
      '<span aria-hidden="true"> &#8203; </span></button>'
    );
  }

  function renderList(list, values) {
    if (list.classList.contains("emoji-list")) {
      renderEmojiList(list, values);
      return;
    }
    var old = list.querySelector(".values");
    if (old) {
      old.remove();
    }
    if (values.length) {
      var div = document.createElement("div");
      div.className = "values";
      values.forEach(function (value, index) {
        var item = document.createElement("div");
        item.className = "value";
        item.dataset.index = index;
        item.innerHTML =
          smallButton("remove-value-btn", "xmark") +
          ' <input class="value-input" type="text">' +
          (values.length > 1
            ? " " + smallButton("shift-up-value-btn", "arrow-up") + " " + smallButton("shift-down-value-btn", "arrow-down")
            : "");
        var input = item.querySelector("input");
        input.value = value;
        input.title = value;
        div.appendChild(item);
      });
      list.insertBefore(div, list.firstChild);
    }
    var holder = list.querySelector(".simple-list-input[data-choices]");
    if (!holder) {
      return;
    }
    var combo = holder.querySelector("details.add-value-input");
    var left = validChoices(holder, values).length > 0;
    if (combo && !left) {
      combo.remove();
    } else if (!combo && left) {
      var label = strings["select_kit.select_to_filter"] || "";
      holder.innerHTML =
        '<details class="select-kit single-select combobox combo-box add-value-input"><summary aria-label="' +
        label +
        '" name="' +
        label +
        '" data-name="" data-value="" tabindex="0" class="select-kit-header single-select-header combo-box-header"><div class="select-kit-header-wrapper"><div class="select-kit-selected-name selected-name choice" title=""><span class="name"></span></div>' +
        icon("angle-down").replace("svg-string", "angle-icon svg-string") +
        '</div></summary><div class="select-kit-body"></div></details>';
      holder.firstChild.dataset.options = holder.dataset.choices;
    }
  }

  function validChoices(holder, values) {
    return JSON.parse(holder.dataset.choices).filter(function (o) {
      return values.indexOf(String(o.id)) === -1;
    });
  }

  function addListValue(list, value) {
    renderList(list, listValues(list).concat([value]));
  }

  // ColorInput#normalize: a hex color gets its # and six digits.
  function isValidHex(color) {
    return /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.test(color || "");
  }

  function normalizeColor(color) {
    if (isValidHex(color)) {
      if (color.charAt(0) !== "#") {
        color = "#" + color;
      }
      if (color.length === 4) {
        color =
          "#" +
          color
            .slice(1)
            .split("")
            .map(function (hex) {
              return hex + hex;
            })
            .join("");
      }
    }
    return color;
  }

  function syncColorPicker(colorPicker, value) {
    var picker = colorPicker.querySelector(".picker");
    var shown = value;
    if (!shown && colorPicker.hasAttribute("data-fallback-primary")) {
      shown = getComputedStyle(document.documentElement).getPropertyValue("--primary").trim();
    }
    shown = normalizeColor(shown);
    picker.title = shown;
    if (isValidHex(shown)) {
      picker.value = shown;
    }
  }

  // The MultiSelects of the list settings (data-list-value, data-choices
  // as the page drew them): ListSetting for the compact, file types, host
  // and group lists, CategorySelector for a category list and TagChooser
  // for a tag list (data-tag-search, its rows searched for). Open, the body
  // has the filter, each chosen value (SelectedChoice, its button removing
  // it; a mandatory one read only) and the content not chosen that the
  // filter matches, led by the typed value to create when allowAny.
  function esc(s) {
    return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/"/g, "&quot;");
  }

  function chooserValues(details) {
    return (details.dataset.listValue || "").split("|").filter(Boolean);
  }

  // The content as {id, name, html}: what the page drew, plus the values
  // created or removed since (onChangeChoices) and the tags found.
  function chooserContent(details) {
    var content = JSON.parse(details.dataset.choices || "[]").map(function (c) {
      return typeof c === "object" ? { id: String(c.id), name: String(c.name), html: c.html } : { id: String(c), name: String(c) };
    });
    (details.extraChoices || []).forEach(function (c) {
      if (!content.some(function (o) { return o.id === c.id; })) {
        content.push(c);
      }
    });
    return content;
  }

  function choiceNamed(details, id) {
    var found = chooserContent(details).filter(function (c) {
      return c.id === id;
    })[0];
    return found || { id: id, name: id };
  }

  function addExtraChoice(details, choice) {
    details.extraChoices = (details.extraChoices || []).filter(function (c) {
      return c.id !== choice.id;
    });
    details.extraChoices.push(choice);
  }

  // The header names the values; the body follows when open.
  function setChooser(details, value) {
    details.dataset.listValue = value;
    var values = chooserValues(details);
    var names = values.map(function (v) {
      return choiceNamed(details, v).name;
    });
    var summary = details.querySelector("summary");
    summary.dataset.value = values.join(",");
    summary.dataset.name = names.join(",");
    details.classList.toggle("has-selection", values.length > 0);
    details.querySelector(".formatted-selection").textContent = values.length
      ? names.join(", ")
      : strings["select_kit.default_header_text"] || "";
    if (details.classList.contains("is-expanded")) {
      drawChooser(details);
    }
  }

  function openChooser(details) {
    details.classList.add("is-expanded");
    details.open = true;
    drawChooser(details);
    var input = details.querySelector(".select-kit-body .filter-input");
    if (input) {
      input.focus();
    }
  }

  function closeChooser(details) {
    details.classList.remove("is-expanded");
    details.open = false;
    details.querySelector(".select-kit-body").innerHTML = "";
  }

  function chooserRow(choice, index, highlighted, label) {
    var tag = choice.tag === true;
    return (
      '<li aria-checked="false" role="menuitemradio" data-index="' +
      index +
      '" data-name="' +
      esc(choice.name) +
      '" data-value="' +
      esc(choice.id) +
      '" title="' +
      esc(choice.name) +
      '" tabindex="0" class="select-kit-row' +
      (tag ? " tag-chooser-row" : choice.html ? " category-row" : "") +
      (highlighted ? " is-highlighted" : "") +
      '">' +
      (tag
        ? '<a data-tag-name="' + esc(choice.name) + '" class="discourse-tag simple">' + esc(choice.name) + "</a>" +
          (choice.count ? ' <span class="discourse-tag-count">x' + esc(choice.count) + "</span>" : "")
        : choice.html
          ? '<div class="category-status">' + choice.html + "</div>"
          : '<span class="name">' + esc(label || choice.name) + "</span>") +
      "</li>"
    );
  }

  function drawChooser(details) {
    var body = details.querySelector(".select-kit-body");
    var old = body.querySelector(".filter-input");
    var term = old ? old.value : "";
    var hadFocus = old && document.activeElement === old;
    var values = chooserValues(details);
    var mandatory = (details.dataset.mandatory || "").split("|").filter(Boolean);
    var html =
      '<div class="select-kit-filter multi-select-filter is-expanded"><input class="filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
      esc(details.dataset.placeholder || "") +
      '" spellcheck="false" tabindex="0" type="search"></div>';
    if (values.length) {
      html +=
        '<div class="selected-content">' +
        values
          .map(function (v) {
            var choice = choiceNamed(details, v);
            if (mandatory.indexOf(v) !== -1) {
              return (
                '<button class="btn btn-default disabled tag-choice" title="' +
                esc(strings["admin.site_settings.mandatory_group"] || "") +
                '">' +
                esc(choice.name) +
                "</button>"
              );
            }
            return (
              '<button aria-label="' +
              esc(t("select_kit.delete_item", { name: choice.name })) +
              '" class="btn btn-default selected-choice tag-choice " data-name="' +
              esc(choice.name) +
              '" data-value="' +
              esc(v) +
              '" type="button">' +
              icon("xmark") +
              (choice.html ? choice.html : '<span class="d-button-label">' + esc(choice.name) + "</span>") +
              "</button>"
            );
          })
          .join("") +
        "</div>";
    }
    body.innerHTML = html + '<ul aria-live="polite" class="select-kit-collection" role="menu"></ul>';
    var input = body.querySelector(".filter-input");
    input.value = term;
    if (hadFocus) {
      input.focus();
    }
    if (details.dataset.tagSearch) {
      searchTags(details, term);
    } else {
      drawRows(details, rowsFor(details, term), term);
    }
  }

  // The content not chosen that the filter matches, the typed value first
  // when it can be created.
  function rowsFor(details, term) {
    var values = chooserValues(details);
    var needle = term.trim().toLowerCase();
    var content = chooserContent(details);
    var rows = content.filter(function (c) {
      return values.indexOf(c.id) === -1 && (!needle || c.name.toLowerCase().indexOf(needle) !== -1);
    });
    var typed = term.trim();
    var exists = content.some(function (c) {
      return c.id === typed;
    });
    if (details.dataset.allowAny === "true" && typed && !exists && values.indexOf(typed) === -1) {
      rows.unshift({ id: typed, name: typed, create: true });
    }
    return rows;
  }

  function drawRows(details, rows, term) {
    var collection = details.querySelector(".select-kit-collection");
    if (!collection) {
      return;
    }
    collection.innerHTML = rows
      .map(function (c, i) {
        return chooserRow(c, i, term.trim() !== "" && i === 0, c.create ? t("select_kit.create", { content: c.name }) : null);
      })
      .join("");
    var count = details.querySelector(".select-kit-body .results-count");
    if (count) {
      count.remove();
    }
    if (term.trim()) {
      collection.insertAdjacentHTML(
        "afterend",
        '<span class="results-count" role="alert">' + esc(tCountName("select_kit.results_count", rows.length)) + "</span>"
      );
    }
  }

  function tCountName(key, count) {
    return t(key + "." + (count === 1 ? "one" : "other"), { count: count });
  }

  // TagChooser's search (everyTag, the chosen tags left out).
  function searchTags(details, term) {
    var params = new URLSearchParams({ q: term, limit: details.dataset.tagSearch, filterForInput: "true" });
    chooserValues(details).forEach(function (name) {
      params.append("selected_tags[]", name);
    });
    fetch(base() + "/tags/filter/search.json?" + params.toString(), {
      credentials: "same-origin",
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
    })
      .then(function (response) {
        return response.ok ? response.json() : { results: [] };
      })
      .then(function (json) {
        var input = details.querySelector(".select-kit-body .filter-input");
        if (!input || input.value !== term) {
          return;
        }
        var values = chooserValues(details);
        var rows = (json.results || [])
          .filter(function (r) {
            return values.indexOf(r.name) === -1;
          })
          .map(function (r) {
            return { id: r.name, name: r.name, count: r.count, tag: true };
          });
        drawRows(details, rows, term);
      });
  }

  // A chosen row or removed value: the new value through the setting's
  // component (HostList refusing wildcards, GroupList storing everyone
  // back while the granular permissions keep it).
  function changeChooser(details, values) {
    var refuse = details.dataset.refuse || "";
    if (
      refuse &&
      values.some(function (v) {
        return refuse.split("").some(function (ch) {
          return v.indexOf(ch) !== -1;
        });
      })
    ) {
      return;
    }
    if (details.dataset.granular === "true") {
      var stored = (details.dataset.stored || "").split("|").filter(Boolean);
      if (stored.indexOf("0") !== -1 && stored.indexOf("5") === -1) {
        values = values
          .map(function (v) {
            return v === "5" ? "0" : v;
          })
          .filter(function (v, i, all) {
            return all.indexOf(v) === i;
          });
      }
    }
    setChooser(details, values.join("|"));
    var row = details.closest(ROW);
    if (row) {
      syncRow(row);
    }
  }

  function pickRow(details, rowEl) {
    var value = rowEl.dataset.value;
    var choice = { id: value, name: rowEl.dataset.name };
    if (rowEl.classList.contains("tag-chooser-row")) {
      choice.tag = true;
    }
    addExtraChoice(details, choice);
    var input = details.querySelector(".select-kit-body .filter-input");
    if (input) {
      input.value = "";
    }
    changeChooser(details, chooserValues(details).concat([value]));
  }

  // ValueList: the values (its delimiter's), each editable, removable and
  // shifted by renderList's buttons, and the ComboBox adding a choice or
  // the typed value.
  function valueListValues(list) {
    return Array.prototype.map.call(list.querySelectorAll(":scope > .values input.value-input"), function (input) {
      return input.value;
    });
  }

  function drawValueListAdder(details) {
    var list = details.closest(".value-list");
    var body = details.querySelector(".select-kit-body");
    var old = body.querySelector(".filter-input");
    var term = old ? old.value : "";
    var values = valueListValues(list);
    var needle = term.trim().toLowerCase();
    var rows = JSON.parse(list.dataset.choices || "[]")
      .map(String)
      .filter(function (c) {
        return values.indexOf(c) === -1 && (!needle || c.toLowerCase().indexOf(needle) !== -1);
      })
      .map(function (c) {
        return { id: c, name: c };
      });
    if (term.trim() && !rows.some(function (r) { return r.id === term.trim(); })) {
      rows.unshift({ id: term.trim(), name: term.trim(), create: true });
    }
    body.innerHTML =
      '<div class="select-kit-filter is-expanded"><input class="filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
      esc(list.dataset.placeholder || "") +
      '" spellcheck="false" tabindex="0" type="search">' +
      icon("magnifying-glass").replace("svg-string", "filter-icon svg-string") +
      "</div>" +
      (rows.length
        ? '<ul aria-live="polite" class="select-kit-collection" role="menu">' +
          rows
            .map(function (c, i) {
              return chooserRow(c, i, needle !== "" && i === 0, c.create ? t("select_kit.create", { content: c.name }) : null);
            })
            .join("") +
          "</ul>"
        : "") +
      (needle ? '<span class="results-count" role="alert">' + esc(tCountName("select_kit.results_count", rows.length)) + "</span>" : "");
    var input = body.querySelector(".filter-input");
    input.value = term;
    input.focus();
  }

  function closeValueListAdder(details) {
    details.classList.remove("is-expanded");
    details.open = false;
    details.querySelector(".select-kit-body").innerHTML = "";
  }

  // ValueList#addValue: a value not listed yet.
  function addValueListValue(details, value) {
    var list = details.closest(".value-list");
    var values = valueListValues(list);
    if (value && values.indexOf(value) === -1) {
      values.push(value);
    }
    closeValueListAdder(details);
    renderList(list, values);
    syncRow(list.closest(ROW));
  }

  // FileTypesList#insertDefaultTypes, with its toast.
  function insertDefaultTypes(row, button) {
    var oldTypes = currentValue(row).split("|");
    var newTypes = [];
    oldTypes.concat(button.dataset.types.split("|")).forEach(function (type) {
      if (newTypes.indexOf(type) === -1) {
        newTypes.push(type);
      }
    });
    var diff = newTypes.filter(function (type) {
      return oldTypes.indexOf(type) === -1;
    });
    if (!diff.length) {
      return;
    }
    toast(t("admin.site_settings.file_types_list.add_types_toast", { types: diff.join(", ") }));
    setChooser(row.querySelector(".setting-value details.list-setting"), newTypes.join("|"));
    syncRow(row);
  }

  // toasts.success: DDefaultToast for three seconds.
  function toast(text) {
    var section = document.querySelector("section.fk-d-toasts");
    if (!section) {
      section = document.createElement("section");
      section.className = "fk-d-toasts";
      document.body.appendChild(section);
    }
    var output = document.createElement("output");
    output.className = "fk-d-toast";
    output.setAttribute("role", "status");
    output.innerHTML =
      '<div class="fk-d-default-toast -success"><div class="fk-d-default-toast__icon-container">' +
      icon("check") +
      '</div><div class="fk-d-default-toast__main-container"><div class="fk-d-default-toast__texts"><div class="fk-d-default-toast__message"></div></div></div><div class="fk-d-default-toast__close-container"><button class="btn no-text btn-icon btn-transparent" type="button">' +
      icon("xmark") +
      '<span aria-hidden="true">&#8203;</span></button></div></div>';
    output.querySelector(".fk-d-default-toast__message").textContent = text;
    section.appendChild(output);
    var close = function () {
      output.remove();
    };
    output.querySelector("button").addEventListener("click", close);
    setTimeout(close, 3000);
  }

  // FileSizeInput: the size in KB shown in the chosen unit.
  var FILE_SIZE_FACTORS = { kb: 1, mb: 1024, gb: 1024 * 1024 };

  function fileSizeUnit(picker) {
    return picker.querySelector(".file-size-unit-selector summary").dataset.value;
  }

  function showFileSize(picker) {
    var row = picker.closest(".row.setting");
    var kb = picker.dataset.kb !== undefined ? picker.dataset.kb : row.dataset.value;
    var input = picker.querySelector(".file-size-input");
    input.value = Number(kb) ? String(Number(kb) / FILE_SIZE_FACTORS[fileSizeUnit(picker)]) : "";
  }

  function fileSizeInput(picker) {
    var value = parseFloat(picker.querySelector(".file-size-input").value);
    picker.dataset.kb = isNaN(value) ? "" : String(value * FILE_SIZE_FACTORS[fileSizeUnit(picker)]);
  }

  // SiteSettingUpload's states: the saved file as the page drew it, no
  // file (Delete), or another path (undo to the default) previewed by URL.
  function setUpload(row, uploader, value) {
    if (uploader.originalHtml === undefined) {
      uploader.originalHtml = uploader.innerHTML;
      uploader.originalClass = uploader.className;
    }
    uploader.innerHTML = uploader.originalHtml;
    uploader.className = uploader.originalClass;
    if (value === row.dataset.value) {
      delete uploader.dataset.value;
      return;
    }
    uploader.dataset.value = value;
    var preview = uploader.querySelector(".file-uploader__preview");
    if (value === "") {
      var controls = uploader.querySelector(":scope > .file-uploader__controls");
      var label = controls ? controls.querySelector("label") : null;
      uploader.className = "file-uploader no-file";
      preview.removeAttribute("style");
      preview.innerHTML = "";
      if (label) {
        label.className = "btn btn-transparent";
        label.lastChild.textContent = " " + (strings["upload_selector.select_file"] || "");
        var holder = document.createElement("div");
        holder.className = "file-uploader__controls";
        holder.appendChild(label);
        preview.appendChild(holder);
      }
      if (controls) {
        controls.remove();
      }
      return;
    }
    preview.style.backgroundImage = "url(" + value + ")";
    var link = preview.querySelector("a.lightbox");
    if (link) {
      link.href = value;
      link.title = value.split("/").pop();
      link.querySelector(".informations").textContent = "";
    }
  }

  function setCombo(details, value, name) {
    var summary = details.querySelector("summary");
    summary.dataset.value = value;
    summary.dataset.name = name;
    details.classList.toggle("has-selection", value !== "");
    var label = details.querySelector(".select-kit-header-wrapper .name");
    label.textContent = name;
    // The font selector names each font in its own class.
    var option = JSON.parse(details.dataset.options).filter(function (o) {
      return String(o.id) === value;
    })[0];
    if (option && option.classNames) {
      label.className = "name " + option.classNames;
    }
  }

  function closeCombo(details) {
    details.classList.remove("is-expanded");
    details.open = false;
    details.querySelector(".select-kit-body").innerHTML = "";
  }

  Discourse.onPage(function () {
    var json = document.getElementById("site-settings-strings");
    strings = json ? JSON.parse(json.textContent) : {};
    var root = page();
    var fromControls = Discourse.siteSettingsFromControls;
    Discourse.siteSettingsFromControls = false;
    // The filter controls start with Only show overridden off and send
    // their state (runInitialFilter): a page loaded with it on drops it,
    // and dependsOn with it.
    var params = new URLSearchParams(location.search);
    if (root && !fromControls && params.get("onlyOverridden")) {
      params.delete("onlyOverridden");
      params.delete("dependsOn");
      setTimeout(function () {
        go(params);
      }, INPUT_DELAY);
    }
    if (root && Discourse.lightbox) {
      root.querySelectorAll(".file-uploader.has-image").forEach(Discourse.lightbox);
    }
    if (root) {
      root.querySelectorAll(".color-picker[data-fallback-primary]").forEach(function (colorPicker) {
        syncColorPicker(colorPicker, colorPicker.closest(".row.setting").dataset.value);
      });
    }
    if (root && Discourse.siteSettingsRefocus) {
      Discourse.siteSettingsRefocus = false;
      var input = root.querySelector("#setting-filter");
      if (input) {
        input.focus();
        input.setSelectionRange(input.value.length, input.value.length);
      }
    }
    return function () {
      document.body.classList.remove("has-admin-changes-banner");
    };
  });

  // A layout redraw (the window crossing a breakpoint) swaps the page in
  // again; the pending edits, and so the changes banner, come along as
  // Ember keeps them in its tracker.
  var pending = null;
  document.addEventListener("discourse:before-redraw", function () {
    pending = dirtyRows().map(function (row) {
      return [row.dataset.setting, currentValue(row)];
    });
  });
  document.addEventListener("discourse:redrawn", function () {
    var root = page();
    if (root && pending) {
      pending.forEach(function (edit) {
        var row = root.querySelector('.row.setting[data-setting="' + edit[0] + '"]');
        if (row) {
          setValue(row, edit[1]);
          syncRow(row);
        }
      });
    }
    pending = null;
  });

  document.addEventListener("input", function (event) {
    var root = page();
    if (!root || !root.contains(event.target)) {
      return;
    }
    if (event.target.id === "setting-filter") {
      var value = event.target.value;
      clearTimeout(timer);
      timer = setTimeout(function () {
        var params = new URLSearchParams(location.search);
        if (value) {
          params.set("filter", value);
        } else {
          params.delete("filter");
        }
        go(params, true);
      }, INPUT_DELAY);
      return;
    }
    var chooserFilter = event.target.closest("details.multi-select[data-list-value] .filter-input");
    if (chooserFilter) {
      var filtered = chooserFilter.closest("details");
      if (filtered.dataset.tagSearch) {
        searchTags(filtered, chooserFilter.value);
      } else {
        drawRows(filtered, rowsFor(filtered, chooserFilter.value), chooserFilter.value);
      }
      return;
    }
    var adderFilter = event.target.closest(".value-list[data-delimiter] > details.combo-box .filter-input");
    if (adderFilter) {
      drawValueListAdder(adderFilter.closest("details"));
      return;
    }
    var addInput = event.target.closest(".simple-list .add-value-input");
    if (addInput) {
      addInput.parentElement.querySelector(".add-value-btn").disabled = addInput.value === "";
      return;
    }
    var colorPicker = event.target.closest(".color-picker");
    if (colorPicker) {
      // The hex input sets the value normalized; the picker sets the hex
      // input (without its #) and so the value.
      var hexInput = colorPicker.querySelector(".hex-input");
      if (event.target.classList.contains("picker")) {
        hexInput.value = event.target.value.replace(/^#/, "");
      }
      colorPicker.dataset.value = normalizeColor(hexInput.value || "");
      syncColorPicker(colorPicker, colorPicker.dataset.value);
    }
    if (event.target.matches(".file-size-input")) {
      fileSizeInput(event.target.closest(".file-size-picker"));
    }
    var row = event.target.closest(ROW);
    if (row) {
      syncRow(row);
    }
  });

  document.addEventListener("change", function (event) {
    var root = page();
    if (!root || !root.contains(event.target)) {
      return;
    }
    if (event.target.matches(".file-uploader input[type='file']")) {
      event.target.value = "";
      unsupported("uploading site setting files");
      return;
    }
    if (event.target.id === "setting-filter-toggle-overridden") {
      var params = new URLSearchParams(location.search);
      if (event.target.checked) {
        params.set("onlyOverridden", "true");
      } else {
        params.delete("onlyOverridden");
      }
      go(params);
      return;
    }
    var row = event.target.closest(ROW);
    if (row) {
      syncRow(row);
    }
  });

  // The choosers' filter: Enter takes the highlighted row (else the first),
  // Escape closes; neither submits the setting's form.
  document.addEventListener("keydown", function (event) {
    var input =
      event.target.closest &&
      event.target.closest("details.multi-select[data-list-value] .filter-input, .value-list[data-delimiter] > details.combo-box .filter-input");
    if (!input || (event.key !== "Enter" && event.key !== "Escape")) {
      return;
    }
    event.preventDefault();
    var details = input.closest("details");
    var adder = details.classList.contains("combo-box");
    if (event.key === "Escape") {
      if (adder) {
        closeValueListAdder(details);
      } else {
        closeChooser(details);
      }
      return;
    }
    var highlighted =
      details.querySelector(".select-kit-collection .select-kit-row.is-highlighted") ||
      details.querySelector(".select-kit-collection .select-kit-row");
    if (adder) {
      // ValueList#keyDown adds what was typed.
      addValueListValue(details, highlighted ? highlighted.dataset.value : input.value.trim());
    } else if (highlighted) {
      pickRow(details, highlighted);
    }
  });

  // A click elsewhere closes an open chooser.
  document.addEventListener("click", function (event) {
    // A row or choice just taken is redrawn away, not elsewhere.
    if (!event.target.isConnected) {
      return;
    }
    document
      .querySelectorAll("details.multi-select[data-list-value].is-expanded, .value-list[data-delimiter] > details.combo-box.is-expanded")
      .forEach(function (details) {
        if (details.contains(event.target)) {
          return;
        }
        if (details.classList.contains("combo-box")) {
          closeValueListAdder(details);
        } else {
          closeChooser(details);
        }
      });
  });

  document.addEventListener("keydown", function (event) {
    var addInput = event.target.closest && event.target.closest(".simple-list input.add-value-input");
    if (addInput && event.key === "Enter" && addInput.value !== "") {
      event.preventDefault();
      var list = addInput.closest(".simple-list");
      addListValue(list, addInput.value);
      addInput.value = "";
      addInput.parentElement.querySelector(".add-value-btn").disabled = true;
      syncRow(list.closest(".row.setting"));
    }
  });

  document.addEventListener("submit", function (event) {
    if (page() && event.target.closest(".row.setting .form-kit")) {
      event.preventDefault();
      save(event.target.closest(".row.setting"));
    }
  });

  document.addEventListener("click", function (event) {
    var root = page();
    if (!root) {
      return;
    }
    var target = event.target;
    if (target.closest(".admin-changes-banner .banner-save")) {
      saveRows(dirtyRows(), true).then(syncBanner);
      return;
    }
    if (target.closest(".admin-changes-banner .banner-discard")) {
      dirtyRows().forEach(cancel);
      return;
    }
    if (!root.contains(target)) {
      return;
    }
    if (target.closest("#clear-filter")) {
      var params = new URLSearchParams(location.search);
      params.delete("filter");
      root.querySelector("#setting-filter").value = "";
      go(params);
      return;
    }
    var menuToggle = target.closest(".admin-site-settings-filter-controls .menu-toggle");
    if (menuToggle && Discourse.float) {
      var list = root.querySelector(".admin-site-settings-category-nav__list");
      Discourse.float.menu(menuToggle, {
        identifier: "admin-site-settings-category-nav",
        modalForMobile: true,
        content: list.cloneNode(true),
      });
      return;
    }
    var row = target.closest(ROW);
    if (!row) {
      return;
    }
    if (target.closest(".setting-controls__ok")) {
      save(row);
      return;
    }
    if (target.closest(".setting-controls__cancel")) {
      cancel(row);
      return;
    }
    if (target.closest(".uploaded-image-list-edit")) {
      unsupported("the uploaded image list modal");
      return;
    }
    if (target.closest(".secret-value-list button")) {
      unsupported("the secret list setting editor");
      return;
    }
    // setDefaultValues: the defaults added to the values.
    var defaultsButton = target.closest(".setting-label [data-default-values]");
    if (defaultsButton) {
      var current = currentValue(row).split("|").filter(Boolean);
      defaultsButton.dataset.defaultValues.split("|").forEach(function (v) {
        if (v && current.indexOf(v) === -1) {
          current.push(v);
        }
      });
      setValue(row, current.join("|"));
      showError(row, null);
      syncRow(row);
      defaultsButton.remove();
      return;
    }
    var chooser = target.closest("details.multi-select[data-list-value]");
    if (chooser) {
      if (target.closest(".select-kit-filter")) {
        return;
      }
      event.preventDefault();
      var pick = target.closest(".select-kit-row");
      var drop = target.closest(".selected-choice");
      if (pick) {
        pickRow(chooser, pick);
      } else if (drop) {
        // ListSetting#deselect: the value stays among the choices.
        addExtraChoice(chooser, choiceNamed(chooser, drop.dataset.value));
        changeChooser(
          chooser,
          chooserValues(chooser).filter(function (v) {
            return v !== drop.dataset.value;
          })
        );
      } else if (target.closest("summary")) {
        if (chooser.classList.contains("is-expanded")) {
          closeChooser(chooser);
        } else if (!row.classList.contains("disabled")) {
          openChooser(chooser);
        }
      }
      return;
    }
    var adder = target.closest(".value-list[data-delimiter] > details.combo-box");
    if (adder) {
      if (target.closest(".select-kit-filter")) {
        return;
      }
      event.preventDefault();
      var added = target.closest(".select-kit-row");
      if (added) {
        addValueListValue(adder, added.dataset.value);
      } else if (adder.classList.contains("is-expanded")) {
        closeValueListAdder(adder);
      } else {
        adder.classList.add("is-expanded");
        adder.open = true;
        drawValueListAdder(adder);
      }
      return;
    }
    var typesButton = target.closest(".file-types-list__button");
    if (typesButton) {
      insertDefaultTypes(row, typesButton);
      return;
    }
    if (target.closest(".emoji-list .emoji-details, .emoji-list .emoji-picker-trigger")) {
      unsupported("the emoji picker on the site settings page");
      return;
    }
    if (target.closest(".file-uploader-lightbox-btn")) {
      var link = row.querySelector(".file-uploader a.lightbox");
      if (link) {
        link.click();
      }
      return;
    }
    if (target.closest(".file-uploader .btn-danger")) {
      setValue(row, "");
      syncRow(row);
      return;
    }
    if (target.closest(".setting-controls__undo")) {
      setValue(row, row.dataset.default);
      showError(row, null);
      syncRow(row);
      return;
    }
    if (target.closest(".form-kit__control-password-toggle")) {
      var password = row.querySelector(".form-kit__control-password");
      password.type = password.type === "password" ? "text" : "password";
      return;
    }
    var add = target.closest(".simple-list .add-value-btn");
    if (add) {
      var simple = add.closest(".simple-list");
      var addInput = simple.querySelector(".add-value-input");
      addListValue(simple, addInput.value);
      addInput.value = "";
      add.disabled = true;
      syncRow(row);
      return;
    }
    var listButton = target.closest(".value-list .remove-value-btn, .value-list .shift-up-value-btn, .value-list .shift-down-value-btn");
    if (listButton) {
      var simpleList = listButton.closest(".value-list");
      var items = listValues(simpleList);
      var index = Number(listButton.closest(".value").dataset.index);
      // An added fixed entry leads the list but is not in the value.
      if (simpleList.querySelector(".values .value[data-added]")) {
        index -= 1;
      }
      if (index < 0) {
        return;
      }
      if (listButton.classList.contains("remove-value-btn")) {
        items.splice(index, 1);
      } else {
        // SimpleList#shift wraps around the ends.
        var to = index + (listButton.classList.contains("shift-up-value-btn") ? -1 : 1);
        if (to > items.length - 1) {
          to = 0;
        } else if (to < 0) {
          to = items.length - 1;
        }
        var moved = items.splice(index, 1)[0];
        items.splice(to, 0, moved);
      }
      renderList(simpleList, items);
      syncRow(row);
      return;
    }
    var combo = target.closest("details.combo-box[data-options]");
    if (combo) {
      event.preventDefault();
      if (combo.classList.contains("is-disabled")) {
        return;
      }
      var adding = combo.classList.contains("add-value-input");
      var choice = target.closest(".select-kit-row");
      if (choice) {
        closeCombo(combo);
        if (adding) {
          addListValue(combo.closest(".simple-list"), choice.dataset.value);
        } else {
          setCombo(combo, choice.dataset.value, choice.dataset.name);
          if (combo.classList.contains("file-size-unit-selector")) {
            showFileSize(combo.closest(".file-size-picker"));
          }
        }
        syncRow(row);
        return;
      }
      if (combo.classList.contains("is-expanded")) {
        closeCombo(combo);
        return;
      }
      var current = combo.querySelector("summary").dataset.value;
      var options = adding
        ? validChoices(combo.parentElement, listValues(combo.closest(".simple-list")))
        : JSON.parse(combo.dataset.options);
      combo.classList.add("is-expanded");
      combo.open = true;
      combo.querySelector(".select-kit-body").innerHTML =
        '<div class="select-kit-collection" role="listbox">' +
        options
          .map(function (o) {
            var name = String(o.name).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/"/g, "&quot;");
            return (
              '<div class="select-kit-row' +
              (String(o.id) === current ? " is-selected" : "") +
              '" role="option" tabindex="0" data-value="' +
              o.id +
              '" data-name="' +
              name +
              '"><span class="name">' +
              name +
              "</span></div>"
            );
          })
          .join("") +
        "</div>";
      return;
    }
    if (target.closest("details.select-kit summary")) {
      event.preventDefault();
      unsupported("the " + row.dataset.type.replace(/_/g, " ") + " setting editor");
      return;
    }
    if (target.closest(".setting-value-edit-button")) {
      unsupported("the JSON setting editors");
    }
  });
})();
