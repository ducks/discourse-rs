// The admin badges pages in the browser (src/admin_badges_view.rs draws
// them): AdminBadgesList's filter, Group settings' EditBadgeGroupings
// modal, AdminBadgesShow's form (the live name, the icon or image
// choice, DIconGridPicker, the badge image's upload, the grouping menu,
// the post header checkbox's dependency, the query's preview, submit and
// delete) and AdminBadgesAward's CSV upload.
(function () {
  "use strict";

  if (!Discourse.once("admin-badges")) {
    return;
  }

  var ui = Discourse.adminUi;
  var t = ui.t;
  var escapeHtml = ui.escapeHtml;

  function page() {
    return document.querySelector(".admin-contents.admin-badges");
  }

  function data() {
    var el = document.getElementById("admin-badges-data");
    try {
      return el ? JSON.parse(el.textContent) : {};
    } catch (_) {
      return {};
    }
  }

  function form() {
    var root = page();
    return root && root.querySelector("form.badge-form");
  }

  function badgeId() {
    var m = location.pathname.match(/\/admin\/badges\/(\d+)\/?$/);
    return m ? m[1] : null;
  }

  // --- AdminBadgesList: DFilterControls on name and description, the
  // text kept in the URL's filter.

  function resetHtml(label) {
    return (
      '<button aria-label="' +
      escapeHtml(t("filter_controls.reset")) +
      '" class="btn ' +
      (label ? "btn-icon-text" : "no-text btn-icon") +
      ' btn-default d-filter-controls__reset" title="' +
      escapeHtml(t("filter_controls.reset")) +
      '" type="button">' +
      ui.icon("arrow-rotate-left") +
      (label
        ? '<span class="d-button-label">' + escapeHtml(t("filter_controls.reset")) + "</span>"
        : '<span aria-hidden="true">&#8203;</span>') +
      "</button>"
    );
  }

  function filterList(list) {
    var input = list.querySelector(".d-filter-controls__input");
    var terms = input.value
      .split(",")
      .map(function (term) {
        return term.trim().toLowerCase();
      })
      .filter(Boolean);
    var shown = 0;
    list.querySelectorAll(".admin-badge-list-item").forEach(function (item) {
      var badge = item.querySelector(".user-badge");
      var name = (badge.dataset.badgeName || "").toLowerCase();
      var description = (badge.getAttribute("title") || "").toLowerCase();
      var match =
        terms.length === 0 ||
        terms.some(function (term) {
          return name.indexOf(term) !== -1 || description.indexOf(term) !== -1;
        });
      item.hidden = !match;
      if (match) {
        shown += 1;
      }
    });
    var ul = list.querySelector(".admin-badge-list");
    if (ul) {
      ul.hidden = shown === 0;
    }
    list.querySelector(".d-filter-controls .d-filter-controls__reset").disabled = input.value.length === 0;
    var none = list.querySelector(".d-filter-controls__no-results");
    if (shown === 0 && input.value.length > 0) {
      if (!none) {
        list
          .querySelector(".d-filter-controls")
          .insertAdjacentHTML(
            "afterend",
            '<div class="d-filter-controls__no-results"><p>' +
              escapeHtml(t("admin.badges.no_badges_found")) +
              "</p>" +
              resetHtml(true) +
              "</div>"
          );
      }
    } else if (none) {
      none.remove();
    }
    var url = new URL(location.href);
    if (input.value) {
      url.searchParams.set("filter", input.value);
    } else {
      url.searchParams.delete("filter");
    }
    history.replaceState(history.state, "", url.pathname + url.search + url.hash);
  }

  var filterTimer = null;
  document.addEventListener("input", function (event) {
    var list = event.target.closest(".admin-badges .content-list");
    if (list && event.target.matches(".d-filter-controls__input")) {
      clearTimeout(filterTimer);
      filterTimer = setTimeout(function () {
        filterList(list);
      }, 250);
    }
  });

  // --- EditBadgeGroupings: the working copy, saved all at once.

  function groupingsModal() {
    var d = data();
    var working = (d.badge_groupings || []).map(function (g, i) {
      return {
        id: g.id,
        name: g.name,
        displayName: (d.grouping_names || [])[i] || g.name,
        system: g.system,
        editing: false,
      };
    });
    function iconButton(name, action, disabled) {
      return (
        '<button class="btn no-text btn-icon btn-default" data-action="' +
        action +
        '"' +
        (disabled ? ' disabled=""' : "") +
        ' type="button">' +
        ui.icon(name) +
        '<span aria-hidden="true">&#8203;</span></button>'
      );
    }
    function draw(body) {
      body.querySelector(".badge-groupings-list").innerHTML = working
        .map(function (wc, i) {
          return (
            '<li class="badge-grouping-item" data-index="' +
            i +
            '"><div class="badge-grouping">' +
            (wc.editing
              ? '<input class="badge-grouping-name-input" type="text" value="' + escapeHtml(wc.name) + '">'
              : "<span>" + escapeHtml(wc.displayName || wc.name) + "</span>") +
            '</div><div class="actions">' +
            (wc.editing ? iconButton("check", "done", false) : iconButton("pencil", "edit", wc.system)) +
            iconButton("chevron-up", "up", false) +
            iconButton("chevron-down", "down", false) +
            iconButton("xmark", "delete", wc.system) +
            "</div></li>"
          );
        })
        .join("");
    }
    Discourse.modal.show({
      title: t("admin.badges.badge_groupings.modal_title"),
      body: function (body) {
        body.classList.add("badge-groupings-modal");
        body.innerHTML =
          '<div class="badge-groupings"><ul class="badge-groupings-list"></ul></div><button class="btn btn-default badge-groupings__add-grouping" type="button"><span class="d-button-label">' +
          escapeHtml(t("admin.badges.new")) +
          "</span></button>";
        draw(body);
        body.addEventListener("input", function (event) {
          var input = event.target.closest(".badge-grouping-name-input");
          if (input) {
            var wc = working[Number(input.closest("li").dataset.index)];
            wc.name = input.value;
            wc.displayName = input.value;
          }
        });
        body.addEventListener("click", function (event) {
          if (event.target.closest(".badge-groupings__add-grouping")) {
            var name = t("admin.badges.badge_grouping");
            working.push({ id: null, name: name, displayName: name, system: false, editing: true });
            draw(body);
            return;
          }
          var button = event.target.closest(".actions button[data-action]");
          if (!button || button.disabled) {
            return;
          }
          var index = Number(button.closest("li").dataset.index);
          var wc = working[index];
          var action = button.dataset.action;
          if (action === "edit") {
            wc.editing = true;
          } else if (action === "done") {
            wc.editing = false;
          } else if (action === "delete") {
            working.splice(index, 1);
          } else {
            var to = index + (action === "up" ? -1 : 1);
            if (to < 0 || to >= working.length) {
              return;
            }
            working.splice(index, 1);
            working.splice(to, 0, wc);
          }
          draw(body);
        });
      },
      footer: function (footer, close) {
        footer.innerHTML =
          '<button class="btn btn-primary badge-groupings__save" type="button"><span class="d-button-label">' +
          escapeHtml(t("admin.badges.save")) +
          '</span></button><button class="btn btn-flat d-modal-cancel" type="button"><span class="d-button-label">' +
          escapeHtml(t("cancel")) +
          "</span></button>";
        footer.querySelector(".d-modal-cancel").addEventListener("click", close);
        footer.querySelector(".badge-groupings__save").addEventListener("click", function () {
          ui.ajax("POST", "/admin/badges/badge_groupings", {
            ids: working.map(function (wc) {
              return wc.id || -1;
            }),
            names: working.map(function (wc) {
              return wc.name;
            }),
          })
            .then(function () {
              close();
              location.reload();
            })
            .catch(function () {
              Discourse.dialog.alert(t("generic_error"));
            });
        });
      },
    });
  }

  // --- AdminBadgesShow.

  function field(f, name) {
    return f.querySelector('[name="' + name + '"]');
  }

  // The post header checkbox: disabled unless listable and show_posts,
  // its note while disabled on a badge that is not a system one.
  function syncPostHeader(f) {
    var disabled = !field(f, "listable").checked || !field(f, "show_posts").checked;
    var box = field(f, "show_in_post_header");
    var container = box.closest(".form-kit__field");
    box.disabled = disabled;
    container.classList.toggle("is-disabled", disabled);
    if (disabled) {
      container.setAttribute("data-disabled", "");
    } else {
      container.removeAttribute("data-disabled");
    }
    container.querySelector(".form-kit__control-checkbox-description").textContent =
      disabled ? f.dataset.postHeaderNote || "" : "";
  }

  function headerIcon(f) {
    var header = f.querySelector(".current-badge-header");
    var name = header.querySelector(".badge-display-name");
    while (header.firstChild && header.firstChild !== name) {
      header.removeChild(header.firstChild);
    }
    var image = f.dataset.imageUrl;
    var iconName = f.querySelector(".d-icon-grid-picker").dataset.value;
    if (image) {
      header.insertAdjacentHTML("afterbegin", "<img src='" + escapeHtml(image) + "' alt=''>");
    } else if (iconName) {
      header.insertAdjacentHTML("afterbegin", ui.icon(iconName));
    }
  }

  function setIcon(f, name) {
    var picker = f.querySelector(".d-icon-grid-picker");
    var trigger = picker.querySelector(".d-icon-grid-picker-trigger");
    if (name) {
      picker.dataset.value = name;
      picker.classList.add("--has-label");
      trigger.title = t("d_icon_grid_picker.selected_icon", { iconName: name });
    } else {
      delete picker.dataset.value;
      picker.classList.remove("--has-label");
      trigger.removeAttribute("title");
    }
    trigger.innerHTML =
      (name ? ui.icon(name) : "") +
      '<span class="d-icon-grid-picker__label">' +
      escapeHtml(name || t("d_icon_grid_picker.select_icon")) +
      "</span>" +
      ui.icon("angle-down").replace("svg-icon", "svg-icon d-icon-grid-picker__caret");
    if (name) {
      // onSetIcon: the image goes.
      f.dataset.imageUrl = "";
      f.dataset.imageUploadId = "";
      setImagePreview(f, "");
    }
    headerIcon(f);
    clearIconError(f);
  }

  function setImagePreview(f, url) {
    var uploader = f.querySelector(".form-kit__control-image");
    uploader.classList.toggle("no-image", !url);
    uploader.classList.toggle("has-image", !!url);
    uploader.querySelector(".file-uploader__preview").style.backgroundImage = "url(" + (url || "") + ")";
  }

  function clearIconError(f) {
    f.querySelectorAll('[data-name="icon"] .form-kit__errors, [data-name="image_url"] .form-kit__errors').forEach(
      function (e) {
        e.closest(".form-kit__meta").remove();
      }
    );
    f.querySelectorAll('[data-name="icon"], [data-name="image_url"]').forEach(function (c) {
      c.classList.remove("has-error");
    });
  }

  // DIconGridPicker's content: the filter, then /svg-sprite/picker-search's
  // pages as the grid.
  function openIconPicker(trigger) {
    var f = form();
    var picker = trigger.closest(".d-icon-grid-picker");
    var value = picker.dataset.value || "";
    var content = document.createElement("div");
    content.className = "d-icon-grid-picker__content";
    content.innerHTML =
      '<div class="d-icon-grid-picker__filter-container"><div class="filter-input-container d-icon-grid-picker__filter">' +
      ui.icon("magnifying-glass").replace("svg-icon", "svg-icon -left") +
      '<input aria-controls="d-icon-grid-picker-listbox" aria-label="' +
      escapeHtml(t("d_icon_grid_picker.search_label")) +
      '" class="filter-input" placeholder="' +
      escapeHtml(t("d_icon_grid_picker.search_placeholder")) +
      '" type="text"></div></div><div aria-label="' +
      escapeHtml(t("d_icon_grid_picker.select_icon")) +
      '" class="d-icon-grid-picker__grid-wrapper" id="d-icon-grid-picker-listbox" role="listbox">' +
      (value
        ? '<div aria-label="favorites" class="d-icon-grid-picker__favorites" role="group"><button aria-label="' +
          escapeHtml(value) +
          '" aria-selected="true" class="d-icon-grid-picker__icon --selected d-icon-grid-picker__selected-chip" data-icon-id="' +
          escapeHtml(value) +
          '" role="option" type="button">' +
          ui.icon(value) +
          '<span class="d-icon-grid-picker__selected-name">' +
          escapeHtml(value) +
          "</span></button></div>"
        : "") +
      '<div class="d-icon-grid-picker__grid" role="presentation"></div></div>';
    var grid = content.querySelector(".d-icon-grid-picker__grid");
    var search = 0;
    function load(filter) {
      var mine = ++search;
      grid.innerHTML = '<div class="d-icon-grid-picker__loading"><div class="spinner"></div></div>';
      ui.ajax(
        "GET",
        "/svg-sprite/picker-search?only_available=false&filter=" + encodeURIComponent(filter)
      )
        .then(function (result) {
          if (mine !== search) {
            return;
          }
          var icons = (result && result.icons) || [];
          if (!icons.length) {
            grid.innerHTML =
              '<div class="d-icon-grid-picker__empty" role="status">' +
              escapeHtml(t("d_icon_grid_picker.no_results")) +
              "</div>";
            return;
          }
          grid.innerHTML = icons
            .map(function (icon) {
              return (
                '<button aria-label="' +
                escapeHtml(icon.id) +
                '" aria-selected="' +
                (icon.id === value ? "true" : "false") +
                '" class="d-icon-grid-picker__icon' +
                (icon.id === value ? " --selected" : "") +
                '" data-icon-id="' +
                escapeHtml(icon.id) +
                '" role="option" type="button">' +
                (icon.symbol || "") +
                ui.icon(icon.id) +
                "</button>"
              );
            })
            .join("");
        })
        .catch(function () {
          grid.innerHTML = "";
          ui.unsupported("the icon picker's search (/svg-sprite/picker-search)");
        });
    }
    var timer = null;
    content.querySelector(".filter-input").addEventListener("input", function (event) {
      clearTimeout(timer);
      var filter = event.target.value;
      timer = setTimeout(function () {
        load(filter);
      }, 300);
    });
    content.addEventListener("click", function (event) {
      var cell = event.target.closest(".d-icon-grid-picker__icon[data-icon-id]");
      if (cell) {
        setIcon(f, cell.dataset.iconId);
        Discourse.float.close("d-icon-grid-picker");
      }
    });
    trigger.setAttribute("aria-expanded", "true");
    Discourse.float.menu(trigger, {
      identifier: "d-icon-grid-picker",
      mainClass: "d-icon-grid-picker-content",
      maxWidth: 490,
      modalForMobile: true,
      content: content,
      onClose: function () {
        trigger.setAttribute("aria-expanded", "false");
      },
    });
    load("");
    var input = content.querySelector(".filter-input");
    if (input) {
      input.focus();
    }
  }

  // FKControlMenu: the grouping's DMenu.
  function openMenu(trigger) {
    var template = trigger.parentNode.querySelector("template.form-kit-menu-content");
    var content = template.content.firstElementChild.cloneNode(true);
    content.addEventListener("click", function (event) {
      var item = event.target.closest(".form-kit__control-menu-item");
      if (!item) {
        return;
      }
      trigger.dataset.value = item.dataset.value;
      trigger.querySelector(".d-button-label").textContent = item.textContent.trim();
      Discourse.float.close("form-kit-menu");
    });
    trigger.setAttribute("aria-expanded", "true");
    Discourse.float.menu(trigger, {
      identifier: "form-kit-menu",
      mainClass: "form-kit__control-menu-content",
      content: content,
      onClose: function () {
        trigger.setAttribute("aria-expanded", "false");
      },
    });
  }

  function uploadImage(f, input) {
    var file = input.files && input.files[0];
    if (!file) {
      return;
    }
    var uploader = f.querySelector(".form-kit__control-image");
    var status = uploader.querySelector(".file-uploader__progress-status");
    status.classList.remove("hidden");
    var body = new FormData();
    body.append("upload_type", "badge_image");
    body.append("file", file);
    ui.ajax("POST", "/uploads.json", body)
      .then(function (upload) {
        f.dataset.imageUploadId = upload.id;
        f.dataset.imageUrl = upload.url;
        setImagePreview(f, upload.url);
        // onSetImage: the icon goes.
        var picker = f.querySelector(".d-icon-grid-picker");
        delete picker.dataset.value;
        setIconLabelOnly(picker);
        headerIcon(f);
        clearIconError(f);
      })
      .catch(ui.failed)
      .then(function () {
        status.classList.add("hidden");
        input.value = "";
      });
  }

  function setIconLabelOnly(picker) {
    var trigger = picker.querySelector(".d-icon-grid-picker-trigger");
    picker.classList.remove("--has-label");
    trigger.removeAttribute("title");
    trigger.innerHTML =
      '<span class="d-icon-grid-picker__label">' +
      escapeHtml(t("d_icon_grid_picker.select_icon")) +
      "</span>" +
      ui.icon("angle-down").replace("svg-icon", "svg-icon d-icon-grid-picker__caret");
  }

  // The FORM_FIELDS the form sends, a system badge's protected ones left
  // out.
  var FORM_FIELDS = [
    "allow_title",
    "multiple_grant",
    "listable",
    "auto_revoke",
    "enabled",
    "show_posts",
    "target_posts",
    "name",
    "description",
    "long_description",
    "icon",
    "image_upload_id",
    "image_url",
    "query",
    "badge_grouping_id",
    "trigger",
    "badge_type_id",
    "show_in_post_header",
  ];

  // The form's data: the badge's FORM_FIELDS, then what the controls
  // hold. handleSubmit's system check reads `system` off this data,
  // which never has it, so every field is sent.
  function formData(f) {
    var badge = data().badge || {};
    var values = {};
    FORM_FIELDS.forEach(function (name) {
      values[name] = badge[name];
    });
    if (values.icon === "") {
      values.icon = undefined;
    }
    var picker = f.querySelector(".d-icon-grid-picker");
    values.enabled = (f.querySelector('[name="enabled"]:checked') || {}).value === "true";
    values.icon = picker.dataset.value || (f.dataset.imageUrl ? null : undefined);
    values.image_upload_id = f.dataset.imageUploadId || "";
    values.image_url = f.dataset.imageUrl || "";
    values.badge_grouping_id = f.querySelector(".form-kit__control-menu-trigger").dataset.value;
    ["name", "description", "long_description", "query", "badge_type_id", "trigger"].forEach(function (name) {
      var el = field(f, name);
      if (el) {
        values[name] = el.value;
      }
    });
    ["allow_title", "multiple_grant", "listable", "auto_revoke", "show_posts", "target_posts", "show_in_post_header"].forEach(
      function (name) {
        var el = field(f, name);
        if (el) {
          values[name] = el.checked;
        }
      }
    );
    var out = {};
    FORM_FIELDS.forEach(function (name) {
      if (values[name] !== undefined) {
        out[name] = values[name];
      }
    });
    return out;
  }

  function showError(container, message) {
    if (container.querySelector(".form-kit__errors")) {
      return;
    }
    container.classList.add("has-error");
    container
      .querySelector(".form-kit__container-content")
      .insertAdjacentHTML(
        "beforeend",
        '<div class="form-kit__meta"><p aria-live="assertive" class="form-kit__errors"><span>' +
          ui.icon("triangle-exclamation") +
          " " +
          escapeHtml(message) +
          "</span></p></div>"
      );
  }

  function submit(f) {
    var values = formData(f);
    if (!f.querySelector(".d-icon-grid-picker").dataset.value && !f.dataset.imageUrl) {
      f.querySelectorAll('[data-name="icon"], [data-name="image_url"]').forEach(function (c) {
        showError(c, t("admin.badges.icon_or_image"));
      });
      return;
    }
    var nameField = field(f, "name");
    if (nameField && !nameField.value.trim()) {
      showError(nameField.closest(".form-kit__field"), t("form_kit.errors.required"));
      return;
    }
    var id = badgeId();
    ui.ajax(id ? "PUT" : "POST", id ? "/admin/badges/" + id : "/admin/badges", values)
      .then(function (result) {
        ui.toast(t("saved"));
        var badge = result && result.badge;
        if (!id && badge) {
          window.location = ui.base() + "/admin/badges/" + badge.id;
          return;
        }
        // The list's button: the name, the icon or image.
        var item = page().querySelector('.admin-badge-list-item a[href$="/admin/badges/' + id + '"] .user-badge');
        if (item && badge) {
          item.dataset.badgeName = badge.name;
          var label = item.querySelector(".badge-display-name");
          while (item.firstChild && item.firstChild !== label) {
            item.removeChild(item.firstChild);
          }
          if (badge.image_url) {
            item.insertAdjacentHTML("afterbegin", "<img src='" + escapeHtml(badge.image_url) + "' alt=''>");
          } else if (badge.icon) {
            item.insertAdjacentHTML("afterbegin", ui.icon(badge.icon));
          }
          label.textContent = badge.name;
          item.classList.toggle("disabled", !badge.enabled);
        }
      })
      .catch(ui.failed);
  }

  function deleteBadge() {
    var id = badgeId();
    if (!id) {
      window.location = ui.base() + "/admin/badges";
      return;
    }
    Discourse.dialog.deleteConfirm({
      title: t("admin.badges.delete_confirm"),
      didConfirm: function () {
        ui.ajax("DELETE", "/admin/badges/" + id)
          .then(function () {
            window.location = ui.base() + "/admin/badges";
          })
          .catch(function () {
            Discourse.dialog.alert(t("generic_error"));
          });
      },
    });
  }

  // BadgePreviewModal
  function preview(f, explain) {
    var buttons = f.querySelectorAll(".preview-badge, .preview-badge-plan");
    buttons.forEach(function (b) {
      b.disabled = true;
    });
    ui.ajax("POST", "/admin/badges/preview.json", {
      sql: field(f, "query").value,
      target_posts: !!field(f, "target_posts").checked,
      trigger: field(f, "trigger").value,
      explain: explain,
    })
      .then(function (badge) {
        Discourse.modal.show({
          className: "badge-query-preview",
          title: t("admin.badges.preview.modal_title"),
          body: function (body) {
            body.innerHTML = previewHtml(badge || {});
          },
        });
      })
      .catch(function () {
        Discourse.dialog.alert("Network error");
      })
      .then(function () {
        buttons.forEach(function (b) {
          b.disabled = false;
        });
      });
  }

  function previewHtml(badge) {
    if (badge.errors) {
      return (
        '<p class="error-header">' +
        escapeHtml(t("admin.badges.preview.sql_error_header")) +
        '</p><pre class="badge-errors">' +
        escapeHtml(badge.errors) +
        "</pre>"
      );
    }
    var sample = badge.sample || [];
    var out =
      '<p class="grant-count">' +
      (badge.grant_count
        ? t("admin.badges.preview.grant_count." + (badge.grant_count === 1 ? "one" : "other"), {
            count: badge.grant_count,
          })
        : t("admin.badges.preview.no_grant_count")) +
      "</p>";
    var warning = badge.grant_count <= 10 ? sample.length !== badge.grant_count : sample.length !== 10;
    if (warning) {
      out +=
        '<div class="count-warning"><p class="heading">' +
        ui.icon("triangle-exclamation") +
        " " +
        escapeHtml(t("admin.badges.preview.bad_count_warning.header")) +
        '</p><p class="body">' +
        escapeHtml(t("admin.badges.preview.bad_count_warning.text")) +
        "</p></div>";
    }
    if (sample.length) {
      out +=
        '<p class="sample">' +
        escapeHtml(t("admin.badges.preview.sample")) +
        "</p><ul>" +
        sample
          .map(function (grant) {
            var key = "admin.badges.preview.grant.with";
            var params = { username: escapeHtml(grant.username) };
            if (grant.post_id) {
              key += "_post";
              params.link =
                '<a href="' +
                ui.base() +
                "/p/" +
                grant.post_id +
                '" data-auto-route="true">' +
                escapeHtml(grant.title) +
                "</a>";
            }
            if (grant.granted_at) {
              key += "_time";
              params.time = escapeHtml(new Date(grant.granted_at).toLocaleString());
            }
            return "<li>" + t(key, params) + "</li>";
          })
          .join("") +
        "</ul>";
    }
    if (badge.query_plan) {
      out +=
        '<div class="badge-query-plan"><pre>' +
        badge.query_plan
          .map(function (line) {
            return escapeHtml(line["QUERY PLAN"]) + "<br>";
          })
          .join("") +
        "</pre></div>";
    }
    return out;
  }

  // --- AdminBadgesAward

  function massAward(awardForm) {
    var input = awardForm.querySelector("#massAwardCSVUpload");
    var file = input.files && input.files[0];
    // The saving note and the results, after the form.
    var results = awardForm.parentNode.querySelector(".mass-award-results");
    if (!results) {
      results = document.createElement("div");
      results.className = "mass-award-results";
      awardForm.after(results);
    }
    if (!file) {
      Discourse.dialog.alert(t("admin.badges.mass_award.aborted"));
      return;
    }
    var body = new FormData();
    body.append("file", file);
    body.append("replace_badge_owners", awardForm.querySelector(".checkbox-label input").checked);
    var existing = awardForm.querySelector(".grant-existing-holders-checkbox");
    body.append("grant_existing_holders", existing ? existing.checked : false);
    var submitButton = awardForm.querySelector("button[type=submit]");
    submitButton.disabled = true;
    results.innerHTML = escapeHtml(t("uploading"));
    ui.ajax("POST", "/admin/badges/award/" + awardForm.dataset.badgeId, body)
      .then(function (result) {
        var count = result.matched_users_count;
        var html =
          "<p>" +
          ui.icon("check").replace("svg-icon", "svg-icon bulk-award-status-icon success") +
          " " +
          escapeHtml(t("admin.badges.mass_award.success." + (count === 1 ? "one" : "other"), { count: count })) +
          "</p>";
        var unmatched = result.unmatched_entries || [];
        if (unmatched.length) {
          var total = result.unmatched_entries_count;
          var truncated = total && unmatched.length && total > unmatched.length;
          html +=
            "<p>" +
            ui.icon("triangle-exclamation").replace("svg-icon", "svg-icon bulk-award-status-icon failure") +
            " <span>" +
            escapeHtml(
              truncated
                ? t(
                    "admin.badges.mass_award.csv_has_unmatched_users_truncated_list." +
                      (total === 1 ? "one" : "other"),
                    { count: total }
                  )
                : t("admin.badges.mass_award.csv_has_unmatched_users")
            ) +
            "</span></p><ul>" +
            unmatched
              .map(function (entry) {
                return "<li>" + escapeHtml(entry) + "</li>";
              })
              .join("") +
            "</ul>";
        }
        results.innerHTML = html;
      })
      .catch(function (error) {
        results.innerHTML =
          "<p>" +
          ui.icon("xmark").replace("svg-icon", "svg-icon bulk-award-status-icon failure") +
          " " +
          escapeHtml(error.message) +
          "</p>";
      })
      .then(function () {
        submitButton.disabled = !(input.files && input.files.length);
      });
  }

  document.addEventListener("change", function (event) {
    var root = page();
    if (!root || !root.contains(event.target)) {
      return;
    }
    if (event.target.id === "massAwardCSVUpload") {
      event.target.closest("form").querySelector("button[type=submit]").disabled = !event.target.files.length;
      return;
    }
    var f = form();
    if (!f || !f.contains(event.target)) {
      return;
    }
    if (event.target.matches(".form-kit__conditional-display .form-kit__control-radio")) {
      var display = event.target.closest(".form-kit__conditional-display");
      var choices = display.querySelectorAll(":scope > .form-kit__inline-radio .form-kit__control-radio");
      var contents = display.querySelectorAll(":scope > .form-kit__conditional-display-content");
      choices.forEach(function (choice, i) {
        contents[i].classList.toggle("hidden", !choice.checked);
      });
    } else if (event.target.matches('[name="listable"], [name="show_posts"]')) {
      syncPostHeader(f);
    } else if (event.target.matches(".form-kit__control-image input[type=file]")) {
      uploadImage(f, event.target);
    }
  });

  document.addEventListener("input", function (event) {
    var f = form();
    if (!f || !f.contains(event.target)) {
      return;
    }
    if (event.target.name === "name") {
      f.querySelector(".current-badge-header .badge-display-name").textContent = event.target.value;
    } else if (event.target.name === "query") {
      var controls = f.querySelector(".badge-query-controls");
      if (controls) {
        controls.hidden = event.target.value.trim().length === 0;
      }
    }
  });

  document.addEventListener(
    "submit",
    function (event) {
      var root = page();
      if (!root || !root.contains(event.target)) {
        return;
      }
      event.preventDefault();
      event.stopImmediatePropagation();
      if (event.target.matches("form.badge-form")) {
        submit(event.target);
      } else if (event.target.matches("form.form-horizontal[data-badge-id]")) {
        massAward(event.target);
      }
    },
    true
  );

  document.addEventListener("click", function (event) {
    var root = page();
    if (!root) {
      return;
    }
    if (event.target.closest(".edit-groupings-btn")) {
      groupingsModal();
      return;
    }
    if (!root.contains(event.target)) {
      return;
    }
    var list = event.target.closest(".content-list");
    if (list && event.target.closest(".d-filter-controls__reset")) {
      list.querySelector(".d-filter-controls__input").value = "";
      filterList(list);
      return;
    }
    var f = form();
    if (!f || !f.contains(event.target)) {
      return;
    }
    var picker = event.target.closest(".d-icon-grid-picker-trigger");
    if (picker) {
      if (picker.getAttribute("aria-expanded") === "true") {
        Discourse.float.close("d-icon-grid-picker");
      } else {
        openIconPicker(picker);
      }
      return;
    }
    var menu = event.target.closest(".form-kit__control-menu-trigger");
    if (menu) {
      if (menu.getAttribute("aria-expanded") === "true") {
        Discourse.float.close("form-kit-menu");
      } else {
        openMenu(menu);
      }
      return;
    }
    if (event.target.closest(".badge-form__delete-badge-btn")) {
      deleteBadge();
    } else if (event.target.closest(".preview-badge")) {
      preview(f, "false");
    } else if (event.target.closest(".preview-badge-plan")) {
      preview(f, "true");
    }
  });

  Discourse.onPage(function () {
    if (!page()) {
      return;
    }
    ui.strings("admin-badges-strings");
  });
})();
