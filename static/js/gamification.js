// discourse-gamification's client on the server-drawn pages
// (src/plugins/gamification/view.rs and admin_view.rs).
//
// The leaderboard: the period chooser goes to the page for its period
// (changePeriod reloads the users from the first page), the info button
// shows the LeaderboardInfo modal, Refresh reloads a leaderboard whose
// positions weren't ready.
//
// The admin pages: New shows the create form, which creates and goes to
// the leaderboard's edit page; the trash button deletes after the
// confirmation; Recalculate shows RecalculateScoresForm, which asks for
// the recalculation and hears /recalculate_scores for the result; the
// edit form saves and goes back to the list.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.once && !Discourse.once("gamification")) {
    return;
  }

  var CHOOSER = ".leaderboard .period-chooser";
  var LEADERBOARDS = "/admin/plugins/discourse-gamification/leaderboards";

  function base() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function go(url) {
    if (!window.htmx) {
      window.location.assign(url);
      return;
    }
    var link = document.createElement("a");
    link.href = url;
    link.hidden = true;
    (document.getElementById("main-outlet") || document.body).appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  function ui() {
    return Discourse.adminUi;
  }

  function t(key, args) {
    return ui() ? ui().t(key, args) : key;
  }

  function loadStrings() {
    if (ui()) {
      ui().strings("gamification-admin-strings");
    }
  }

  // The period chooser: open and close as select-kit's, a row goes to the
  // page for its period.
  document.addEventListener(
    "toggle",
    function (event) {
      var chooser = event.target;
      if (!chooser.matches || !chooser.matches(CHOOSER)) {
        return;
      }
      if (chooser.classList.contains("is-disabled")) {
        chooser.open = false;
        return;
      }
      chooser.classList.toggle("is-expanded", chooser.open);
    },
    true
  );

  function closeChoosers(except) {
    document.querySelectorAll(CHOOSER + "[open]").forEach(function (chooser) {
      if (chooser !== except) {
        chooser.open = false;
      }
    });
  }

  function remaining(count) {
    var key =
      count === 1
        ? "gamification.daily_update_scores_availability.one"
        : "gamification.daily_update_scores_availability.other";
    return t(key, { count: count });
  }

  // RecalculateScoresForm, from the template the page carries.
  function showRecalculate() {
    var template = document.querySelector(".recalculate-scores-template");
    if (!template || !Discourse.modal) {
      return;
    }
    var left = parseInt(template.dataset.remaining, 10) || 0;
    var stop = null;
    Discourse.modal.show({
      className: "recalculate-scores-form-modal",
      title: t("gamification.recalculate"),
      body: function (bodyEl, close) {
        var content = template.content.cloneNode(true);
        var body = content.querySelector(".recalculate-modal__body");
        var footer = content.querySelector(".recalculate-modal__footer");
        bodyEl.appendChild(body);
        bodyEl.parentNode.appendChild(footer);
        footer.classList.add("d-modal__footer");
        var range = body.querySelector("#update-range");
        var custom = body.querySelector(".-custom-range");
        var dates = body.querySelector(".recalculate-modal__date-range");
        var from = body.querySelector("#custom-from-date");
        var apply = footer.querySelector("#apply-section");
        var cancel = footer.querySelector("#cancel-section");
        var text = footer.querySelector(".recalculate-modal__footer-text");

        function option() {
          return range.options[range.selectedIndex];
        }
        function sync() {
          var isCustom = range.value === "5";
          custom.hidden = !isCustom;
          dates.hidden = isCustom;
          dates.textContent = option().dataset.range || "";
          apply.disabled = left <= 0 || (isCustom && !from.value);
        }
        range.addEventListener("change", sync);
        from.addEventListener("input", sync);
        sync();

        cancel.addEventListener("click", function () {
          close();
        });
        apply.addEventListener("click", function () {
          var fromDate = range.value === "5" ? from.value : option().dataset.from;
          body.innerHTML =
            '<div class="recalculate-modal__status"><em></em></div>';
          body.querySelector("em").textContent = t("gamification.recalculating");
          apply.disabled = true;
          stop = ui().subscribe("/recalculate_scores", function (message) {
            if (!message || !message.success) {
              return;
            }
            left = message.remaining;
            template.dataset.remaining = String(left);
            body.innerHTML =
              '<div class="recalculate-modal__status is-success">' +
              ui().icon("check") +
              " <span></span></div>";
            body.querySelector("span").textContent = t("gamification.completed");
            cancel.querySelector(".d-button-label").textContent = t("gamification.close");
            text.textContent = remaining(left);
          });
          ui()
            .ajax("PUT", "/admin/plugins/gamification/recalculate-scores.json", {
              from_date: fromDate,
            })
            .catch(ui().failed);
        });
      },
      onClose: function () {
        if (stop) {
          stop();
        }
      },
    });
  }

  function field(form, name) {
    return form.querySelector('[name="' + name + '"]');
  }

  function multi(form, name) {
    var select = field(form, name);
    if (!select) {
      return [];
    }
    return Array.prototype.filter
      .call(select.options, function (o) {
        return o.selected;
      })
      .map(function (o) {
        return o.value;
      });
  }

  // AdminEditLeaderboard#save: the overrides given, the rest as they are.
  function saveLeaderboard(form) {
    var data = {
      name: field(form, "name").value,
      from_date: field(form, "from_date").value,
      to_date: field(form, "to_date").value,
      included_groups_ids: multi(form, "included_groups_ids"),
      excluded_groups_ids: multi(form, "excluded_groups_ids"),
      visible_to_groups_ids: multi(form, "visible_to_groups_ids"),
      default_period: field(form, "default_period").value,
      period_filter_disabled:
        form.querySelector('.d-toggle-switch__checkbox[data-name="period_filter_disabled"]')
          .getAttribute("aria-checked") === "true",
      scorable_category_ids: multi(form, "scorable_category_ids"),
    };
    var overrides = {};
    var any = false;
    form.querySelectorAll('input[name^="score_override_"]').forEach(function (input) {
      if (input.value !== "") {
        overrides[input.name.replace("score_override_", "")] = parseInt(input.value, 10);
        any = true;
      }
    });
    if (any) {
      data.score_overrides = overrides;
    }
    ui()
      .ajax("PUT", "/admin/plugins/gamification/leaderboard/" + form.dataset.leaderboardId, data)
      .then(function () {
        ui().toast(t("gamification.leaderboard.save_success"));
        go(base() + LEADERBOARDS);
      })
      .catch(ui().failed);
  }

  function createLeaderboard(form) {
    var container = form.closest(".new-leaderboard-container");
    var name = field(form, "name").value;
    if (!name.trim()) {
      field(form, "name").focus();
      return;
    }
    ui()
      .ajax("POST", "/admin/plugins/gamification/leaderboard", {
        name: name,
        created_by_id: container.dataset.createdById,
      })
      .then(function (leaderboard) {
        ui().toast(t("gamification.leaderboard.create_success"));
        go(base() + LEADERBOARDS + "/" + leaderboard.id);
      })
      .catch(ui().failed);
  }

  function showCreate(show) {
    var container = document.querySelector(".new-leaderboard-container");
    if (!container) {
      return;
    }
    container.hidden = !show;
    var empty = document.querySelector(".admin-plugin-config-area__empty-list");
    if (empty) {
      empty.hidden = show;
    }
    if (show) {
      var input = container.querySelector('[name="name"]');
      if (input) {
        input.focus();
      }
    }
  }

  document.addEventListener("click", function (event) {
    var target = event.target;
    if (!target.closest) {
      return;
    }
    var row = target.closest(CHOOSER + " .select-kit-row");
    if (row) {
      var chooser = row.closest(CHOOSER);
      chooser.open = false;
      if (row.dataset.value !== chooser.querySelector("summary").dataset.value) {
        go(location.pathname + "?period=" + encodeURIComponent(row.dataset.value));
      }
      return;
    }
    closeChoosers(target.closest(CHOOSER));

    if (target.closest(".leaderboard .leaderboard-info")) {
      var template = document.querySelector(".leaderboard-info-template");
      if (template && Discourse.modal) {
        Discourse.modal.show({
          className: "leaderboard-info-modal",
          title: template.dataset.title,
          body: function (bodyEl) {
            bodyEl.appendChild(template.content.cloneNode(true));
          },
        });
      }
      return;
    }
    if (target.closest(".leaderboard__not-ready .refresh")) {
      go(location.pathname + location.search);
      return;
    }

    if (!document.querySelector(".discourse-gamification")) {
      return;
    }
    loadStrings();
    if (target.closest(".leaderboard-admin__btn-new, .leaderboard-admin__cta-new")) {
      showCreate(true);
      return;
    }
    if (target.closest(".new-leaderboard__cancel")) {
      showCreate(false);
      return;
    }
    if (target.closest(".leaderboard-admin__btn-recalculate")) {
      showRecalculate();
      return;
    }
    var del = target.closest(".leaderboard-admin__delete");
    if (del) {
      var id = del.dataset.leaderboardId;
      Discourse.dialog.deleteConfirm({
        message: t("gamification.leaderboard.confirm_destroy"),
        didConfirm: function () {
          return ui()
            .ajax("DELETE", "/admin/plugins/gamification/leaderboard/" + id)
            .then(function () {
              ui().toast(t("gamification.leaderboard.delete_success"));
              var tr = document.getElementById("leaderboard-admin__row-" + id);
              if (tr) {
                tr.remove();
              }
            })
            .catch(ui().failed);
        },
      });
      return;
    }
    var toggle = target.closest(".edit-create-leaderboard-form .d-toggle-switch__checkbox");
    if (toggle) {
      var on = toggle.getAttribute("aria-checked") !== "true";
      toggle.setAttribute("aria-checked", on ? "true" : "false");
      var slider = toggle.parentNode.querySelector(".d-toggle-switch__checkbox-slider");
      if (slider) {
        slider.innerHTML = on ? ui().icon("check") : "";
      }
    }
  });

  document.addEventListener("submit", function (event) {
    var form = event.target;
    if (form.matches && form.matches(".edit-create-leaderboard-form")) {
      event.preventDefault();
      loadStrings();
      saveLeaderboard(form);
    } else if (form.matches && form.matches(".new-leaderboard-form")) {
      event.preventDefault();
      loadStrings();
      createLeaderboard(form);
    }
  });

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      closeChoosers(null);
    }
  });
})();
