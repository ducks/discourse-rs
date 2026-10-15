// The multi-select choosers of the preferences (select-kit's
// MultiSelect): CategorySelector (rows from #prefs-category-rows, the
// categories selected under any level left out), TagChooser
// (/tags/filter/search, the tags of every level left out) and UserChooser
// (/u/search/users, ranked as lib/user-search ranks them). What a chooser
// holds is its data-items, [value, name] pairs, which preferences.js
// saves. Also IgnoredUserList and the IgnoreDurationModal it opens
// (EmailGroupUserChooser and DFutureDateInput's time shortcuts), which the
// profile's notification level dropdown opens too.
(function () {
  "use strict";

  if (!Discourse.once("prefs-choosers")) {
    return;
  }

  function strings() {
    var el = document.getElementById("prefs-choosers-strings");
    try {
      return el ? JSON.parse(el.textContent) : {};
    } catch (e) {
      return {};
    }
  }

  function meta(name) {
    return (document.querySelector('meta[name="' + name + '"]') || {}).content || "";
  }

  function escapeHtml(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
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

  function count(key, n) {
    var s = strings();
    return String((n === 1 ? s[key + "_one"] : s[key + "_other"]) || "").replace("%{count}", n);
  }

  function getJson(url) {
    return fetch(meta("discourse-base-uri") + url, {
      credentials: "same-origin",
      headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
    }).then(function (r) {
      if (!r.ok) {
        throw new Error(r.status);
      }
      return r.json();
    });
  }

  function items(chooser) {
    try {
      return JSON.parse(chooser.dataset.items || "[]");
    } catch (e) {
      return [];
    }
  }

  // The choices of every chooser of the kind on the page: what a
  // CategorySelector's blockedCategories and a TagChooser's blockedTags are.
  function blocked(chooser) {
    var out = [];
    var scope = chooser.closest(".d-modal") || document;
    scope.querySelectorAll('details.multi-select[data-chooser="' + chooser.dataset.chooser + '"]').forEach(function (c) {
      items(c).forEach(function (i) {
        out.push(String(i[1]).toLowerCase());
      });
    });
    return out;
  }

  function setItems(chooser, list) {
    var s = strings();
    chooser.dataset.items = JSON.stringify(list);
    var summary = chooser.querySelector("summary");
    var names = list.map(function (i) {
      return i[1];
    });
    summary.dataset.name = names.join(",");
    if (chooser.dataset.chooser !== "email-user") {
      summary.dataset.value = list
        .map(function (i) {
          return i[0];
        })
        .join(",");
    }
    chooser.querySelector(".formatted-selection").textContent = names.length ? names.join(", ") : s.default_header;
    chooser.classList.toggle("has-selection", names.length > 0);
    chooser.dataset.dirty = "true";
    chooser.dispatchEvent(new Event("change", { bubbles: true }));
    syncPrecedence();
  }

  // TrackingController#showMutePrecedenceSetting
  function syncPrecedence() {
    var setting = document.querySelector(".user-preferences__watched-precedence-over-muted");
    if (!setting) {
      return;
    }
    var has = function (key) {
      var chooser = document.querySelector('details.multi-select[data-pref="' + key + '"]');
      return !!chooser && items(chooser).length > 0;
    };
    var shown =
      (has("watched_category_ids") && has("muted_tags")) || (has("watched_tags") && has("muted_category_ids"));
    setting.style.display = shown ? "" : "none";
  }

  function float(chooser) {
    var body = chooser.querySelector(".select-kit-body");
    var header = chooser.querySelector("summary");
    var ui = window.FloatingUIDOM;
    if (!ui) {
      return;
    }
    body.style.position = "fixed";
    body.style.top = "0";
    body.style.left = "0";
    body.style.minWidth = Math.round(header.getBoundingClientRect().width) + "px";
    ui.computePosition(header, body, {
      placement: "bottom-start",
      strategy: "fixed",
      middleware: [ui.flip({ padding: 3 }), ui.shift({ limiter: ui.limitShift() }), ui.offset(3)],
    }).then(function (result) {
      body.style.transform = "translate(" + Math.round(result.x) + "px," + Math.round(result.y) + "px)";
      body.style.visibility = "visible";
    });
  }

  function selectedContent(chooser, list) {
    if (!list.length) {
      return "";
    }
    var s = strings();
    var kind = chooser.dataset.chooser;
    return (
      '<div class="selected-content">' +
      list
        .map(function (i) {
          var label;
          if (kind === "category") {
            var row = document.querySelector('#prefs-category-rows');
            var badge = row ? row.content.querySelector('.category-row[data-value="' + i[0] + '"] .category-status') : null;
            label = badge ? lastBadge(badge) : '<span class="d-button-label">' + escapeHtml(i[1]) + "</span>";
          } else {
            label = '<span class="d-button-label">' + escapeHtml(i[1]) + "</span>";
          }
          return (
            '<button aria-label="' +
            escapeHtml(String(s.delete_item || "").replace("%{name}", i[1])) +
            '" class="btn btn-default selected-choice tag-choice ' +
            (kind === "category" ? "selected-choice-category" : "") +
            '" data-name="' +
            escapeHtml(i[1]) +
            '" data-value="' +
            escapeHtml(i[0]) +
            '" type="button">' +
            icon("xmark") +
            label +
            "</button>"
          );
        })
        .join("") +
      "</div>"
    );
  }

  // The category's own badge, without its topic count.
  function lastBadge(status) {
    var wrappers = status.querySelectorAll(".badge-category__wrapper");
    var badge = wrappers[wrappers.length - 1].cloneNode(true);
    var countEl = badge.querySelector(".topic-count");
    if (countEl) {
      countEl.remove();
    }
    return badge.outerHTML;
  }

  function filterHtml(chooser) {
    var s = strings();
    var kind = chooser.dataset.chooser;
    return (
      '<div class="select-kit-filter multi-select-filter' +
      (kind === "email-user" ? " email-group-user-chooser-filter" : "") +
      ' is-expanded"><input class="ember-text-field filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
      escapeHtml(kind === "tag" ? s.tags_placeholder : s.filter_placeholder) +
      '" spellcheck="false" tabindex="0" type="search"></div>'
    );
  }

  // The body: the filter, the choices, the rows for the filter.
  function render(chooser, rows, showCount) {
    var body = chooser.querySelector(".select-kit-body");
    var input = body.querySelector(".filter-input");
    var term = input ? input.value : "";
    var list = items(chooser);
    var max = parseInt(chooser.dataset.maximum || "0", 10);
    var html = filterHtml(chooser);
    if (chooser.dataset.chooser !== "email-user") {
      html += selectedContent(chooser, list);
    }
    if (!(max && list.length >= max) && rows !== null) {
      html += '<ul aria-live="polite" class="select-kit-collection" role="menu">' + rows + "</ul>";
      if (showCount) {
        var n = (rows.match(/select-kit-row/g) || []).length;
        html += '<span class="results-count" role="alert">' + escapeHtml(count("results", n)) + "</span>";
      }
    }
    body.innerHTML = html;
    var fresh = body.querySelector(".filter-input");
    fresh.value = term;
    fresh.focus();
    float(chooser);
  }

  function categoryRows(chooser, term) {
    var template = document.getElementById("prefs-category-rows");
    if (!template) {
      return "";
    }
    var skip = blocked(chooser);
    var needle = term.toLowerCase();
    var out = [];
    template.content.querySelectorAll(".category-row").forEach(function (row) {
      var name = row.dataset.name;
      if (skip.indexOf(name.toLowerCase()) !== -1) {
        return;
      }
      if (needle && name.toLowerCase().indexOf(needle) === -1) {
        return;
      }
      var clone = row.cloneNode(true);
      clone.dataset.index = out.length;
      if (needle && !out.length) {
        clone.classList.add("is-highlighted");
      }
      out.push(clone.outerHTML);
    });
    return out.join("");
  }

  function tagRows(chooser, term) {
    var s = strings();
    var params = new URLSearchParams({ q: term, limit: String(s.max_tag_search_results || 5) });
    blocked(chooser)
      .slice(0, 100)
      .forEach(function (name) {
        params.append("selected_tags[]", name);
      });
    return getJson("/tags/filter/search.json?" + params.toString()).then(function (json) {
      var skip = blocked(chooser);
      var results = (json.results || []).filter(function (r) {
        return skip.indexOf(String(r.name).toLowerCase()) === -1;
      });
      if (s.tags_sort_alphabetically) {
        results.sort(function (a, b) {
          return a.name.localeCompare(b.name);
        });
      }
      var seen = {};
      return results
        .filter(function (r) {
          if (seen[r.name]) {
            return false;
          }
          seen[r.name] = true;
          return true;
        })
        .map(function (r, i) {
          return (
            '<li aria-checked="false" role="menuitemradio" data-index="' +
            i +
            '" data-name="' +
            escapeHtml(r.name) +
            '" data-value="' +
            escapeHtml(r.id !== undefined ? r.id : r.name) +
            '" title="' +
            escapeHtml(r.name) +
            '" tabindex="0" class="select-kit-row tag-chooser-row' +
            (term && i === 0 ? " is-highlighted" : "") +
            '"><a data-tag-name="' +
            escapeHtml(String(r.name).toLowerCase()) +
            '" class="discourse-tag ' +
            escapeHtml(s.tag_style || "simple") +
            '">' +
            escapeHtml(r.text || r.name) +
            "</a>" +
            (r.count ? '<span class="discourse-tag-count">x' + r.count + "</span>" : "") +
            "</li>"
          );
        })
        .join("");
    });
  }

  function lowerCaseIncludes(string, term) {
    return !!(string && term && string.toLowerCase().indexOf(term.toLowerCase()) !== -1);
  }

  // lib/user-search's organizeResults, users only.
  function organizeUsers(users, term, exclude, limit) {
    term = ((term || "").trim().split(/\s/, 1) || [])[0];
    users = users.filter(function (u) {
      return exclude.indexOf(u.username) === -1;
    });
    users.forEach(function (u) {
      u.isMetadataMatch = !!term && !lowerCaseIncludes(u.username, term) && !lowerCaseIncludes(u.name, term);
    });
    var seen = {};
    var lower = function (v) {
      return v == null ? v : String(v).toLowerCase();
    };
    var pick = function (test) {
      return users.filter(function (u) {
        if (seen[u.username] || !test(u)) {
          return false;
        }
        seen[u.username] = true;
        return true;
      });
    };
    return []
      .concat(
        pick(function (u) {
          return lower(u.username) === lower(term);
        }),
        pick(function (u) {
          return !u.isMetadataMatch && !(term && !lowerCaseIncludes(u.username, term));
        }),
        pick(function (u) {
          return lower(u.name) === lower(term) && !u.isMetadataMatch;
        }),
        pick(function (u) {
          return lowerCaseIncludes(u.name, term) && !u.isMetadataMatch;
        }),
        users.filter(function (u) {
          return u.isMetadataMatch;
        })
      )
      .slice(0, limit);
  }

  var USER_IGNORE = /([ -⁯⸀-⹿\\'!"#$%&()*,\/:;<=>?\[\]^`{|}~])|\s\s|^\s$|^[^+]*\+[^@]*$/;

  function avatar(template, username) {
    return (
      '<img alt="" width="24" height="24" src="' +
      escapeHtml(meta("discourse-base-uri") + String(template || "").replace("{size}", "24").replace(meta("discourse-base-uri"), "")) +
      '" class="avatar" title="' +
      escapeHtml(username) +
      '">'
    );
  }

  function userRows(chooser, term) {
    term = term.replace(/^@/, "");
    if (term === "" || term.indexOf("@") !== -1 || USER_IGNORE.test(term)) {
      return Promise.resolve(null);
    }
    var s = strings();
    var exclude = items(chooser).map(function (i) {
      return i[0];
    });
    exclude.push(s.current_username);
    var params = new URLSearchParams({ term: term, include_groups: "false", limit: "6" });
    return getJson("/u/search/users.json?" + params.toString()).then(function (json) {
      var email = chooser.dataset.chooser === "email-user";
      return organizeUsers(json.users || [], term, exclude, 6)
        .map(function (u, i) {
          var name = u.name || u.username;
          var row =
            '<li aria-checked="false" role="menuitemradio" data-index="' +
            i +
            '" data-name="' +
            escapeHtml(name) +
            '" data-value="' +
            escapeHtml(u.username) +
            '" title="' +
            escapeHtml(name) +
            '" tabindex="0" class="select-kit-row ' +
            (email ? "email-group-user-chooser-row" : "user-row") +
            (i === 0 ? " is-highlighted" : "") +
            '">' +
            avatar(u.avatar_template, u.username);
          if (email) {
            row +=
              '<div class="email-group-user-chooser__user --username-first"><span class="identifier">' +
              escapeHtml(u.username) +
              '</span><span class="name">' +
              escapeHtml(u.name || "") +
              "</span></div>";
          } else {
            row +=
              '<span class="username">' +
              escapeHtml(u.username) +
              '</span><span class="name">' +
              escapeHtml(u.name || "") +
              "</span>";
          }
          return row + "</li>";
        })
        .join("");
    });
  }

  var searchTimer = null;

  function refresh(chooser) {
    var input = chooser.querySelector(".select-kit-body .filter-input");
    var term = input ? input.value : "";
    var kind = chooser.dataset.chooser;
    if (kind === "category") {
      render(chooser, categoryRows(chooser, term), term !== "");
      return;
    }
    clearTimeout(searchTimer);
    searchTimer = setTimeout(
      function () {
        (kind === "tag" ? tagRows(chooser, term) : userRows(chooser, term))
          .then(function (rows) {
            if (chooser.open) {
              render(chooser, rows, kind !== "tag" && rows !== null);
            }
          })
          .catch(function () {});
      },
      kind === "tag" ? 0 : 300
    );
  }

  function choose(chooser, row) {
    var list = items(chooser);
    var max = parseInt(chooser.dataset.maximum || "0", 10);
    var item = [row.dataset.value, chooser.dataset.chooser === "user" || chooser.dataset.chooser === "email-user" ? row.dataset.value : row.dataset.name];
    if (max === 1) {
      list = [item];
    } else {
      list.push(item);
    }
    setItems(chooser, list);
    var input = chooser.querySelector(".select-kit-body .filter-input");
    if (input) {
      input.value = "";
    }
    if (max && list.length >= max) {
      chooser.open = false;
      return;
    }
    refresh(chooser);
  }

  document.addEventListener(
    "toggle",
    function (event) {
      var chooser = event.target;
      if (!chooser.matches || !chooser.matches("details.multi-select[data-chooser]")) {
        return;
      }
      chooser.classList.toggle("is-expanded", chooser.open);
      var body = chooser.querySelector(".select-kit-body");
      if (!chooser.open) {
        body.innerHTML = "";
        body.removeAttribute("style");
        return;
      }
      render(chooser, chooser.dataset.chooser === "category" ? categoryRows(chooser, "") : null, false);
      if (chooser.dataset.chooser === "tag") {
        refresh(chooser);
      }
    },
    true
  );

  document.addEventListener("input", function (event) {
    var input = event.target.closest("details.multi-select[data-chooser] .filter-input");
    if (input) {
      refresh(input.closest("details"));
    }
  });

  document.addEventListener("keydown", function (event) {
    var input = event.target.closest("details.multi-select[data-chooser] .filter-input");
    if (!input) {
      return;
    }
    var chooser = input.closest("details");
    if (event.key === "Enter") {
      event.preventDefault();
      var row = chooser.querySelector(".select-kit-row.is-highlighted") || chooser.querySelector(".select-kit-collection .select-kit-row");
      if (row) {
        choose(chooser, row);
      }
    } else if (event.key === "Escape") {
      chooser.open = false;
    }
  });

  document.addEventListener("click", function (event) {
    var row = event.target.closest("details.multi-select[data-chooser] .select-kit-collection .select-kit-row");
    if (row) {
      event.preventDefault();
      choose(row.closest("details"), row);
      return;
    }
    var choice = event.target.closest("details.multi-select[data-chooser] .selected-choice");
    if (choice) {
      event.preventDefault();
      var chooser = choice.closest("details");
      setItems(
        chooser,
        items(chooser).filter(function (i) {
          return String(i[0]) !== choice.dataset.value;
        })
      );
      refresh(chooser);
      return;
    }
    document.querySelectorAll("details.multi-select[data-chooser][open]").forEach(function (chooser) {
      if (!chooser.contains(event.target)) {
        chooser.open = false;
      }
    });
    var add = event.target.closest(".user-ignore .add-ignored-user");
    if (add) {
      ignoreModal(null, function (username) {
        addIgnored(username);
      });
      return;
    }
    var remove = event.target.closest(".user-ignore .remove-ignored-user");
    if (remove) {
      removeIgnored(remove.closest(".ignored-user-list-item"));
    }
  });

  // --- IgnoredUserList --------------------------------------------------

  function prefsUser() {
    var f = document.querySelector(".user-preferences form.form-vertical");
    return f ? { username: f.dataset.username, id: f.dataset.userId } : null;
  }

  function notificationLevel(username, data) {
    var body = Object.keys(data)
      .map(function (k) {
        return encodeURIComponent(k) + "=" + encodeURIComponent(data[k]);
      })
      .join("&");
    return fetch(meta("discourse-base-uri") + "/u/" + encodeURIComponent(username) + "/notification_level.json", {
      method: "PUT",
      credentials: "same-origin",
      headers: {
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        "X-CSRF-Token": meta("csrf-token"),
        "X-Requested-With": "XMLHttpRequest",
      },
      body: body,
    }).then(function (r) {
      return r.json().then(function (json) {
        if (!r.ok) {
          throw new Error((json.errors || []).join(" ") || r.statusText);
        }
        return json;
      });
    });
  }

  function popupAjaxError(e) {
    if (Discourse.dialog) {
      Discourse.dialog.popupAjaxError(e.message);
    }
  }

  function addIgnored(username) {
    var list = document.querySelector(".user-ignore .ignored-list");
    if (!list) {
      return;
    }
    var names = Array.prototype.map.call(list.querySelectorAll(".ignored-user-name"), function (n) {
      return n.textContent;
    });
    if (names.indexOf(username) !== -1) {
      return;
    }
    if (!names.length) {
      list.innerHTML = "";
    }
    var item = document.createElement("div");
    item.innerHTML =
      '<div class="ignored-user-list-item"><span class="ignored-user-name">' +
      escapeHtml(username) +
      '</span><button class="btn no-text btn-icon remove-ignored-user" type="button">' +
      icon("xmark") +
      '<span aria-hidden="true">&#8203;</span></button></div>';
    list.appendChild(item);
  }

  function removeIgnored(item) {
    var username = item.querySelector(".ignored-user-name").textContent;
    var list = item.closest(".ignored-list");
    item.parentNode.remove();
    if (!list.querySelector(".ignored-user-list-item")) {
      list.textContent = list.dataset.empty;
    }
    var user = prefsUser();
    notificationLevel(username, { notification_level: "normal", acting_user_id: user ? user.id : "" }).catch(
      popupAjaxError
    );
  }

  // --- IgnoreDurationModal ----------------------------------------------

  var momentLoading = null;

  function loadMoment() {
    if (window.moment && window.moment.tz) {
      return Promise.resolve();
    }
    if (momentLoading) {
      return momentLoading;
    }
    var s = strings();
    var load = function (src) {
      return new Promise(function (resolve, reject) {
        var script = document.createElement("script");
        script.src = src;
        script.onload = resolve;
        script.onerror = reject;
        document.head.appendChild(script);
      });
    };
    momentLoading = (window.moment ? Promise.resolve() : load(s.moment)).then(function () {
      return window.moment.tz ? null : load(s.moment_timezone);
    });
    return momentLoading;
  }

  // lib/time-utils and lib/time-shortcut: the shortcuts the modal offers.
  function shortcuts() {
    var s = strings();
    var tz = s.timezone || window.moment.tz.guess();
    var now = function () {
      return window.moment.tz(tz);
    };
    var startOfDay = function (m) {
      return m.hour(8).startOf("hour");
    };
    var laterToday = function () {
      var later = now().add(3, "hours");
      if (later.hour() >= 18) {
        return later.hour(18).startOf("hour");
      }
      return later.minutes() < 30 ? later.startOf("hour") : later.add(30, "minutes").startOf("hour");
    };
    var day = now().day();
    var list = [
      { id: "later_today", icon: "far-clock", label: "later_today", time: laterToday(), format: "time" },
      { id: "tomorrow", icon: "calendar-day", label: "tomorrow", time: startOfDay(now().add(1, "day")), format: "time_short_day" },
      { id: "later_this_week", icon: "calendar-days", label: "later_this_week", time: startOfDay(now().add(2, "days")), format: "time_short_day" },
      { id: "this_weekend", icon: "bed", label: "this_weekend", time: startOfDay(now().day(6)), format: "time_short_day" },
      {
        id: "start_of_next_business_week",
        icon: "briefcase",
        label: day === 1 || day === 0 ? "start_of_next_business_week_alt" : "start_of_next_business_week",
        time: startOfDay(now().add(7, "days")).day(1),
        format: "long_no_year",
      },
      { id: "two_weeks", icon: "far-calendar-plus", label: "two_weeks", time: startOfDay(now().add(2, "weeks").day(1)), format: "long_no_year" },
      { id: "next_month", icon: "far-calendar-plus", label: "next_month", time: startOfDay(now().add(1, "month").startOf("month")), format: "long_no_year" },
      { id: "two_months", icon: "far-calendar-plus", label: "two_months", time: startOfDay(now().add(2, "months").startOf("month")), format: "long_no_year" },
      { id: "three_months", icon: "far-calendar-plus", label: "three_months", time: startOfDay(now().add(3, "months").startOf("month")), format: "long_no_year" },
      { id: "four_months", icon: "far-calendar-plus", label: "four_months", time: startOfDay(now().add(4, "months").startOf("month")), format: "long_no_year" },
      { id: "six_months", icon: "far-calendar-plus", label: "six_months", time: startOfDay(now().add(6, "months").startOf("month")), format: "long_with_year" },
      { id: "one_year", icon: "far-calendar-plus", label: "one_year", time: startOfDay(now().add(1, "years").startOf("month")), format: "long_with_year" },
      { id: "forever", icon: "gavel", label: "forever", time: startOfDay(now().add(1000, "years").startOf("month")), format: "long_with_year" },
    ];
    // hideDynamicTimeShortcuts
    var hide = {};
    if (now().hour() >= 17) {
      hide.later_today = true;
    }
    if (day >= 4) {
      hide.later_this_week = true;
    }
    if (!s.suggest_weekends || day === 5 || day === 6 || day === 0) {
      hide.this_weekend = true;
    }
    return list
      .filter(function (x) {
        return !hide[x.id];
      })
      .map(function (x) {
        x.name = s.time_shortcut[x.label];
        x.formatted = x.time.format(s.dates[x.format]);
        return x;
      });
  }

  function dateRows(list) {
    return list
      .map(function (x, i) {
        return (
          '<li aria-checked="false" role="menuitemradio" data-index="' +
          i +
          '" data-name="' +
          escapeHtml(x.name) +
          '" data-value="' +
          x.id +
          '" title="' +
          escapeHtml(x.name) +
          '" tabindex="0" class="select-kit-row future-date-input-selector-row"><div class="future-date-input-selector-icons">' +
          icon(x.icon) +
          '</div><span class="name">' +
          escapeHtml(x.name) +
          '</span><span class="future-date-input-selector-datetime">' +
          escapeHtml(x.formatted) +
          "</span></li>"
        );
      })
      .join("");
  }

  // `onUserIgnored(username)` once the level is set; `username` given
  // (the profile's dropdown) leaves out the chooser.
  function ignoreModal(username, onUserIgnored) {
    var s = strings();
    loadMoment().then(function () {
      var list = shortcuts();
      var selected = null;
      var acting = prefsUser();
      Discourse.modal.show({
        className: "ignore-duration-with-username-modal",
        title: s.ignore_title,
        body: function (body) {
          var html = "";
          if (!username) {
            html +=
              '<div class="controls tracking-controls"><label>' +
              icon("far-eye-slash", "icon") +
              " " +
              escapeHtml(s.ignore_username) +
              '</label><details class="select-kit multi-select user-chooser email-group-user-chooser" data-chooser="email-user" data-maximum="1" data-items="[]"><summary data-name="" tabindex="0" class="select-kit-header multi-select-header"><div class="select-kit-header-wrapper"><span class="formatted-selection">' +
              escapeHtml(s.default_header) +
              "</span>" +
              icon("plus", "angle-icon") +
              '</div></summary><div class="select-kit-body"></div></details></div>';
          }
          html +=
            '<div class="future-date-input"><div class="control-group"><label>' +
            escapeHtml(s.ignore_when) +
            '</label><details class="select-kit single-select combobox combo-box future-date-input-selector"><summary aria-label="' +
            escapeHtml(s.filter_by.replace("%{name}", s.select_timeframe)) +
            '" name="' +
            escapeHtml(s.filter_by.replace("%{name}", s.select_timeframe)) +
            '" data-name="' +
            escapeHtml(s.select_timeframe) +
            '" tabindex="0" class="select-kit-header single-select-header combo-box-header future-date-input-selector-header"><div class="select-kit-header-wrapper"><div class="select-kit-selected-name selected-name choice" data-name="' +
            escapeHtml(s.select_timeframe) +
            '" title="' +
            escapeHtml(s.select_timeframe) +
            '"><span class="name">' +
            escapeHtml(s.select_timeframe) +
            "</span></div>" +
            icon("angle-down", "angle-icon") +
            '</div></summary><div class="select-kit-body"><div class="select-kit-filter is-expanded"><input class="ember-text-field filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
            escapeHtml(s.filter_placeholder) +
            '" spellcheck="false" tabindex="0" type="search">' +
            icon("magnifying-glass") +
            '</div><ul aria-live="polite" class="select-kit-collection" role="menu">' +
            dateRows(list) +
            "</ul></div></details></div></div><p>" +
            escapeHtml(s.ignore_note) +
            "</p>";
          body.innerHTML = html;
          var dates = body.querySelector(".future-date-input-selector");
          dates.addEventListener("toggle", function () {
            dates.classList.toggle("is-expanded", dates.open);
            var caret = dates.querySelector(".angle-icon");
            if (caret) {
              caret.outerHTML = icon(dates.open ? "angle-up" : "angle-down", "angle-icon");
            }
            if (dates.open) {
              float(dates);
              var filter = dates.querySelector(".filter-input");
              filter.value = "";
              dates.querySelectorAll(".select-kit-row").forEach(function (r) {
                r.hidden = false;
              });
              filter.focus();
            }
          });
          dates.querySelector(".filter-input").addEventListener("input", function (e) {
            var needle = e.target.value.toLowerCase();
            dates.querySelectorAll(".select-kit-row").forEach(function (r) {
              r.hidden = needle !== "" && r.dataset.name.toLowerCase().indexOf(needle) === -1;
            });
          });
          dates.querySelector(".select-kit-collection").addEventListener("click", function (e) {
            var row = e.target.closest(".select-kit-row");
            if (!row) {
              return;
            }
            var choice = list.filter(function (x) {
              return x.id === row.dataset.value;
            })[0];
            selected = choice.time.clone().locale("en").format("YYYY-MM-DD HH:mmZ");
            var header = dates.querySelector(".select-kit-header-wrapper");
            header.innerHTML =
              '<div class="future-date-input-selector-icons">' +
              icon(choice.icon) +
              '</div><div class="select-kit-selected-name selected-name choice" data-name="' +
              escapeHtml(choice.name) +
              '" data-value="' +
              choice.id +
              '" title="' +
              escapeHtml(choice.name) +
              '"><span class="name">' +
              escapeHtml(choice.name) +
              '</span></div><span class="future-date-input-selector-datetime">' +
              escapeHtml(choice.formatted) +
              "</span>" +
              icon("angle-down", "angle-icon");
            dates.classList.add("has-selection");
            dates.querySelectorAll(".select-kit-row").forEach(function (r) {
              var is = r === row;
              r.classList.toggle("is-selected", is);
              r.classList.toggle("is-highlighted", is);
              r.setAttribute("aria-checked", is ? "true" : "false");
            });
            dates.open = false;
          });
        },
        footer: function (footer, close) {
          footer.innerHTML =
            '<button class="btn btn-text btn-primary" type="button"><span class="d-button-label">' +
            escapeHtml(s.ignore_save) +
            '</span></button><div class="loading-container inline-spinner"></div>';
          footer.querySelector(".btn-primary").addEventListener("click", function () {
            var modal = footer.closest(".d-modal");
            var target = username;
            if (!target) {
              var chooser = modal.querySelector(".email-group-user-chooser");
              var chosen = items(chooser)[0];
              target = chosen ? chosen[0] : null;
            }
            if (!selected || !target) {
              var flash = modal.querySelector("#modal-alert");
              if (!flash) {
                flash = document.createElement("div");
                flash.id = "modal-alert";
                flash.setAttribute("role", "alert");
                flash.className = "alert alert-error";
                modal.querySelector(".d-modal__container").insertBefore(flash, modal.querySelector(".d-modal__body"));
              }
              flash.textContent = s.ignore_required;
              return;
            }
            var data = { notification_level: "ignore", expiring_at: selected };
            if (acting) {
              data.acting_user_id = acting.id;
            }
            notificationLevel(target, data)
              .then(function () {
                if (onUserIgnored) {
                  onUserIgnored(target);
                }
                close();
              })
              .catch(popupAjaxError);
          });
        },
      });
    });
  }

  Discourse.ignoreDurationModal = ignoreModal;
})();
