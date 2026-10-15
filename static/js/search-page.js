// The full-page search (/search): what FullPageSearchController does in
// the browser. Enter or the Search button searches (the term, the sort
// order's term, the type and the context checkbox in the URL, swapped in
// as a link's page is); the clear button empties the field; the types
// switch the search; the sort order's ComboBox opens its rows and searches
// in that order (search-advanced.js runs the advanced filters). Terms are
// highlighted in the results as HighlightSearch does (highlightHTML), and
// DLoadMore's sentinel loads the next page of results as it comes into
// view.
(function () {
  "use strict";

  var script = document.currentScript;
  var config = { minLength: 3, searchType: "topics_posts", tooShort: "", clearLabel: "" };
  // The controller's sortOrders' terms by id.
  var ORDER_TERMS = { 1: "order:latest", 2: "order:likes", 3: "order:views", 4: "order:latest_topic", 5: "order:read" };

  function readConfig() {
    if (script) {
      config.minLength = parseInt(script.dataset.minLength, 10) || 0;
      config.tooShort = script.dataset.tooShort || "";
      config.clearLabel = script.dataset.clearLabel || "";
    }
    var active = document.querySelector(".search-types__type.active");
    config.searchType = active ? active.dataset.searchType : "topics_posts";
  }
  readConfig();

  // Swapped-in pages run this again; the listeners stay from the first.
  if (!Discourse.once("search-page")) {
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

  // lib/search's MIN_LENGTH_BYPASS_PATTERN and isValidSearchTerm.
  var BYPASS = /^(l|r)$|order:|category:|categories:|tags?:|before:|after:|status:|user:|group:|badge:|in:|with:|#|@/i;
  function validTerm(term) {
    if (!term) {
      return false;
    }
    var trimmed = term.trim();
    return BYPASS.test(trimmed) || trimmed.length >= config.minLength;
  }

  // highlightHTML: the words as one phrase, case-insensitive, between
  // non-word characters, in every text node under the element.
  function highlight(element, term) {
    if (!term) {
      return;
    }
    var words = (term.match(/"[^"]+"|[^\s]+/g) || [])
      .filter(function (w) {
        return w !== "l";
      })
      .map(function (w) {
        return w.replace(/^"(.*)"$/, "$1").replace(/[-\/\\^$*+?.()|[\]{}]/g, "\\$&");
      })
      .filter(Boolean);
    if (!words.length) {
      return;
    }
    var pattern;
    try {
      pattern = new RegExp("(\\W|^)(" + words.join(" ") + ")(\\W|$)", "i");
    } catch (_) {
      return;
    }
    (function walk(node) {
      if (node.nodeType === Node.ELEMENT_NODE) {
        if (node.tagName === "SCRIPT" || node.tagName === "STYLE") {
          return 0;
        }
        if (node.tagName === "SPAN" && node.className === "search-highlight") {
          return 0;
        }
        for (var i = 0; i < node.childNodes.length; i++) {
          i += walk(node.childNodes[i]);
        }
        return 0;
      }
      if (node.nodeType !== Node.TEXT_NODE) {
        return 0;
      }
      var match = node.data.match(pattern);
      if (!match) {
        return 0;
      }
      var span = document.createElement("span");
      span.className = "search-highlight";
      span.innerText = match[2];
      var matched = node.splitText(match.index + match[1].length);
      matched.splitText(match[2].length);
      matched.parentNode.replaceChild(span, matched);
      return 1;
    })(element);
  }

  function highlightEntries(entries) {
    var list = document.querySelector(".fps-result-entries");
    var term = list ? list.dataset.highlight : "";
    entries.forEach(function (entry) {
      entry.querySelectorAll(".topic-title > span, .blurb > span:not(.date)").forEach(function (span) {
        highlight(span, term);
      });
    });
  }

  // A route change: the page for the URL, swapped in as a link's is
  // (hx-boost).
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

  function sortTerm() {
    var row = document.querySelector("#search-sort-by .select-kit-row.is-selected");
    return row ? ORDER_TERMS[row.dataset.value] || "" : "";
  }

  // FullPageSearchController#_search with the page's state.
  function search(options) {
    options = options || {};
    var input = document.querySelector("input.search-query");
    if (!input) {
      return;
    }
    var term = input.value;
    var order = options.order != null ? options.order : sortTerm();
    var notice = document.querySelector(".search-notice");
    if (!order && !validTerm(term)) {
      if (notice) {
        var invalid = document.createElement("div");
        invalid.className = "fps-invalid";
        invalid.textContent = config.tooShort;
        notice.replaceChildren(invalid);
      }
      return;
    }
    var params = new URLSearchParams(location.search);
    var q = term + (order ? " " + order : "");
    params.set("q", q);
    params.delete("page");
    params.delete("expanded");
    var type = options.type || config.searchType;
    if (type && type !== "topics_posts") {
      params.set("search_type", type);
    } else {
      params.delete("search_type");
    }
    var context = document.querySelector('.search-context input[name="searchContext"]');
    if (context) {
      params.set("skip_context", context.checked ? "false" : "true");
    }
    go(location.pathname + "?" + params.toString());
  }

  Discourse.onPage(function () {
    readConfig();
    if (!document.querySelector(".search-container")) {
      return;
    }
    highlightEntries(Array.prototype.slice.call(document.querySelectorAll(".fps-result")));
    var input = document.querySelector("input.search-query");
    if (input && !input.value) {
      input.focus();
    }
    var sentinel = document.querySelector(".search-container .load-more-sentinel[data-next-page]");
    if (!sentinel || !window.IntersectionObserver) {
      return;
    }
    var observer = new IntersectionObserver(function (seen) {
      if (seen.some(function (e) { return e.isIntersecting; })) {
        loadMore(sentinel);
      }
    });
    observer.observe(sentinel);
    return function () {
      observer.disconnect();
    };
  });

  var loading = false;
  // FullPageSearchController#loadMore: the next page's results after
  // these, and its footer.
  function loadMore(sentinel) {
    var next = sentinel.dataset.nextPage;
    if (loading || !next) {
      return;
    }
    loading = true;
    var params = new URLSearchParams(location.search);
    params.set("page", next);
    fetch(location.pathname + "?" + params.toString(), { credentials: "same-origin" })
      .then(function (response) {
        return response.ok ? response.text() : Promise.reject(response.status);
      })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, "text/html");
        var list = document.querySelector(".fps-result-entries");
        var added = Array.prototype.slice.call(doc.querySelectorAll(".fps-result"));
        added.forEach(function (entry) {
          list.appendChild(document.importNode(entry, true));
        });
        highlightEntries(Array.prototype.slice.call(list.querySelectorAll(".fps-result")).slice(-added.length));
        var footer = doc.querySelector(".search-results .loading-container");
        var current = document.querySelector(".search-results .loading-container");
        if (footer && current) {
          current.innerHTML = footer.innerHTML;
        }
        var nextSentinel = doc.querySelector(".load-more-sentinel");
        if (nextSentinel && nextSentinel.dataset.nextPage) {
          sentinel.dataset.nextPage = nextSentinel.dataset.nextPage;
        } else {
          delete sentinel.dataset.nextPage;
        }
      })
      .catch(function () {})
      .then(function () {
        loading = false;
      });
  }

  document.addEventListener("keydown", function (event) {
    if (event.key === "Enter" && event.target.matches && event.target.matches("input.search-query")) {
      event.preventDefault();
      search();
    }
  });

  document.addEventListener("input", function (event) {
    var input = event.target;
    if (!input.matches || !input.matches("input.search-query")) {
      return;
    }
    // The clear button while there is a term.
    var field = input.closest(".search-bar__field");
    var clear = field.querySelector(".clear-search");
    if (input.value && !clear) {
      var label = config.clearLabel.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
      field.insertAdjacentHTML(
        "beforeend",
        '<a aria-label="' + label + '" class="clear-search" href="" title="' + label + '">' + icon("xmark") + "</a>"
      );
    } else if (!input.value && clear) {
      clear.remove();
    }
  });

  // The sort order's ComboBox: its body floats under it as select-kit's
  // does (bottom-start, fixed, at least the header's width).
  document.addEventListener(
    "toggle",
    function (event) {
      var chooser = event.target;
      if (!chooser.matches || !chooser.matches("#search-sort-by")) {
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
    if (!document.querySelector(".search-container")) {
      return;
    }
    var target = event.target;
    var row = target.closest("#search-sort-by .select-kit-row");
    if (row) {
      document.getElementById("search-sort-by").open = false;
      // setSortOrder searches without collapsing the advanced filters.
      if (Discourse.searchAdvancedKeepOpen) {
        Discourse.searchAdvancedKeepOpen();
      }
      search({ order: ORDER_TERMS[row.dataset.value] || "" });
      return;
    }
    document.querySelectorAll("#search-sort-by[open]").forEach(function (chooser) {
      if (!chooser.contains(target)) {
        chooser.open = false;
      }
    });
    if (target.closest(".search-cta")) {
      search();
      return;
    }
    var clear = target.closest(".search-bar .clear-search");
    if (clear) {
      event.preventDefault();
      var input = document.querySelector("input.search-query");
      input.value = "";
      clear.remove();
      input.focus();
      return;
    }
    // setSearchType: a term already there searches in the new type;
    // without one the caret goes to the field.
    var type = target.closest(".search-types__type");
    if (type) {
      var term = document.querySelector("input.search-query").value;
      if (validTerm(term)) {
        search({ type: type.dataset.searchType });
        return;
      }
      document.querySelectorAll(".search-types__type").forEach(function (button) {
        button.classList.toggle("active", button === type);
      });
      config.searchType = type.dataset.searchType;
      document.querySelector("input.search-query").focus();
      return;
    }
  });
})();
