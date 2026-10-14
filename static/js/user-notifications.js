// UserNotificationsDropdown on a user's profile: its rows float under it
// as select-kit's do, and picking one sets the viewer's notification level
// for that user (PUT /u/:username/notification_level): normal or muted.
// Ignoring asks for a duration (IgnoreDurationModal), which isn't ported.
(function () {
  "use strict";

  if (!Discourse.once("user-notifications")) {
    return;
  }

  function icon(name, extra) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      " svg-icon fa-width-auto" +
      (extra ? " " + extra : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function showError(message) {
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = message || "Something went wrong.";
      error.hidden = false;
    }
  }

  var LEVELS = { changeToNormal: "normal", changeToMuted: "mute" };

  document.addEventListener(
    "toggle",
    function (event) {
      var chooser = event.target;
      if (!chooser.matches || !chooser.matches(".user-notifications-dropdown")) {
        return;
      }
      var open = chooser.open;
      chooser.classList.toggle("is-expanded", open);
      var caret = chooser.querySelector(".angle-icon");
      if (caret) {
        caret.outerHTML = icon(open ? "angle-up" : "angle-down", "angle-icon");
      }
      var header = chooser.querySelector("summary");
      var body = chooser.querySelector(".select-kit-body");
      if (!open || !body || !window.FloatingUIDOM) {
        return;
      }
      var ui = window.FloatingUIDOM;
      var small = window.innerWidth <= 450;
      body.style.position = "fixed";
      body.style.top = "0";
      body.style.left = "0";
      body.style.minWidth = small ? "" : Math.max(Math.round(header.getBoundingClientRect().width), 220) + "px";
      body.style.width = small ? window.innerWidth - 20 + "px" : "";
      ui.computePosition(header, body, {
        placement: "bottom-start",
        strategy: "fixed",
        middleware: [ui.flip({ padding: 3 }), ui.shift({ limiter: ui.limitShift() }), ui.offset(3)],
      }).then(function (result) {
        var x = small ? 10 : Math.round(result.x);
        body.style.transform = "translate(" + x + "px," + Math.round(result.y) + "px)";
      });
    },
    true
  );

  document.addEventListener("click", function (event) {
    var row = event.target.closest(".user-notifications-dropdown .select-kit-row");
    document.querySelectorAll(".user-notifications-dropdown[open]").forEach(function (chooser) {
      if (!chooser.contains(event.target) || row) {
        chooser.open = false;
      }
    });
    if (!row) {
      return;
    }
    var value = row.dataset.value;
    if (!LEVELS[value]) {
      showError("Not ported yet: ignoring a user (the ignore duration modal).");
      return;
    }
    var chooser = row.closest(".user-notifications-dropdown");
    var username = decodeURIComponent(location.pathname.split("/u/")[1].split("/")[0]);
    var base = (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
    var csrf = (document.querySelector('meta[name="csrf-token"]') || {}).content || "";
    fetch(base + "/u/" + encodeURIComponent(username) + "/notification_level.json", {
      method: "PUT",
      credentials: "same-origin",
      headers: {
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        "X-CSRF-Token": csrf,
        "X-Requested-With": "XMLHttpRequest",
      },
      body: "notification_level=" + LEVELS[value],
    })
      .then(function (response) {
        return response.json().then(function (body) {
          if (!response.ok) {
            throw new Error((body.errors || []).join(" "));
          }
        });
      })
      .then(function () {
        chooser.querySelectorAll(".select-kit-row").forEach(function (r) {
          var selected = r === row;
          r.classList.toggle("is-selected", selected);
          r.classList.toggle("is-highlighted", selected);
          r.setAttribute("aria-checked", selected ? "true" : "false");
        });
        var name = row.dataset.name;
        var summary = chooser.querySelector("summary");
        summary.dataset.name = name;
        summary.dataset.value = value;
        var choice = chooser.querySelector(".select-kit-selected-name");
        choice.dataset.name = name;
        choice.dataset.value = value;
        choice.title = name;
        choice.querySelector(".name").textContent = name;
        var rowIcon = row.querySelector(".icons .d-icon");
        var headerIcon = choice.querySelector(".d-icon");
        if (rowIcon && headerIcon) {
          headerIcon.outerHTML = rowIcon.outerHTML;
        }
      })
      .catch(function (e) {
        showError(e.message);
      });
  });
})();
