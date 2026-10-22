// The users directory (/u): what the Ember users route does in the
// browser. The period chooser opens its rows and goes to the period; a
// column header sorts by it (DTableHeaderToggle); the name filter loads
// the matching users after 500ms (UsersController#onUsernameFilterChanged)
// with the name in the URL; and DLoadMore's sentinel loads the next page
// of /directory_items.json as it scrolls into view (ResultSet#loadMore).
// Rows it loads are drawn as directory_view draws them, from the page's
// #users-directory-config. A refused request shows in the page's error
// line, as htmx's refusals do.
(function () {
  "use strict";

  // Swapped-in pages run this again; the listeners stay from the first.
  if (!Discourse.once("users-directory")) {
    return;
  }

  var config = null;
  var loading = false;

  function escape(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#x27;")
      .replace(/`/g, "&#x60;")
      .replace(/=/g, "&#x3D;");
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

  function plural(forms, count) {
    return (count === 1 ? forms.one : forms.other).replace(/%\{count\}/g, count);
  }

  // lib/formatter's number: the short form.
  function number(value) {
    var n = Math.round(parseFloat(value));
    if (isNaN(n)) {
      n = 0;
    }
    if (n > 999999) {
      return config.millions.replace("%{number}", (n / 1000000).toFixed(1));
    }
    if (n > 99999) {
      return config.thousands.replace("%{number}", String(Math.floor(n / 1000)));
    }
    if (n > 999) {
      return config.thousands.replace("%{number}", (n / 1000).toFixed(1));
    }
    return String(n);
  }

  // durationTiny
  function duration(seconds) {
    if (typeof seconds !== "number") {
      return "&mdash;";
    }
    var t = function (key, count) {
      return escape(plural(config.tiny[key], count));
    };
    var minutes = Math.max(Math.round(seconds / 60), 1);
    if (seconds <= 59) {
      return t("less_than_x_minutes", 1);
    }
    if (minutes <= 44) {
      return t("x_minutes", minutes);
    }
    if (minutes <= 89) {
      return t("about_x_hours", 1);
    }
    if (minutes <= 1409) {
      return t("about_x_hours", Math.round(minutes / 60));
    }
    if (minutes <= 2519) {
      return t("x_days", 1);
    }
    if (minutes <= 129599) {
      return t("x_days", Math.round(minutes / 1440));
    }
    if (minutes <= 525599) {
      return t("x_months", Math.round(minutes / 43200));
    }
    var years = minutes / 525600;
    var remainder = years % 1;
    if (remainder < 0.25) {
      return t("about_x_years", Math.floor(years));
    }
    if (remainder < 0.75) {
      return t("over_x_years", Math.floor(years));
    }
    return t("almost_x_years", Math.floor(years) + 1);
  }

  // user_info_view::user_info with its heading level 3.
  function userInfo(user) {
    var username = escape(user.username);
    var path = config.basePath + "/u/" + escape(String(user.username).toLowerCase());
    var template = String(user.avatar_template || "");
    var src =
      (template.charAt(0) === "/" && template.charAt(1) !== "/" ? config.basePath : "") +
      escape(template.replace("{size}", "48"));
    var name = user.name ? String(user.name) : "";
    var nameFirst = config.prioritizeName && name.trim() !== "";
    var nameHtml = name
      ? ' <span class="name-wrapper"><a data-user-card="' +
        username +
        '" href="' +
        path +
        '"><span class="name">' +
        escape(name) +
        "</span></a></span>"
      : "";
    return (
      '<div class="user-info small" data-username="' +
      username +
      '"><div class="user-image"><div class="user-image-inner"><a aria-hidden="true" data-user-card="' +
      username +
      '" href="' +
      path +
      '"><img alt="" width="48" height="48" src="' +
      src +
      '" class="avatar" title="' +
      username +
      '"></a></div></div><div class="user-detail"><div class="name-line' +
      (nameFirst ? " --name-first" : "") +
      '"><span class="username-wrapper"><a aria-level="3" data-user-card="' +
      username +
      '" href="' +
      path +
      '" role="heading"><span class="username">' +
      username +
      "</span></a></span>" +
      nameHtml +
      '</div><div class="title">' +
      escape(user.title) +
      "</div></div></div>"
    );
  }

  // directory_view::user_field_value (DirectoryItemUserFieldValue)
  function userFieldValue(item, fieldId) {
    var fields = (item.user && item.user.user_fields) || {};
    var data = fields[fieldId];
    if (!data || !data.value || data.value.length === 0) {
      return "-";
    }
    var values = data.value
      .toString()
      .split(",")
      .map(function (v) {
        return v.replace(/-/g, " ").trim();
      });
    if (data.searchable) {
      return values
        .map(function (v) {
          return (
            '<a class="directory-value-list-item" href="' +
            config.basePath +
            "/u?name=" +
            encodeURIComponent(v) +
            '">' +
            escape(v) +
            "</a>"
          );
        })
        .join("");
    }
    return escape(values.join(","));
  }

  // directory_view::row
  function row(item) {
    var me = config.meId != null && item.user && item.user.id === config.meId;
    var html =
      '<div class="directory-table__row' +
      (me ? " me" : "") +
      '" role="row"><div class="directory-table__cell" role="rowheader">' +
      userInfo(item.user || {}) +
      "</div>";
    config.columns.forEach(function (column) {
      if (column.userFieldId != null) {
        html +=
          '<div class="directory-table__cell--user-field" role="cell"><span class="directory-table__label"><span>' +
          escape(column.name) +
          '</span></span> <span class="directory-table__value--user-field">' +
          userFieldValue(item, column.userFieldId) +
          "</span></div>";
        return;
      }
      var count = item[column.name] || 0;
      html +=
        '<div class="directory-table__cell" role="cell"><span class="directory-table__label"><span>' +
        (column.icon ? icon(column.icon) + " " : "") +
        escape(count === 1 ? column.label.one : column.label.other) +
        '</span></span> <span class="directory-table__value">' +
        number(item[column.name]) +
        "</span></div>";
    });
    if (config.showTimeRead) {
      html +=
        '<div class="directory-table__cell time-read" role="cell"><span class="directory-table__label"><span>' +
        escape(config.timeRead) +
        '</span></span> <span class="directory-table__value">' +
        duration(item.time_read) +
        "</span></div>";
    }
    return html + "</div>";
  }

  function showError(body) {
    var message = "Something went wrong.";
    try {
      var parsed = JSON.parse(body);
      if (parsed.errors) {
        message = parsed.errors.join(" ");
      }
    } catch (_) {}
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = message;
      error.hidden = false;
    }
  }

  function getJson(url) {
    return fetch(url, {
      credentials: "same-origin",
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
    }).then(function (response) {
      if (!response.ok) {
        return response.text().then(function (body) {
          showError(body);
          throw new Error("refused");
        });
      }
      return response.json();
    });
  }

  // The route's query params as the URL carries them.
  function pageUrl(changes) {
    var params = new URLSearchParams(location.search);
    params.set("order", currentOrder());
    Object.keys(changes).forEach(function (key) {
      if (changes[key] == null || changes[key] === "") {
        params.delete(key);
      } else {
        params.set(key, changes[key]);
      }
    });
    if (params.get("period") === "weekly") {
      params.delete("period");
    }
    var keys = Array.from(params.keys()).sort();
    var query = keys
      .map(function (key) {
        return encodeURIComponent(key) + "=" + encodeURIComponent(params.get(key));
      })
      .join("&");
    return location.pathname + (query ? "?" + query : "");
  }

  // A route change: the page for the URL, swapped in as a link's is
  // (hx-boost), so the page's live stream carries on.
  function go(url) {
    if (!window.htmx) {
      window.location.assign(url);
      return;
    }
    var link = document.createElement("a");
    link.href = url;
    link.hidden = true;
    document.getElementById("main-outlet").appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  function currentOrder() {
    var header = document.querySelector('.directory-table__column-header[aria-sort]:not([aria-sort="none"])');
    if (header) {
      return header.dataset.order;
    }
    return new URLSearchParams(location.search).get("order") || (config.columns[0] || {}).name || "likes_received";
  }

  function loadMore() {
    if (loading || !config || config.loaded >= config.total) {
      return;
    }
    var body = document.querySelector(".directory-table__body");
    if (!body) {
      return;
    }
    loading = true;
    getJson(config.loadMoreUrl)
      .then(function (result) {
        var items = result.directory_items || [];
        body.insertAdjacentHTML("beforeend", items.map(row).join(""));
        config.loaded += items.length;
        config.total = result.meta.total_rows_directory_items;
        config.loadMoreUrl = result.meta.load_more_directory_items;
        // Nothing new: nothing more to load.
        if (items.length === 0) {
          config.total = config.loaded;
        }
      })
      .catch(function () {})
      .then(function () {
        loading = false;
      });
  }

  // _setUsernameFilter: the users matching the name, the name in the URL.
  function filter(name) {
    var params = new URLSearchParams(location.search);
    var query = new URLSearchParams();
    query.set("period", params.get("period") || "weekly");
    query.set("order", currentOrder());
    ["asc", "group", "exclude_groups", "exclude_usernames"].forEach(function (key) {
      if (params.get(key)) {
        query.set(key, params.get(key));
      }
    });
    if (name) {
      query.set("name", name);
    }
    history.replaceState(history.state, "", pageUrl({ name: name }));
    getJson(config.basePath + "/directory_items?" + query.toString())
      .then(function (result) {
        var items = result.directory_items || [];
        config.loaded = items.length;
        config.total = result.meta.total_rows_directory_items;
        config.loadMoreUrl = result.meta.load_more_directory_items;
        var total = document.querySelector(".users-directory .total-rows");
        if (total) {
          total.textContent = config.total ? plural(config.totalRows, config.total) : "";
        }
        var body = document.querySelector(".directory-table__body");
        if (body && items.length) {
          body.innerHTML = items.map(row).join("");
        } else {
          // The table or the empty state: the page as the server draws it.
          location.reload();
        }
      })
      .catch(function () {});
  }

  Discourse.onPage(function () {
    var script = document.getElementById("users-directory-config");
    if (!script) {
      config = null;
      return;
    }
    config = JSON.parse(script.textContent);
    loading = false;
    var cleanups = [];

    var sentinel = document.querySelector(".users-directory")
      ? document.querySelector(".load-more-sentinel")
      : null;
    if (sentinel && "IntersectionObserver" in window) {
      var observer = new IntersectionObserver(function (entries) {
        if (entries[0].isIntersecting) {
          setTimeout(loadMore, 100);
        }
      });
      observer.observe(sentinel);
      cleanups.push(function () {
        observer.disconnect();
      });
    }

    var input = document.querySelector(".users-directory .filter-name");
    var timer = null;
    if (input) {
      var onInput = function () {
        clearTimeout(timer);
        timer = setTimeout(function () {
          filter(input.value);
        }, 500);
      };
      input.addEventListener("input", onInput);
      cleanups.push(function () {
        clearTimeout(timer);
        input.removeEventListener("input", onInput);
      });
    }

    // HorizontalScrollSyncWrapper: the top scrollbar follows the table's.
    var content = document.querySelector(".directory-table-container .horizontal-scroll-sync__content");
    var top = document.querySelector(".directory-table-container .horizontal-scroll-sync__top-scroll");
    var fake = document.querySelector(".directory-table-container .horizontal-scroll-sync__fake-content");
    if (content && top && fake) {
      var syncWidth = function () {
        var wide = content.scrollWidth > content.clientWidth;
        fake.style.width = content.scrollWidth + "px";
        fake.style.height = wide && content.getBoundingClientRect().bottom > window.innerHeight ? "12px" : "0";
      };
      var fromContent = function () {
        top.scrollLeft = content.scrollLeft;
      };
      var fromTop = function () {
        content.scrollLeft = top.scrollLeft;
      };
      content.addEventListener("scroll", fromContent, { passive: true });
      top.addEventListener("scroll", fromTop, { passive: true });
      window.addEventListener("resize", syncWidth);
      window.addEventListener("scroll", syncWidth, { passive: true });
      syncWidth();
      cleanups.push(function () {
        window.removeEventListener("resize", syncWidth);
        window.removeEventListener("scroll", syncWidth);
      });
    }

    return function () {
      cleanups.forEach(function (fn) {
        fn();
      });
    };
  });

  // The period chooser and the group filter: details elements whose body
  // floats under them as select-kit's does (bottom-start, fixed, at least
  // the header's width).
  document.addEventListener(
    "toggle",
    function (event) {
      var chooser = event.target;
      // A top list's chooser is top-period.js's.
      if (!chooser.classList || !chooser.matches(".period-chooser, .directory-group-selector") || chooser.closest(".top-lists")) {
        return;
      }
      var open = chooser.open;
      chooser.classList.toggle("is-expanded", open);
      var caret = chooser.querySelector(".angle-icon");
      if (caret) {
        caret.outerHTML = icon(open ? "angle-up" : "angle-down").replace(
          "svg-icon fa-width-auto",
          "svg-icon fa-width-auto angle-icon"
        );
      }
      var header = chooser.querySelector("summary");
      var body = chooser.querySelector(".select-kit-body");
      if (!open || !body || !window.FloatingUIDOM) {
        return;
      }
      // SelectKit#updateFloatingUiPosition with its verticalOffset of 3;
      // under 451px wide the body spans the screen less 10px a side.
      var ui = window.FloatingUIDOM;
      var small = window.innerWidth <= 450;
      var headerOffset = parseInt(document.documentElement.style.getPropertyValue("--header-offset"), 10) || 0;
      body.style.position = "fixed";
      body.style.top = "0";
      body.style.left = "0";
      body.style.minWidth = small ? "" : Math.max(Math.round(header.getBoundingClientRect().width), 220) + "px";
      body.style.width = small ? window.innerWidth - 20 + "px" : "";
      ui.computePosition(header, body, {
        placement: "bottom-start",
        strategy: "fixed",
        middleware: [
          ui.flip({ padding: { top: 3 + headerOffset, bottom: 3 }, fallbackStrategy: "initialPlacement" }),
          ui.shift({ limiter: ui.limitShift() }),
          ui.offset(3),
        ],
      }).then(function (result) {
        var x = small ? 10 : Math.round(result.x);
        body.style.transform = "translate(" + x + "px," + Math.round(result.y) + "px)";
      });
    },
    true
  );

  document.addEventListener("click", function (event) {
    if (!config) {
      return;
    }
    var periodRow = event.target.closest(".period-chooser .select-kit-row");
    if (periodRow) {
      go(pageUrl({ period: periodRow.dataset.value }));
      return;
    }
    // UsersController#groupChanged: the row's group, none for all groups.
    var groupRow = event.target.closest(".directory-group-selector .select-kit-row");
    if (groupRow) {
      go(pageUrl({ group: groupRow.dataset.value || null }));
      return;
    }
    // A click outside closes them.
    document.querySelectorAll(".period-chooser[open], .directory-group-selector[open]").forEach(function (chooser) {
      if (!chooser.contains(event.target)) {
        chooser.open = false;
      }
    });
    var header = event.target.closest(".directory-table__column-header.sortable");
    if (header) {
      var url = pageUrl({ asc: header.dataset.asc || null });
      var params = new URLSearchParams(url.split("?")[1] || "");
      params.set("order", header.dataset.order);
      var keys = Array.from(params.keys()).sort();
      go(
        location.pathname +
          "?" +
          keys
            .map(function (key) {
              return encodeURIComponent(key) + "=" + encodeURIComponent(params.get(key));
            })
            .join("&")
      );
    }
  });

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      document.querySelectorAll(".period-chooser[open], .directory-group-selector[open]").forEach(function (chooser) {
        chooser.open = false;
      });
    }
    if (event.key === "Enter" && event.target.closest) {
      var target = event.target.closest(".directory-table__column-header .header-contents, .period-chooser .select-kit-row, .directory-group-selector .select-kit-row");
      if (target) {
        target.click();
      }
    }
  });
})();
