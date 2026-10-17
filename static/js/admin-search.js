// The admin search (admin/components/admin-search.gjs with the
// AdminSearchDataSource service): the items the server formats
// (/admin/search?fragment, which lists them as buildMap does, once per
// page) scored as the service's `search` scores them, drawn as the
// component draws its results. The page /admin/search holds one with its
// `filter`; the sidebar's search box and `mod+/` open one in the modal
// (modal/admin-search.gjs). Enter in the input goes to the page with the
// filter, Up and Down move through the results.
(function () {
  "use strict";

  if (!Discourse.once("admin-search")) {
    return;
  }

  var MIN_FILTER_LENGTH = 2;
  var MAX_TYPE_RESULT_COUNT_LOW = 15;
  var MAX_TYPE_RESULT_COUNT_HIGH = 50;
  var SEARCH_SCORES = {
    labelStart: 20,
    labelPartial: 15,
    exactKeyword: 10,
    partialKeyword: 7,
    fallback: 5,
    pageBonusScore: 20,
  };
  // ADMIN_SEARCH_RESULT_TYPES, as the items' types.
  var TYPES = ["page", "setting", "theme", "component", "report", "upcoming_change"];
  // INPUT_DELAY
  var INPUT_DELAY = 250;

  var baseMeta = document.querySelector("meta[name='discourse-base-uri']");
  var base = baseMeta ? baseMeta.content : "";
  var strings = {};
  try {
    strings = JSON.parse(document.getElementById("admin-search-strings").textContent);
  } catch (e) {
    strings = {};
  }

  function t(key, values) {
    var text = strings[key] === undefined ? "[en." + key + "]" : strings[key];
    Object.keys(values || {}).forEach(function (name) {
      text = text.split("%{" + name + "}").join(String(values[name]));
    });
    return text;
  }

  function escapeHtml(text) {
    return String(text)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#x27;")
      .replace(/`/g, "&#x60;")
      .replace(/=/g, "&#x3D;");
  }

  function escapeRegExp(text) {
    return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  }

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' +
      escapeHtml(name) +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      escapeHtml(name) +
      '"></use></svg>'
    );
  }

  // buildMap, once: the items by type.
  var map = null;
  function buildMap() {
    if (!map) {
      map = fetch(base + "/admin/search?fragment", {
        headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
        credentials: "same-origin",
      })
        .then(function (response) {
          if (!response.ok) {
            throw new Error(response.status);
          }
          return response.json();
        })
        .then(function (json) {
          var byType = {};
          TYPES.forEach(function (type) {
            byType[type] = [];
          });
          json.items.forEach(function (item) {
            (byType[item.type] = byType[item.type] || []).push(item);
          });
          return byType;
        });
    }
    return map;
  }

  // AdminSearchDataSource#search
  function search(byType, filter) {
    if (filter.length < MIN_FILTER_LENGTH) {
      return [];
    }
    var results = [];
    var escaped = escapeRegExp(filter.toLowerCase().trim().replace(/\s+/g, " "));
    var perTypeLimit =
      filter.length < MIN_FILTER_LENGTH + 1 ? MAX_TYPE_RESULT_COUNT_LOW : MAX_TYPE_RESULT_COUNT_HIGH;
    var labelStart = new RegExp("^" + escaped, "i");
    var labelPartial = new RegExp("\\b" + escaped, "i");
    var words = escaped.split(" ").filter(function (keyword) {
      return keyword.length >= 3;
    });
    var exact = words.map(function (keyword) {
      return new RegExp("(" + keyword + ")\\b", "i");
    });
    var partial = words.map(function (keyword) {
      return new RegExp("\\b" + keyword, "i");
    });
    var fallback = new RegExp(escaped, "i");
    TYPES.forEach(function (type) {
      var typeResults = [];
      (byType[type] || []).forEach(function (item) {
        var score = 0;
        if (labelStart.test(item.label)) {
          score += SEARCH_SCORES.labelStart;
        } else if (labelPartial.test(item.label)) {
          score += SEARCH_SCORES.labelPartial;
        }
        var matches = function (regex) {
          return regex.test(item.keywords);
        };
        if (exact.length > 0 && exact.every(matches)) {
          score += SEARCH_SCORES.exactKeyword;
        } else if (partial.length > 0 && partial.every(matches)) {
          score += SEARCH_SCORES.partialKeyword;
        }
        if (filter.length >= 3 && fallback.test(item.keywords)) {
          score += SEARCH_SCORES.fallback;
        }
        if (score > 0) {
          if (type === "page") {
            score += SEARCH_SCORES.pageBonusScore;
          }
          typeResults.push({ item: item, score: score });
        }
      });
      typeResults.sort(function (a, b) {
        return b.score - a.score;
      });
      results = results.concat(typeResults.slice(0, perTypeLimit));
    });
    results.sort(function (a, b) {
      return b.score - a.score;
    });
    return results.map(function (r) {
      return r.item;
    });
  }

  // AdminSearch over its markup in `root`: `onEnter(filter)` for Enter in
  // the input.
  function attach(root, initialFilter, onEnter) {
    var container = root.querySelector(".admin-search__input-container");
    var input = root.querySelector(".admin-search__input-field");
    var status = root.querySelector('[role="status"].sr-only');
    var results = root.querySelector(".admin-search__results");
    var filter = initialFilter || "";
    var searchResults = [];
    var loading = false;
    var dataReady = false;
    var byType = null;
    var timer = null;

    input.value = filter;

    function spinner() {
      return filter !== "" && (loading || !dataReady);
    }

    function draw() {
      var has = searchResults.length > 0;
      container.classList.toggle("--has-results", has);
      results.classList.toggle("--has-results", has);
      var noResults = filter !== "" && !has && !spinner();
      var noResultsText = t("admin.search.no_results", { filter: escapeHtml(filter) });
      status.textContent = "";
      if (has) {
        status.textContent = t(
          searchResults.length === 1 ? "admin.search.result_count.one" : "admin.search.result_count.other",
          { count: searchResults.length, filter: filter }
        );
      }
      if (noResults) {
        status.textContent += noResultsText;
      }
      var paragraph = root.querySelector(".admin-search__no-results");
      if (noResults) {
        if (!paragraph) {
          paragraph = document.createElement("p");
          paragraph.setAttribute("aria-live", "polite");
          paragraph.className = "admin-search__no-results";
          paragraph.setAttribute("role", "status");
          results.parentNode.insertBefore(paragraph, results);
        }
        // The template's whitespace around the text.
        paragraph.textContent = "\n    " + noResultsText + "\n  ";
      } else if (paragraph) {
        paragraph.remove();
      }

      var holder = document.createElement("div");
      holder.className = "loading-container";
      if (spinner()) {
        holder.classList.add("visible");
        holder.setAttribute("data-loading", "true");
        holder.innerHTML = '<div class="spinner "></div>';
      } else {
        searchResults.forEach(function (result) {
          var row = document.createElement("div");
          row.className = "admin-search__result";
          row.setAttribute("data-result-type", result.type);
          var link = document.createElement("a");
          link.className = "admin-search__result-link";
          link.setAttribute("href", result.url);
          link.setAttribute("tabindex", "0");
          var name = document.createElement("div");
          name.className = "admin-search__result-name";
          name.innerHTML =
            (result.icon ? icon(result.icon) : "") +
            '<span class="admin-search__result-name-label"></span>';
          name.querySelector(".admin-search__result-name-label").textContent = result.label;
          link.appendChild(name);
          if (result.description) {
            // trustHTML: the description's markup, its setting links too.
            var description = document.createElement("div");
            description.className = "admin-search__result-description";
            description.innerHTML = result.description;
            link.appendChild(description);
          }
          row.appendChild(link);
          holder.appendChild(row);
        });
      }
      results.replaceChildren(holder);
    }

    function run() {
      loading = true;
      clearTimeout(timer);
      timer = setTimeout(function () {
        if (byType) {
          searchResults = search(byType, filter);
          loading = false;
        }
        draw();
      }, INPUT_DELAY);
    }

    input.addEventListener("input", function () {
      searchResults = [];
      filter = input.value;
      if (filter.length > 0) {
        run();
      }
      draw();
    });

    input.addEventListener("keydown", function (event) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        event.stopPropagation();
        var first = root.querySelector(".admin-search__result .admin-search__result-link");
        if (first) {
          first.focus();
        }
      }
      if (event.key === "Enter") {
        event.preventDefault();
        event.stopPropagation();
        onEnter(filter);
      }
    });

    results.addEventListener("keydown", function (event) {
      var link = event.target.closest(".admin-search__result-link");
      if (!link) {
        return;
      }
      if (event.key === "Enter") {
        event.preventDefault();
        event.stopPropagation();
        link.click();
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        event.stopPropagation();
        var previous = link.parentElement.previousElementSibling;
        if (previous && previous.firstElementChild) {
          previous.firstElementChild.focus();
        } else {
          input.focus();
        }
      }
      if (event.key === "ArrowDown") {
        event.preventDefault();
        event.stopPropagation();
        var next = link.parentElement.nextElementSibling;
        if (next && next.firstElementChild) {
          next.firstElementChild.focus();
        }
      }
    });

    buildMap().then(
      function (items) {
        byType = items;
        dataReady = true;
        if (filter !== "") {
          loading = true;
          run();
        }
      },
      function () {
        map = null;
      }
    );

    return {
      // didUpdate of @initialFilter: the page's filter changed.
      setFilter: function (value) {
        filter = value;
        input.value = value;
        run();
      },
    };
  }

  function goToPage(filter) {
    window.location.href = base + "/admin/search?filter=" + encodeURIComponent(filter);
  }

  // The search page: its filter from the query, Enter changing it
  // (refreshModel) without leaving the page.
  var page = document.querySelector(".admin-search .admin-config-area__full-width");
  if (page) {
    var params = new URLSearchParams(window.location.search);
    var instance = attach(page, params.get("filter") || "", function (filter) {
      var url = new URL(window.location.href);
      url.searchParams.set("filter", filter);
      window.history.pushState({}, "", url);
      instance.setFilter(filter);
    });
    window.addEventListener("popstate", function () {
      instance.setFilter(new URLSearchParams(window.location.search).get("filter") || "");
    });
    page.querySelector(".admin-search__input-field").focus();

    // The shortcut's sentence where there is a keyboard to press it
    // (capabilities.hasKeyboard): a fine pointer, or a keydown only a
    // physical keyboard sends; both tracked, so it follows them.
    var description = document.querySelector(".admin-search .d-page-header__description");
    if (description) {
      var plain = description.innerHTML;
      var finePointer = window.matchMedia("(pointer: fine)");
      var keyboardSeen = false;
      var drawDescription = function () {
        if (!finePointer.matches && !keyboardSeen) {
          description.innerHTML = plain;
          return;
        }
        var apple = /Mac|iPhone|iPad|iPod/.test(navigator.platform || navigator.userAgent);
        var shortcutHTML = "<kbd>" + (apple ? "⌘" : "Ctrl") + "</kbd> <kbd>/</kbd>";
        description.innerHTML =
          plain + " " + t("admin.config.search_everything.header_shortcut", { shortcutHTML: shortcutHTML });
      };
      finePointer.addEventListener("change", drawDescription);
      document.addEventListener(
        "keydown",
        function (event) {
          if (!keyboardSeen && ["Meta", "Control", "Alt", "Tab"].indexOf(event.key) !== -1) {
            keyboardSeen = true;
            drawDescription();
          }
        },
        true
      );
      drawDescription();
    }
  }

  // The modal: AdminSearch in a DModal without a header, closed by a
  // route change (a result's link leaves the page).
  var template = document.getElementById("admin-search-modal-template");
  var open = false;
  function showModal(event) {
    if (event) {
      event.preventDefault();
      event.stopPropagation();
    }
    if (!template || open || !Discourse.modal) {
      return;
    }
    open = true;
    Discourse.modal.show({
      className: "admin-search-modal --quick-palette",
      hideHeader: true,
      body: function (body) {
        body.appendChild(template.content.cloneNode(true));
        attach(body, "", goToPage);
      },
      onClose: function () {
        open = false;
      },
    });
  }

  document.addEventListener("mousedown", function (event) {
    if (event.target.closest(".sidebar-search__input")) {
      showModal(event);
    }
  });
  document.addEventListener("click", function (event) {
    if (event.target.closest(".sidebar-search__icon")) {
      showModal(event);
    }
  });
  document.addEventListener("keydown", function (event) {
    var mod = /Mac|iPhone|iPad|iPod/.test(navigator.platform || navigator.userAgent)
      ? event.metaKey
      : event.ctrlKey;
    if (mod && event.key === "/" && !event.altKey && !event.shiftKey) {
      showModal(event);
    }
  });
})();
