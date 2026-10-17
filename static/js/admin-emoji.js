// The emoji pages in the browser (src/admin_emoji_view.rs draws them):
// AdminConfigAreasEmojisList's DFilterControls (the name filter, the
// group dropdown, reset, no results), selecting emojis to export and the
// export's zip, deleting an emoji; EmojiUploader (the group ComboBox with
// allowAny, each chosen file posted to /admin/config/emoji, the first
// with the name); EmojisImport (the zip's preview, the conflicts'
// resolutions, confirm). A route button (data-href) is admin-ui.js's.
(function () {
  "use strict";

  if (!Discourse.once("admin-emoji")) {
    return;
  }

  function ui() {
    return Discourse.adminUi;
  }

  function t(key, args) {
    return ui().t(key, args);
  }

  function esc(s) {
    return ui().escapeHtml(s);
  }

  function root() {
    return document.querySelector(".admin-contents.admin-emojis");
  }

  // The page through the boosted navigation, as a route transition.
  function go(path) {
    var url = ui().base() + path;
    if (!window.htmx) {
      location.assign(url);
      return;
    }
    var link = document.createElement("a");
    link.href = url;
    link.hidden = true;
    document.getElementById("main-outlet").appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  // --- the list --------------------------------------------------------------

  var selecting = false;
  var selected = {};
  var exporting = false;

  function emojis() {
    var el = root();
    try {
      return JSON.parse((el && el.dataset.emojis) || "[]");
    } catch (_) {
      return [];
    }
  }

  function setEmojis(list) {
    root().dataset.emojis = JSON.stringify(list);
  }

  function filtered() {
    var el = root();
    var input = el.querySelector(".d-filter-controls__input");
    var dropdown = el.querySelector(".d-filter-controls__dropdown");
    var terms = (input ? input.value : "")
      .split(",")
      .map(function (s) {
        return s.trim().toLowerCase();
      })
      .filter(Boolean);
    var group = dropdown ? dropdown.value : "all";
    return emojis().filter(function (e) {
      if (
        terms.length &&
        !terms.some(function (term) {
          return e.name && String(e.name).toLowerCase().indexOf(term) !== -1;
        })
      ) {
        return false;
      }
      return group === "all" || e.group === group;
    });
  }

  function selectedCount() {
    return Object.keys(selected).length;
  }

  function button(cls, label, icon, disabled) {
    return (
      '<button class="btn ' +
      (icon ? "btn-icon-text " : "") +
      cls +
      '"' +
      (disabled ? ' disabled=""' : "") +
      ' type="button">' +
      (icon ? ui().icon(icon) : "") +
      '<span class="d-button-label">' +
      esc(label) +
      "</span></button>"
    );
  }

  function actions() {
    if (!selecting) {
      return button("btn-default admin-emoji-list__select-to-export", t("admin.emoji.select_to_export"), "download");
    }
    var count = selectedCount();
    var label = count === 0 ? t("admin.export_json.button_text") : t("admin.emoji.export_count", { count: count });
    return (
      button("btn-primary admin-emoji-list__export-btn", label, exporting ? "spinner" : "download", count === 0 || exporting) +
      button("btn-default admin-emoji-list__cancel-btn", t("cancel"))
    );
  }

  function row(e) {
    var name = esc(e.name);
    var label = function (key) {
      return '<div class="d-table__mobile-label">' + esc(t(key)) + "</div>";
    };
    var html = '<tr class="d-table__row" data-emoji-name="' + name + '">';
    if (selecting) {
      html +=
        '<td class="d-table__cell admin-emoji-list__select-col"><input aria-label="' +
        esc(t("admin.emoji.select", { name: e.name })) +
        '" class="admin-emoji-list__select" type="checkbox"' +
        (selected[e.name] ? " checked" : "") +
        "></td>";
    }
    html +=
      '<td class="d-table__cell --overview"><img alt="' +
      esc(t("admin.emoji.alt")) +
      '" class="emoji emoji-custom" src="' +
      esc(e.url) +
      '" title="' +
      name +
      '"></td><td class="d-table__cell --detail">' +
      label("admin.emoji.name") +
      ":" +
      name +
      ':</td><td class="d-table__cell --detail">' +
      label("admin.emoji.group") +
      esc(e.group) +
      '</td><td class="d-table__cell --detail">' +
      label("admin.emoji.created_by") +
      esc(e.created_by);
    html += "</td>";
    if (!selecting) {
      html +=
        '<td class="d-table__cell --controls action"><button class="btn btn-default btn-small d-table__cell-action-delete" type="button"><span class="d-button-label">' +
        esc(t("admin.emoji.delete")) +
        "</span></button></td>";
    }
    return html + "</tr>";
  }

  // DFilterControls' content for the filtered emojis, or its no results.
  function draw() {
    var el = root();
    var controls = el.querySelector(".d-filter-controls");
    if (!controls) {
      return;
    }
    var list = filtered();
    var input = controls.querySelector(".d-filter-controls__input");
    var dropdown = controls.querySelector(".d-filter-controls__dropdown");
    var active = (input && input.value.length > 0) || (dropdown && dropdown.value !== "all");
    controls.querySelector(":scope > .d-filter-controls__reset").disabled = !active;
    Array.prototype.forEach.call(dropdown.options, function (o) {
      o.classList.toggle("--selected", o.selected);
    });
    controls.querySelectorAll(
      ".admin-emoji-list__select-to-export, .admin-emoji-list__export-btn, .admin-emoji-list__cancel-btn"
    ).forEach(function (b) {
      b.remove();
    });
    controls.insertAdjacentHTML("beforeend", actions());

    var main = controls.parentNode;
    var old = main.querySelector("#custom_emoji, .d-filter-controls__no-results");
    var html;
    if (!list.length) {
      var reset = esc(t("filter_controls.reset"));
      html =
        '<div class="d-filter-controls__no-results"><p>' +
        esc(t("admin.emoji.no_results")) +
        '</p><button aria-label="' +
        reset +
        '" class="btn btn-icon-text btn-default d-filter-controls__reset" title="' +
        reset +
        '" type="button">' +
        ui().icon("arrow-rotate-left") +
        '<span class="d-button-label">' +
        reset +
        "</span></button></div>";
    } else {
      var all = list.every(function (e) {
        return selected[e.name];
      });
      var some =
        !all &&
        list.some(function (e) {
          return selected[e.name];
        });
      html = '<table class="d-table admin-emoji-list" id="custom_emoji"><thead class="d-table__header"><tr class="d-table__row">';
      if (selecting) {
        html +=
          '<th class="d-table__header-cell admin-emoji-list__select-col"><input aria-label="' +
          esc(t("admin.emoji.select_all")) +
          '" class="admin-emoji-list__select-all" type="checkbox"' +
          (all ? " checked" : "") +
          "></th>";
      }
      html +=
        '<th class="d-table__header-cell">' +
        esc(t("admin.emoji.image")) +
        '</th><th class="d-table__header-cell">' +
        esc(t("admin.emoji.name")) +
        '</th><th class="d-table__header-cell">' +
        esc(t("admin.emoji.group")) +
        '</th><th class="d-table__header-cell" colspan="3">' +
        esc(t("admin.emoji.created_by")) +
        '</th></tr></thead><tbody class="d-table__body">' +
        list.map(row).join("") +
        "</tbody></table>";
      if (some) {
        setTimeout(function () {
          var box = main.querySelector(".admin-emoji-list__select-all");
          if (box) {
            box.indeterminate = true;
          }
        });
      }
    }
    if (old) {
      old.outerHTML = html;
    } else {
      controls.insertAdjacentHTML("afterend", html);
    }
  }

  function exportSelected() {
    if (!selectedCount() || exporting) {
      return;
    }
    exporting = true;
    draw();
    var csrf = document.querySelector("meta[name='csrf-token']");
    fetch(ui().base() + "/admin/config/emoji/export", {
      method: "POST",
      credentials: "same-origin",
      headers: {
        "Content-Type": "application/json",
        "X-CSRF-Token": csrf ? csrf.content : "",
      },
      body: JSON.stringify({ names: Object.keys(selected) }),
    })
      .then(function (response) {
        if (!response.ok) {
          return response
            .json()
            .catch(function () {
              return {};
            })
            .then(function (data) {
              Discourse.dialog.alert((data.errors && data.errors[0]) || t("admin.emoji.export_failed"));
            });
        }
        return response.blob().then(function (blob) {
          var disposition = response.headers.get("Content-Disposition") || "";
          var match = /filename="([^"]+)"/.exec(disposition);
          var link = document.createElement("a");
          link.href = URL.createObjectURL(blob);
          link.download = match ? match[1] : "emojis.zip";
          document.body.appendChild(link);
          link.click();
          link.remove();
          setTimeout(function () {
            URL.revokeObjectURL(link.href);
          }, 1000);
        });
      })
      .catch(function () {
        Discourse.dialog.alert(t("admin.emoji.export_failed"));
      })
      .then(function () {
        exporting = false;
        if (root()) {
          draw();
        }
      });
  }

  document.addEventListener("input", function (event) {
    if (root() && event.target.closest(".admin-emojis .d-filter-controls__input")) {
      draw();
    }
  });

  document.addEventListener("change", function (event) {
    var el = root();
    if (!el) {
      return;
    }
    var target = event.target;
    if (target.closest(".d-filter-controls__dropdown")) {
      draw();
      return;
    }
    if (target.matches(".admin-emoji-list__select")) {
      var name = target.closest("tr").dataset.emojiName;
      if (selected[name]) {
        delete selected[name];
      } else {
        selected[name] = true;
      }
      draw();
      return;
    }
    if (target.matches(".admin-emoji-list__select-all")) {
      var list = filtered();
      var all = list.every(function (e) {
        return selected[e.name];
      });
      list.forEach(function (e) {
        if (all) {
          delete selected[e.name];
        } else {
          selected[e.name] = true;
        }
      });
      draw();
    }
  });

  document.addEventListener("click", function (event) {
    var el = root();
    if (!el) {
      return;
    }
    var target = event.target;
    if (target.closest(".d-filter-controls__reset")) {
      el.querySelector(".d-filter-controls__input").value = "";
      el.querySelector(".d-filter-controls__dropdown").value = "all";
      draw();
      return;
    }
    if (target.closest(".admin-emoji-list__select-to-export")) {
      selecting = true;
      selected = {};
      draw();
      return;
    }
    if (target.closest(".admin-emoji-list__cancel-btn")) {
      selecting = false;
      selected = {};
      draw();
      return;
    }
    if (target.closest(".admin-emoji-list__export-btn")) {
      exportSelected();
      return;
    }
    var del = target.closest(".d-table__cell-action-delete");
    if (del) {
      var name = del.closest("tr").dataset.emojiName;
      Discourse.dialog.deleteConfirm({
        title: t("admin.emoji.delete_confirm", { name: name }),
        didConfirm: function () {
          ui()
            .ajax("DELETE", "/admin/config/emoji/" + encodeURIComponent(name))
            .then(function () {
              var rest = emojis().filter(function (e) {
                return e.name !== name;
              });
              delete selected[name];
              if (!rest.length) {
                go("/admin/config/emoji");
                return;
              }
              setEmojis(rest);
              draw();
            }, ui().failed);
        },
      });
    }
  });

  // --- EmojiUploader -----------------------------------------------------------

  function uploader() {
    var el = root();
    return el && el.querySelector(".admin-emoji-form");
  }

  // ComboBox's allowAny: a filter as the group chooser opens, the typed
  // group offered as a row of its own when no group matches it.
  document.addEventListener(
    "toggle",
    function (event) {
      var details = event.target;
      if (!details.matches || !details.matches("#emoji-group-selector") || !details.open) {
        return;
      }
      setTimeout(function () {
        var body = details.querySelector(".select-kit-body");
        if (body.querySelector(".select-kit-filter")) {
          return;
        }
        body.insertAdjacentHTML(
          "afterbegin",
          '<div class="select-kit-filter is-expanded"><input autocapitalize="off" autocomplete="off" autocorrect="off" class="filter-input" placeholder="' +
            esc(t("select_kit.filter_placeholder_with_any")) +
            '" spellcheck="false" tabindex="0" type="text"></div>'
        );
        body.querySelector(".filter-input").focus();
      });
    },
    true
  );

  document.addEventListener("input", function (event) {
    var input = event.target.closest && event.target.closest("#emoji-group-selector .filter-input");
    if (!input) {
      return;
    }
    var details = input.closest("details");
    var term = input.value.trim().toLowerCase();
    var collection = details.querySelector(".select-kit-collection");
    var any = false;
    collection.querySelectorAll(".select-kit-row:not(.create-row)").forEach(function (r) {
      var match = !term || r.dataset.name.toLowerCase().indexOf(term) !== -1;
      r.hidden = !match;
      any = any || r.dataset.name.toLowerCase() === term;
    });
    var create = collection.querySelector(".create-row");
    if (create) {
      create.remove();
    }
    if (term && !any) {
      collection.insertAdjacentHTML(
        "afterbegin",
        '<li class="select-kit-row create-row" role="menuitemradio" tabindex="0" data-name="' +
          esc(input.value.trim()) +
          '" data-value="' +
          esc(input.value.trim()) +
          '" title="' +
          esc(input.value.trim()) +
          '"><span class="name">' +
          esc(input.value.trim()) +
          "</span></li>"
      );
    }
  });

  function group() {
    var details = document.getElementById("emoji-group-selector");
    return details ? details.dataset.value || "default" : "default";
  }

  var uploading = false;

  function setUploading(on, progress) {
    uploading = on;
    var el = uploader();
    if (!el) {
      return;
    }
    var choose = el.querySelector(".admin-emoji-uploader__choose");
    choose.disabled = on;
    el.querySelector(".hidden-upload-field").disabled = on;
    choose.querySelector(".d-button-label").textContent = on
      ? t("admin.emoji.uploading") + " " + (progress || 0) + "%"
      : t("admin.emoji.choose_files");
  }

  document.addEventListener("click", function (event) {
    var choose = event.target.closest(".admin-emoji-uploader__choose");
    if (choose && !uploading) {
      uploader().querySelector(".hidden-upload-field").click();
    }
  });

  document.addEventListener("change", function (event) {
    var field = event.target.closest && event.target.closest(".admin-emoji-form .hidden-upload-field");
    if (!field || !field.files.length) {
      return;
    }
    var files = Array.prototype.slice.call(field.files);
    var nameInput = document.getElementById("emoji-name");
    var chosenGroup = group();
    var done = 0;
    setUploading(true, 0);
    var chain = Promise.resolve();
    files.forEach(function (file) {
      chain = chain.then(function () {
        var data = new FormData();
        var name = nameInput.value.trim();
        if (name) {
          data.append("name", name);
          // Several files can't all take the name.
          nameInput.value = "";
        }
        if (chosenGroup && chosenGroup !== "default") {
          data.append("group", chosenGroup);
        }
        data.append("upload_type", "emoji");
        data.append("file", file, file.name);
        return ui()
          .ajax("POST", "/admin/config/emoji.json", data)
          .then(function () {
            done += 1;
            setUploading(true, Math.round((done / files.length) * 100));
          });
      });
    });
    chain.then(
      function () {
        setUploading(false);
        go("/admin/config/emoji");
      },
      function (error) {
        setUploading(false);
        field.value = "";
        ui().failed(error);
      }
    );
  });

  // --- EmojisImport ------------------------------------------------------------

  var preview = null;
  var resolutions = {};
  var CONFLICTS = ["conflict_both", "conflict_image", "conflict_group"];

  function importRoot() {
    var el = root();
    return el && el.querySelector(".admin-emoji-import");
  }

  function controlsFor(category, r) {
    if (category === "invalid") {
      return r.errors && r.errors.length ? '<p class="admin-emoji-import__error">' + esc(r.errors[0]) + "</p>" : "";
    }
    if (r.category === "identical") {
      return '<span class="admin-emoji-import__badge --skip">' + esc(t("admin.emoji.import_will_skip")) + "</span>";
    }
    if (category !== "conflict") {
      return "";
    }
    var choice = resolutions[r.name] || "incoming";
    var radio = function (value, key) {
      return (
        '<label><input name="resolution-' +
        esc(r.name) +
        '" type="radio" value="' +
        value +
        '" data-name="' +
        esc(r.name) +
        '"' +
        (choice === value ? " checked" : "") +
        ">" +
        esc(t(key)) +
        "</label>"
      );
    };
    return (
      '<div class="admin-emoji-import__conflict-resolution">' +
      radio("incoming", "admin.emoji.import_use_incoming") +
      radio("keep", "admin.emoji.import_keep_existing") +
      "</div>"
    );
  }

  function section(category, rows, counts) {
    var heading;
    if (category === "no_action") {
      heading =
        esc(t("admin.emoji.import_category.new")) +
        '<span class="admin-emoji-import__section-count">(' +
        counts.newCount +
        ")</span>" +
        (counts.identical
          ? '<small class="admin-emoji-import__section-subtitle">' +
            esc(t("admin.emoji.import_unchanged_count", { count: counts.identical })) +
            "</small>"
          : "");
    } else {
      heading =
        esc(t("admin.emoji.import_category." + category)) +
        '<span class="admin-emoji-import__section-count">(' +
        rows.length +
        ")</span>";
    }
    var conflict = category === "conflict";
    var body = rows
      .map(function (r) {
        var image = r.incoming_url
          ? '<img alt="' +
            esc(t("admin.emoji.import_incoming_alt", { name: r.name })) +
            '" class="emoji emoji-custom" src="' +
            esc(r.incoming_url) +
            '">'
          : "";
        if (conflict && r.existing_url) {
          image +=
            '<span aria-label="' +
            esc(t("admin.emoji.import_existing_alt", { name: r.name })) +
            '" class="admin-emoji-import__existing-value"><img alt="" aria-hidden="true" class="emoji emoji-custom" src="' +
            esc(r.existing_url) +
            '"></span>';
        }
        return (
          '<tr class="d-table__row"><td class="d-table__cell --overview">' +
          image +
          '</td><td class="d-table__cell --detail">:' +
          esc(r.name) +
          ":" +
          (conflict ? '<span class="admin-emoji-import__existing-value">:' + esc(r.name) + ":</span>" : "") +
          '</td><td class="d-table__cell --detail">' +
          esc(r.group) +
          (conflict
            ? '<span aria-label="' +
              esc(t("admin.emoji.import_existing_group_alt")) +
              '" class="admin-emoji-import__existing-value">' +
              esc(r.existing_group) +
              "</span>"
            : "") +
          '</td><td class="d-table__cell --controls">' +
          controlsFor(category, r) +
          "</td></tr>"
        );
      })
      .join("");
    return (
      '<div class="admin-emoji-import__section"><h3 class="admin-emoji-import__section-heading">' +
      heading +
      "</h3>" +
      (category === "invalid"
        ? '<p class="admin-emoji-import__section-description">' + esc(t("admin.emoji.import_excluded")) + "</p>"
        : "") +
      '<table class="d-table admin-emoji-import__table"><thead class="d-table__header"><tr class="d-table__row"><th class="d-table__header-cell">' +
      esc(t("admin.emoji.image")) +
      '</th><th class="d-table__header-cell">' +
      esc(t("admin.emoji.name")) +
      '</th><th class="d-table__header-cell">' +
      esc(t("admin.emoji.group")) +
      '</th><th class="d-table__header-cell"></th></tr></thead><tbody class="d-table__body">' +
      body +
      "</tbody></table></div>"
    );
  }

  function drawPreview(confirming) {
    var el = importRoot();
    var rows = preview.rows;
    var by = {
      invalid: rows.filter(function (r) {
        return r.category === "invalid";
      }),
      conflict: rows.filter(function (r) {
        return CONFLICTS.indexOf(r.category) !== -1;
      }),
      no_action: rows.filter(function (r) {
        return r.category === "new" || r.category === "identical";
      }),
    };
    var counts = {
      newCount: rows.filter(function (r) {
        return r.category === "new";
      }).length,
      identical: rows.filter(function (r) {
        return r.category === "identical";
      }).length,
    };
    var html =
      '<h2 class="admin-emoji-import__heading">' +
      esc(t("admin.emoji.import_preview_heading")) +
      '</h2><p class="admin-emoji-import__summary">' +
      esc(
        t("admin.emoji.import_summary", {
          new: counts.newCount,
          conflicts: by.conflict.length,
          unchanged: counts.identical,
          invalid: by.invalid.length,
        })
      ) +
      "</p>";
    ["invalid", "conflict", "no_action"].forEach(function (category) {
      if (by[category].length) {
        html += section(category, by[category], counts);
      }
    });
    html +=
      '<div class="admin-emoji-import__actions">' +
      button("btn-primary admin-emoji-import__confirm", t("admin.emoji.import_confirm"), confirming ? "spinner" : null, confirming) +
      button("btn-default admin-emoji-import__cancel", t("cancel")) +
      "</div>";
    el.dataset.empty = el.dataset.empty || el.innerHTML;
    el.innerHTML = html;
  }

  document.addEventListener("change", function (event) {
    var input = event.target.closest && event.target.closest(".admin-emoji-import__file-input");
    if (input) {
      var file = input.files[0];
      if (!file) {
        return;
      }
      input.disabled = true;
      input.insertAdjacentHTML("afterend", '<span class="admin-emoji-import__uploading">' + esc(t("admin.emoji.uploading")) + "</span>");
      var data = new FormData();
      data.append("file", file, file.name);
      ui()
        .ajax("POST", "/admin/config/emoji/import_preview", data)
        .then(
          function (json) {
            preview = json;
            resolutions = {};
            drawPreview(false);
          },
          function (error) {
            input.disabled = false;
            input.value = "";
            var note = input.parentNode.querySelector(".admin-emoji-import__uploading");
            if (note) {
              note.remove();
            }
            ui().failed(error);
          }
        );
      return;
    }
    var radio = event.target.closest && event.target.closest(".admin-emoji-import__conflict-resolution input");
    if (radio) {
      resolutions[radio.dataset.name] = radio.value;
    }
  });

  document.addEventListener("click", function (event) {
    var el = importRoot();
    if (!el) {
      return;
    }
    if (event.target.closest(".admin-emoji-import__cancel")) {
      preview = null;
      resolutions = {};
      el.innerHTML = el.dataset.empty;
      delete el.dataset.empty;
      return;
    }
    if (event.target.closest(".admin-emoji-import__confirm") && preview) {
      drawPreview(true);
      ui()
        .ajax("POST", "/admin/config/emoji/import_confirm", { token: preview.token, resolutions: resolutions })
        .then(
          function () {
            preview = null;
            go("/admin/config/emoji");
          },
          function (error) {
            drawPreview(false);
            ui().failed(error);
          }
        );
    }
  });

  Discourse.onPage(function () {
    selecting = false;
    selected = {};
    exporting = false;
    uploading = false;
    preview = null;
    resolutions = {};
    if (root()) {
      ui().strings("admin-emoji-strings");
    }
  });
})();
