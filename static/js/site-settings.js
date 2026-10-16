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
// URL's query; the page for them is swapped in as a link's is. Not
// ported, and saying so when used: the editors the page says so of, the
// pickers' dropdowns, uploading, the emoji picker and the backfill
// question.
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
    return controls ? controls.closest(".admin-contents") : null;
  }

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
    var checkbox = row.querySelector(".setting-value input.form-kit__control-checkbox");
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
    var listSetting = row.querySelector(".setting-value details.list-setting");
    if (listSetting && listSetting.dataset.listValue !== undefined) {
      return listSetting.dataset.listValue;
    }
    var input = row.querySelector(
      ".setting-value input.form-kit__control-input, .setting-value input.form-kit__control-password, .setting-value textarea"
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
    var checkbox = row.querySelector(".setting-value input.form-kit__control-checkbox");
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
    var listSetting = row.querySelector(".setting-value details.list-setting");
    if (listSetting && row.querySelector(".file-types-list__button")) {
      setFileTypes(listSetting, value);
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
      ".setting-value input.form-kit__control-input, .setting-value input.form-kit__control-password, .setting-value textarea"
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
      return row.classList.contains("--dirty");
    });
  }

  // The row's controls: save and cancel while changed, undo while
  // overridden, nothing otherwise.
  function syncRow(row) {
    var value = currentValue(row);
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
      return Array.prototype.map.call(list.querySelectorAll(".values .emoji-name"), function (name) {
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
    if (!values.length) {
      return;
    }
    var ul = document.createElement("ul");
    ul.className = "values emoji-value-list";
    values.forEach(function (value, index) {
      var item = document.createElement("li");
      item.className = "value";
      item.dataset.index = index;
      item.innerHTML =
        smallButton("remove-value-btn", "xmark") +
        ' <div class="value-input emoji-details can-edit" role="button"> <img class="emoji-list-emoji" height="15px" width="15px"> <span class="emoji-name"></span> </div>' +
        (values.length > 1
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

  // The file types list's closed MultiSelect, naming its values.
  function setFileTypes(details, value) {
    details.dataset.listValue = value;
    var values = value.split("|").filter(Boolean);
    var summary = details.querySelector("summary");
    summary.dataset.value = values.join(",");
    summary.dataset.name = values.join(",");
    details.classList.toggle("has-selection", values.length > 0);
    details.querySelector(".formatted-selection").textContent = values.length
      ? values.join(", ")
      : strings["select_kit.default_header_text"] || "";
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
    setFileTypes(row.querySelector(".setting-value details.list-setting"), newTypes.join("|"));
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
    var row = event.target.closest(".row.setting[data-setting]");
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
    var row = event.target.closest(".row.setting[data-setting]");
    if (row) {
      syncRow(row);
    }
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
    var row = target.closest(".row.setting[data-setting]");
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
