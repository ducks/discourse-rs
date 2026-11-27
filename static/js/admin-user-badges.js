// The admin user's badges tab (templates/admin-user/badges.gjs over
// AdminUserBadgesController): the badge ComboBox is combo-box.js's,
// Grant POSTs /user_badges, Revoke confirms and DELETEs
// /user_badges/:id, and each reloads the page for the grants and the
// grantable badges as the server now sees them. Expand shows a group's
// grants in its place, latest first among the rest.
(function () {
  "use strict";

  if (!Discourse.once("admin-user-badges")) {
    return;
  }

  var ROOT = ".admin-container.user-badges";

  function base() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function csrf() {
    var meta = document.querySelector("meta[name='csrf-token']");
    return meta ? meta.content : "";
  }

  function strings() {
    var el = document.getElementById("admin-user-badges-strings");
    try {
      return el ? JSON.parse(el.textContent) : {};
    } catch (_) {
      return {};
    }
  }

  function t(key) {
    return strings()[key] || key;
  }

  function ajax(method, path, data) {
    var options = {
      method: method,
      credentials: "same-origin",
      headers: {
        Accept: "application/json",
        "X-Requested-With": "XMLHttpRequest",
        "X-CSRF-Token": csrf(),
      },
    };
    if (data) {
      options.headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      options.body = new URLSearchParams(data).toString();
    }
    return fetch(base() + path, options).then(function (response) {
      return response.text().then(function (text) {
        var json = null;
        try {
          json = text ? JSON.parse(text) : null;
        } catch (_) {}
        if (!response.ok || (json && json.failed)) {
          var message = (json && json.errors && json.errors.join(" ")) || (json && json.message) || t("generic_error");
          throw new Error(message);
        }
        return json;
      });
    });
  }

  // popupAjaxError
  function failed(e) {
    Discourse.dialog.alert(String(e.message));
  }

  function grant(form) {
    var root = form.closest(ROOT);
    var badgeId = Discourse.comboBox.value(form.querySelector("details.grant-badge-combo"));
    var reason = form.querySelector(".badge-reason").value;
    ajax("POST", "/user_badges", { username: root.dataset.username, badge_id: badgeId, reason: reason }).then(function () {
      location.reload();
    }, failed);
  }

  function revoke(button) {
    Discourse.dialog.yesNoConfirm({
      message: t("admin.badges.revoke_confirm"),
      didConfirm: function () {
        ajax("DELETE", "/user_badges/" + button.dataset.userBadgeId).then(function () {
          location.reload();
        }, failed);
      },
    });
  }

  // expandGroup: the group's grants replace it, and the rows sort by
  // granted_at, latest first (a stable sort, as groupedBadges' is).
  function expand(button) {
    var row = button.closest("tr");
    var tbody = row.parentNode;
    tbody.querySelectorAll('tr[data-grouped-in="' + button.dataset.badgeId + '"]').forEach(function (r) {
      r.hidden = false;
      r.removeAttribute("data-grouped-in");
    });
    row.remove();
    var header = tbody.querySelector("tr:first-child");
    var rows = Array.prototype.slice.call(tbody.querySelectorAll("tr[data-granted-at]"));
    var visible = rows.filter(function (r) {
      return !r.hidden;
    });
    visible
      .map(function (r, i) {
        return { row: r, i: i, at: Number(r.dataset.grantedAt) };
      })
      .sort(function (a, b) {
        return b.at - a.at || a.i - b.i;
      })
      .forEach(function (entry) {
        tbody.appendChild(entry.row);
      });
    rows
      .filter(function (r) {
        return r.hidden;
      })
      .forEach(function (r) {
        tbody.appendChild(r);
      });
    tbody.insertBefore(header, tbody.firstChild);
  }

  document.addEventListener(
    "toggle",
    function (event) {
      var chooser = event.target;
      if (!chooser.matches || !chooser.matches(ROOT + " details.grant-badge-combo")) {
        return;
      }
      Discourse.comboBox.expanded(chooser);
      if (chooser.open) {
        Discourse.comboBox.float(chooser);
        var filter = chooser.querySelector(".filter-input");
        if (filter) {
          filter.focus();
        }
      }
    },
    true
  );

  document.addEventListener("input", function (event) {
    var input = event.target.closest(ROOT + " details.grant-badge-combo .filter-input");
    if (input) {
      Discourse.comboBox.filterRows(input.closest("details"), input.value);
    }
  });

  document.addEventListener("click", function (event) {
    var row = event.target.closest(ROOT + " details.grant-badge-combo .select-kit-row");
    document.querySelectorAll(ROOT + " details.grant-badge-combo[open]").forEach(function (chooser) {
      if (!chooser.contains(event.target) || row) {
        chooser.open = false;
      }
    });
    if (row) {
      event.preventDefault();
      Discourse.comboBox.select(row.closest("details"), row);
      return;
    }
    var revokeButton = event.target.closest(ROOT + " .revoke-badge");
    if (revokeButton) {
      revoke(revokeButton);
      return;
    }
    var expandButton = event.target.closest(ROOT + " .expand-badge");
    if (expandButton) {
      expand(expandButton);
    }
  });

  document.addEventListener("submit", function (event) {
    var form = event.target.closest(ROOT + " form.grant-badge-form");
    if (form) {
      event.preventDefault();
      grant(form);
    }
  });

  // A chooser opened before this script ran missed its toggle event.
  document.querySelectorAll(ROOT + " details.grant-badge-combo[open]").forEach(function (chooser) {
    Discourse.comboBox.expanded(chooser);
    Discourse.comboBox.float(chooser);
  });
})();
