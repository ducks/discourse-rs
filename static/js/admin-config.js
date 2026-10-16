// The /admin/config areas' pages in the browser (src/admin_config_view.rs
// draws them): the flags list (AdminFlagItem's toggle, edit, and its menu
// moving or deleting a flag, the page drawn again after) and
// AdminFlagsForm (FormKit's validation, its character counters, the
// applies-to MultiSelect, the save). A route button (data-href) is
// admin-ui.js's.
(function () {
  "use strict";

  if (!Discourse.once("admin-config")) {
    return;
  }

  function ui() {
    return Discourse.adminUi;
  }

  function t(key, args) {
    return ui().t(key, args);
  }

  function tCount(key, count, args) {
    return t(key + "." + (count === 1 ? "one" : "other"), Object.assign({ count: count }, args || {}));
  }

  function escapeHtml(s) {
    return ui().escapeHtml(s);
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

  function reload() {
    go(location.pathname.replace(ui().base(), "") + location.search);
  }

  function page() {
    return document.querySelector(".admin-contents.admin-config.flags");
  }

  // --- the flags list ------------------------------------------------------

  document.addEventListener("click", function (event) {
    var root = page();
    if (!root) {
      return;
    }
    var target = event.target;
    var row = target.closest(".admin-flag-item");
    if (!row) {
      return;
    }
    var id = row.dataset.flagId;
    if (target.closest(".admin-flag-item__toggle")) {
      var toggle = target.closest(".admin-flag-item__toggle");
      var on = toggle.getAttribute("aria-checked") !== "true";
      setToggle(toggle, on);
      row.classList.remove("saved");
      ui()
        .ajax("PUT", "/admin/config/flags/" + id + "/toggle")
        .catch(function (e) {
          setToggle(toggle, !on);
          ui().failed(e);
        })
        .then(function () {
          row.classList.add("saved");
        });
      return;
    }
    if (target.closest(".admin-flag-item__edit")) {
      go("/admin/config/flags/" + id);
    }
  });

  // AdminFlagItem's DMenu: move the flag, or delete it; the page is drawn
  // again after, as the list rerenders.
  ui().menus(function (trigger, content) {
    var row = trigger.closest(".admin-flag-item");
    if (!row) {
      return;
    }
    var id = row.dataset.flagId;
    content.addEventListener("click", function (event) {
      var button = event.target.closest("button");
      if (!button || button.disabled) {
        return;
      }
      if (button.classList.contains("admin-flag-item__move-up") || button.classList.contains("admin-flag-item__move-down")) {
        var direction = button.classList.contains("admin-flag-item__move-up") ? "up" : "down";
        row.classList.remove("saved");
        ui()
          .ajax("PUT", "/admin/config/flags/" + id + "/reorder/" + direction)
          .then(reload, function (e) {
            row.classList.add("saved");
            ui().failed(e);
          });
        Discourse.float.close("flag-menu");
        return;
      }
      if (button.classList.contains("admin-flag-item__delete")) {
        row.classList.remove("saved");
        Discourse.dialog.yesNoConfirm({
          message: t("admin.config_areas.flags.delete_confirm", { name: row.dataset.flagName }),
          didConfirm: function () {
            ui()
              .ajax("DELETE", "/admin/config/flags/" + id)
              .then(reload, function (e) {
                row.classList.add("saved");
                ui().failed(e);
              });
            Discourse.float.close("flag-menu");
          },
          didCancel: function () {
            row.classList.add("saved");
            Discourse.float.close("flag-menu");
          },
        });
      }
    });
  });

  function setToggle(button, on) {
    button.setAttribute("aria-checked", on ? "true" : "false");
    var slider = button.parentNode.querySelector(".d-toggle-switch__checkbox-slider");
    slider.innerHTML = on ? ui().icon("check") : "";
  }

  // --- AdminFlagsForm --------------------------------------------------------

  function form() {
    return document.querySelector(".admin-contents.admin-config.flags form[data-flag-form]");
  }

  var submitted = false;

  function fieldValue(container) {
    var type = container.dataset.controlType;
    if (type === "custom") {
      var details = container.querySelector("details.multi-select");
      var value = details.querySelector("summary").dataset.value;
      return value ? value.split(",") : [];
    }
    var control = container.querySelector("input, textarea");
    return control.value;
  }

  // FormKit's validators for the field: required, length.
  function errorsOf(container) {
    var value = fieldValue(container);
    var empty = Array.isArray(value) ? value.length === 0 : String(value).trim() === "";
    if (container.dataset.required !== undefined && empty) {
      return [t("form_kit.errors.required")];
    }
    if (Array.isArray(value) && value.length === 0) {
      return [t("admin.config_areas.flags.form.invalid_applies_to")];
    }
    if (container.dataset.min && !Array.isArray(value)) {
      var length = value.length;
      var min = Number(container.dataset.min);
      var max = Number(container.dataset.max);
      if (length < min) {
        return [tCount("form_kit.errors.too_short", min)];
      }
      if (length > max) {
        return [tCount("form_kit.errors.too_long", max)];
      }
    }
    return [];
  }

  // The fields with errors in the order FormKit keeps them: a field
  // validated again goes to the end.
  var order = [];

  // The errors under each field and their summary above the form; `only`
  // the field that changed, or every field (a submit).
  function validate(el, only) {
    var errors = [];
    el.querySelectorAll(".form-kit__field[data-name]").forEach(function (container) {
      var own = submitted ? errorsOf(container) : [];
      var meta = container.querySelector(".form-kit__meta");
      var existing = container.querySelector(".form-kit__errors");
      if (existing) {
        existing.remove();
      }
      var control = container.querySelector("input.form-kit__control-input, textarea");
      container.classList.toggle("has-error", own.length > 0);
      if (control) {
        if (own.length) {
          control.setAttribute("aria-invalid", "true");
        } else {
          control.removeAttribute("aria-invalid");
        }
      }
      if (own.length) {
        if (!meta) {
          meta = document.createElement("div");
          meta.className = "form-kit__meta";
          container.querySelector(".form-kit__container-content").appendChild(meta);
        }
        meta.insertAdjacentHTML(
          "afterbegin",
          '<p aria-live="assertive" class="form-kit__errors"><span>' + ui().icon("triangle-exclamation") + " " + escapeHtml(own.join(", ")) + "</span></p>"
        );
        errors.push({ name: container.dataset.name, title: container.dataset.title, message: own.join(", ") });
      } else if (meta && !meta.children.length) {
        meta.remove();
      }
    });
    var byName = {};
    errors.forEach(function (e) {
      byName[e.name] = e;
    });
    if (only) {
      order = order.filter(function (n) {
        return n !== only;
      });
      if (byName[only]) {
        order.push(only);
      }
    } else {
      order = errors.map(function (e) {
        return e.name;
      });
    }
    order = order.filter(function (n) {
      return byName[n];
    });
    errors = order.map(function (n) {
      return byName[n];
    });
    var summary = el.querySelector(".form-kit__errors-summary");
    if (summary) {
      summary.remove();
    }
    if (errors.length) {
      el.insertAdjacentHTML(
        "afterbegin",
        '<div aria-live="assertive" class="form-kit__errors-summary"><h2 class="form-kit__errors-summary-title">' +
          ui().icon("triangle-exclamation") +
          " " +
          escapeHtml(tCount("form_kit.errors_summary_title", errors.length)) +
          '</h2><ul class="form-kit__errors-summary-list">' +
          errors
            .map(function (e) {
              return (
                '<li><a href="#control-' +
                escapeHtml(e.name) +
                '" rel="noopener noreferrer">' +
                escapeHtml(e.title) +
                "</a>: " +
                escapeHtml(e.message) +
                "</li>"
              );
            })
            .join("") +
          "</ul></div>"
      );
    }
    return errors.length === 0;
  }

  // The character counter (`length` validation's meta).
  function count(container) {
    var counter = container.querySelector(".form-kit__char-counter");
    if (!counter) {
      return;
    }
    var length = fieldValue(container).length;
    counter.textContent = length + "/" + container.dataset.max;
    counter.classList.toggle(
      "--insufficient",
      length < Number(container.dataset.min) || length > Number(container.dataset.max)
    );
  }

  document.addEventListener("input", function (event) {
    var el = form();
    if (!el || !el.contains(event.target) || event.target.closest(".select-kit-filter")) {
      return;
    }
    var container = event.target.closest(".form-kit__field");
    if (!container) {
      return;
    }
    count(container);
    validate(el, container.dataset.name);
  });

  document.addEventListener("submit", function (event) {
    var el = form();
    if (!el || event.target !== el) {
      return;
    }
    // Before the boosted form's own listener: the form is the script's.
    event.preventDefault();
    event.stopPropagation();
    submitted = true;
    if (!validate(el)) {
      return;
    }
    var get = function (name) {
      return el.querySelector("[name='" + name + "']");
    };
    var data = {
      name: get("name").value,
      description: get("description").value,
      enabled: get("enabled").checked,
      applies_to: fieldValue(el.querySelector("[data-name='appliesTo']")),
      require_message: get("requireMessage").checked,
      auto_action_type: get("autoActionType").checked,
    };
    var id = el.dataset.flagId;
    ui()
      .ajax(id ? "PUT" : "POST", id ? "/admin/config/flags/" + id : "/admin/config/flags", data)
      .then(function () {
        go("/admin/config/flags");
      }, ui().failed);
  }, true);

  // --- select-kit's MultiSelect ---------------------------------------------

  function choicesOf(details) {
    return JSON.parse(details.dataset.choices || "[]");
  }

  function chosenOf(details) {
    var value = details.querySelector("summary").dataset.value;
    return value ? value.split(",") : [];
  }

  function nameOf(details, id) {
    var found = choicesOf(details).find(function (c) {
      return c.id === id;
    });
    return found ? found.name : id;
  }

  function setChosen(details, chosen) {
    var summary = details.querySelector("summary");
    var names = chosen.map(function (id) {
      return nameOf(details, id);
    });
    summary.dataset.value = chosen.join(",");
    summary.dataset.name = names.join(",");
    details.querySelector(".formatted-selection").textContent = names.length ? names.join(", ") : details.dataset.none;
    details.classList.toggle("has-selection", chosen.length > 0);
    drawBody(details);
    var el = form();
    if (el) {
      validate(el, "appliesTo");
    }
  }

  function drawBody(details) {
    var body = details.querySelector(".select-kit-body");
    var input = body.querySelector(".filter-input");
    var filter = input ? input.value.toLowerCase() : "";
    var chosen = chosenOf(details);
    var html =
      '<div class="select-kit-filter multi-select-filter is-expanded"><input class="filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
      escapeHtml(t("select_kit.filter_placeholder")) +
      '" spellcheck="false" tabindex="0" type="search"></div>';
    if (chosen.length) {
      html +=
        '<div class="selected-content">' +
        chosen
          .map(function (id) {
            var name = nameOf(details, id);
            return (
              '<button aria-label="' +
              escapeHtml(t("select_kit.delete_item", { name: name })) +
              '" class="btn btn-default selected-choice tag-choice " data-name="' +
              escapeHtml(name) +
              '" data-value="' +
              escapeHtml(id) +
              '" type="button">' +
              ui().icon("xmark") +
              '<span class="d-button-label">' +
              escapeHtml(name) +
              "</span></button>"
            );
          })
          .join("") +
        "</div>";
    }
    var rows = choicesOf(details).filter(function (c) {
      return chosen.indexOf(c.id) === -1 && (!filter || c.name.toLowerCase().indexOf(filter) !== -1);
    });
    html +=
      '<ul aria-live="polite" class="select-kit-collection" role="menu">' +
      rows
        .map(function (c, i) {
          return (
            '<li aria-checked="false" role="menuitemradio" data-index="' +
            i +
            '" data-name="' +
            escapeHtml(c.name) +
            '" data-value="' +
            escapeHtml(c.id) +
            '" title="' +
            escapeHtml(c.name) +
            '" tabindex="0" class="select-kit-row"><span class="name">' +
            escapeHtml(c.name) +
            "</span></li>"
          );
        })
        .join("") +
      "</ul>";
    body.innerHTML = html;
    var again = body.querySelector(".filter-input");
    again.value = filter;
    again.focus();
    float(details);
  }

  function float(details) {
    var body = details.querySelector(".select-kit-body");
    var header = details.querySelector("summary");
    var ui2 = window.FloatingUIDOM;
    if (!ui2) {
      return;
    }
    body.style.position = "fixed";
    body.style.minWidth = Math.round(details.getBoundingClientRect().width) + "px";
    body.style.top = "0";
    body.style.left = "0";
    ui2
      .computePosition(header, body, {
        placement: "bottom-start",
        strategy: "fixed",
        middleware: [ui2.flip({ padding: 3 }), ui2.shift({ limiter: ui2.limitShift() }), ui2.offset(3)],
      })
      .then(function (result) {
        body.style.transform = "translate(" + Math.round(result.x) + "px, " + Math.round(result.y) + "px)";
        body.style.visibility = "visible";
        body.style.pointerEvents = "auto";
      });
  }

  function close(details) {
    details.open = false;
    details.classList.remove("is-expanded");
    var body = details.querySelector(".select-kit-body");
    body.innerHTML = "";
    body.removeAttribute("style");
  }

  document.addEventListener("click", function (event) {
    var details = event.target.closest && event.target.closest(".admin-contents details.select-kit.multi-select[data-choices]");
    document.querySelectorAll(".admin-contents details.select-kit.multi-select[data-choices][open]").forEach(function (d) {
      if (d !== details) {
        close(d);
      }
    });
    if (!details) {
      return;
    }
    event.preventDefault();
    var row = event.target.closest(".select-kit-row");
    if (row) {
      setChosen(details, chosenOf(details).concat([row.dataset.value]));
      return;
    }
    var choice = event.target.closest(".selected-choice");
    if (choice) {
      setChosen(
        details,
        chosenOf(details).filter(function (v) {
          return v !== choice.dataset.value;
        })
      );
      return;
    }
    if (event.target.closest(".select-kit-filter")) {
      return;
    }
    if (details.open) {
      close(details);
    } else {
      details.open = true;
      details.classList.add("is-expanded");
      drawBody(details);
    }
  });

  document.addEventListener("input", function (event) {
    var input = event.target.closest && event.target.closest("details.multi-select[data-choices] .filter-input");
    if (input) {
      drawBody(input.closest("details"));
    }
  });

  Discourse.onPage(function () {
    submitted = false;
    order = [];
    ui().strings("admin-config-strings");
  });
})();
