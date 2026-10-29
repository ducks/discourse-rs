// The category editor (crate::category_edit_view): its tabs, the
// permissions table, and Save as Ember's Category#save sends it (JSON,
// PUT /categories/:id or POST /categories, the category types as a
// list: Ideas among them is topic voting's toggle), then Delete
// (DELETE /categories/:id after a confirmation).
(function () {
  "use strict";

  if (!Discourse.once("category-edit")) {
    return;
  }

  function base() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  function csrf() {
    return (document.querySelector('meta[name="csrf-token"]') || {}).content || "";
  }

  function escape(text) {
    var div = document.createElement("div");
    div.textContent = text == null ? "" : String(text);
    return div.innerHTML;
  }

  function page() {
    return document.querySelector(".edit-category-page");
  }

  function data() {
    var p = page();
    try {
      return JSON.parse((p && p.dataset.category) || "{}");
    } catch (_) {
      return {};
    }
  }

  function value(form, name) {
    var el = form.querySelector('[name="' + name + '"]');
    if (!el) {
      return undefined;
    }
    if (el.type === "checkbox") {
      return el.checked;
    }
    return el.value;
  }

  function number(form, name) {
    var v = value(form, name);
    if (v === undefined || v === "") {
      return null;
    }
    return Number(v);
  }

  function text(form, name) {
    var v = value(form, name);
    return v === undefined || v === "" ? null : v;
  }

  // _permissionsForUpdate: group name to permission type.
  function permissions(form) {
    var out = {};
    form.querySelectorAll(".permission-row").forEach(function (row) {
      var select = row.querySelector('select[name="permission_type"]');
      out[row.dataset.groupName] = Number(select.value);
    });
    return out;
  }

  function categoryTypes(form) {
    var out = [];
    form.querySelectorAll(".category-type-option").forEach(function (option) {
      var box = option.querySelector('input[type="checkbox"]');
      if (box && box.checked) {
        out.push(option.dataset.categoryType);
      }
    });
    return out;
  }

  // The plugins' custom field controls (`custom_fields.<name>`) over the
  // category's custom fields, sent whole as Category#save does; none
  // when no plugin draws any.
  function customFields(form, d) {
    var controls = form.querySelectorAll('[name^="custom_fields."]');
    if (!controls.length) {
      return undefined;
    }
    var out = Object.assign({}, d.custom_fields || {});
    controls.forEach(function (el) {
      out[el.name.slice("custom_fields.".length)] = el.type === "checkbox" ? el.checked : el.value;
    });
    return out;
  }

  function payload(form, d) {
    var parent = value(form, "parent_category_id");
    return {
      custom_fields: customFields(form, d),
      name: value(form, "name"),
      slug: value(form, "slug"),
      color: value(form, "color"),
      text_color: value(form, "text_color"),
      permissions: permissions(form),
      auto_close_hours: number(form, "auto_close_hours"),
      default_slow_mode_seconds: number(form, "default_slow_mode_seconds"),
      position: d.position == null ? null : d.position,
      parent_category_id: parent ? Number(parent) : null,
      allow_badges: value(form, "allow_badges"),
      topic_title_placeholder: text(form, "topic_title_placeholder"),
      all_topics_wiki: value(form, "all_topics_wiki"),
      allow_unlimited_owner_edits_on_first_post: value(form, "allow_unlimited_owner_edits_on_first_post"),
      sort_order: text(form, "sort_order"),
      topic_featured_link_allowed: value(form, "topic_featured_link_allowed"),
      show_subcategory_list: value(form, "show_subcategory_list"),
      num_featured_topics: number(form, "num_featured_topics"),
      default_view: text(form, "default_view"),
      subcategory_list_style: value(form, "subcategory_list_style"),
      default_top_period: value(form, "default_top_period"),
      minimum_required_tags: number(form, "minimum_required_tags"),
      navigate_to_first_post_after_read: value(form, "navigate_to_first_post_after_read"),
      read_only_banner: text(form, "read_only_banner"),
      default_list_filter: value(form, "default_list_filter"),
      style_type: value(form, "style_type"),
      emoji: text(form, "emoji"),
      icon: text(form, "icon"),
      category_types: categoryTypes(form),
    };
  }

  function showErrors(form, errors) {
    var box = form.querySelector(".edit-category-errors");
    box.innerHTML = errors.map(escape).join("<br>");
    box.hidden = errors.length === 0;
  }

  function request(method, url, body) {
    return fetch(url, {
      method: method,
      credentials: "same-origin",
      headers: {
        "Content-Type": "application/json",
        Accept: "application/json",
        "X-CSRF-Token": csrf(),
        "X-Requested-With": "XMLHttpRequest",
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    }).then(function (response) {
      return response.json().then(
        function (json) {
          return { ok: response.ok, json: json };
        },
        function () {
          return { ok: response.ok, json: {} };
        }
      );
    });
  }

  function save(event) {
    var form = event.target.closest(".edit-category-form");
    if (!form) {
      return;
    }
    event.preventDefault();
    var d = data();
    var url = d.id ? base() + "/categories/" + d.id + ".json" : base() + "/categories.json";
    var button = form.querySelector("#save-category");
    button.disabled = true;
    request(d.id ? "PUT" : "POST", url, payload(form, d))
      .then(function (result) {
        button.disabled = false;
        if (!result.ok) {
          showErrors(form, result.json.errors || [d.id ? d.strings.save_error : d.strings.creation_error]);
          return;
        }
        showErrors(form, []);
        var category = result.json.category || {};
        var parent = category.parent_category_id;
        var slug = category.slug || category.id + "-category";
        if (!d.id) {
          // A new category: its page.
          window.location.href = base() + "/c/" + slug + "/" + category.id;
          return;
        }
        if (parent || slug !== (d.url || "").split("/").slice(-2)[0]) {
          window.location.reload();
          return;
        }
        Discourse.dialog.alert(escape(d.strings.saved));
      })
      .catch(function () {
        button.disabled = false;
        showErrors(form, [d.strings.save_error]);
      });
  }

  function remove(event) {
    if (!event.target.closest("#delete-category")) {
      return;
    }
    var d = data();
    Discourse.dialog.deleteConfirm({
      message: escape(d.strings.delete_confirm),
      didConfirm: function () {
        request("DELETE", base() + "/categories/" + d.id + ".json").then(function (result) {
          if (result.ok) {
            window.location.href = base() + "/categories";
          } else {
            Discourse.dialog.alert(escape((result.json.errors || []).join(" ")));
          }
        });
      },
    });
  }

  // The tabs switch in place, the URL following.
  function tab(event) {
    var link = event.target.closest(".edit-category-nav a");
    if (!link) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    var name = link.getAttribute("href").split("/").pop();
    document.querySelectorAll(".edit-category-nav a").forEach(function (a) {
      a.classList.toggle("active", a === link);
    });
    document.querySelectorAll(".edit-category-tab").forEach(function (section) {
      section.hidden = !section.classList.contains("edit-category-tab-" + name);
    });
    history.replaceState(history.state, "", link.getAttribute("href"));
  }

  function permissionRow(name) {
    var p = page();
    var template = p.querySelector(".permission-row");
    var row = document.createElement("div");
    row.className = "permission-row row-body";
    row.dataset.groupName = name;
    var select = template
      ? template.querySelector('select[name="permission_type"]').cloneNode(true)
      : null;
    row.innerHTML = '<span class="group-name">' + escape(name) + "</span>";
    if (select) {
      select.value = "1";
      row.appendChild(select);
    }
    var removeButton = document.createElement("button");
    removeButton.className = "btn btn-flat btn-icon no-text remove-permission";
    removeButton.type = "button";
    removeButton.textContent = "×";
    row.appendChild(removeButton);
    return row;
  }

  function permissionsChange(event) {
    var add = event.target.closest('select[name="add_group"]');
    if (add && add.value) {
      var table = page().querySelector(".category-permissions-table");
      table.appendChild(permissionRow(add.value));
      add.querySelector('option[value="' + CSS.escape(add.value) + '"]').remove();
      add.value = "";
    }
  }

  function permissionsRemove(event) {
    var button = event.target.closest(".remove-permission");
    if (!button) {
      return;
    }
    var row = button.closest(".permission-row");
    var name = row.dataset.groupName;
    row.remove();
    var add = page().querySelector('select[name="add_group"]');
    if (add) {
      var option = document.createElement("option");
      option.value = name;
      option.textContent = name;
      add.appendChild(option);
    }
  }

  function swatch(event) {
    var input = event.target.closest('input[name="color"], input[name="text_color"]');
    if (!input) {
      return;
    }
    var sw = input.closest(".category-color-editor").querySelector(".color-swatch");
    if (/^([0-9a-f]{3}|[0-9a-f]{6})$/i.test(input.value)) {
      sw.style.backgroundColor = "#" + input.value;
    }
  }

  document.addEventListener("submit", function (event) {
    if (page()) {
      save(event);
    }
  });
  document.addEventListener("click", function (event) {
    if (!page()) {
      return;
    }
    tab(event);
    remove(event);
    permissionsRemove(event);
  });
  document.addEventListener("change", function (event) {
    if (page()) {
      permissionsChange(event);
    }
  });
  document.addEventListener("input", function (event) {
    if (page()) {
      swatch(event);
    }
  });
})();
