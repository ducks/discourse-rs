// Chat search in the browser (ChatSearch), in the drawer's search route
// (plugins/chat/search_view.rs draws it empty):
//
//   - the filter: what is typed becomes the query (chatSearchQuery, kept
//     across the drawer's routes) after INPUT_DELAY, and a new query is
//     searched from the first result
//   - the sort menu (DMenu): relevance or latest, searched again
//   - the results GET /live/chat/search draws, LIMIT at a time, the next
//     page as the list's end shows (DLoadMore); none found, the alert
//   - a result opens its message (DiscourseURL.routeTo, which the drawer
//     takes), on a click, Enter or Space
(function () {
  "use strict";

  if (!Discourse.once("chat-search")) {
    return;
  }

  // discourse/lib/environment's INPUT_DELAY.
  var INPUT_DELAY = 250;

  // chatSearchQuery#query, which outlives the route.
  var query = null;
  // The search shown: its root, the component's tracked state and the
  // nodes drawn after the filters.
  var current = null;
  var filterTimer = null;

  function basePath() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function withoutPrefix(url) {
    var base = basePath();
    if (base && url.indexOf(base) === 0) {
      url = url.slice(base.length);
    }
    return url || "/";
  }

  function isBlank(value) {
    return !value || !String(value).trim();
  }

  // The nodes after the filters, drawn for the state: the list while
  // there are results (the spinner while loading, the sentinel while
  // there are more), else the alert once loaded.
  function draw() {
    var s = current;
    s.nodes.forEach(function (node) {
      node.remove();
    });
    s.nodes = [];
    s.list = null;
    s.spinner = null;
    s.sentinel = null;
    if (!query || !query.length) {
      return;
    }
    var name = s.count ? "results" : s.loading ? null : "empty";
    if (!name) {
      return;
    }
    var fragment = s.root.querySelector("template.chat-search__" + name + "-template").content.cloneNode(true);
    s.nodes = Array.prototype.slice.call(fragment.childNodes);
    if (s.count) {
      s.list = fragment.querySelector(".chat-message-search-entries");
      s.list.innerHTML = s.items;
      s.spinner = fragment.querySelector(".chat-search-loading .loading-container");
    }
    s.root.querySelector(".chat-search__filters").after(fragment);
    drawProgress();
  }

  // DConditionalLoadingSpinner and DLoadMore.
  function drawProgress() {
    var s = current;
    if (s.spinner) {
      s.spinner.classList.toggle("visible", s.loading);
      s.spinner.innerHTML = s.loading ? '<div class="spinner"></div>' : "";
    }
    if (s.sentinel) {
      s.observer.unobserve(s.sentinel);
      s.sentinel.remove();
      s.sentinel = null;
    }
    if (s.list && s.hasMore) {
      s.sentinel = document.createElement("div");
      s.sentinel.className = "load-more";
      s.list.parentNode.insertBefore(s.sentinel, s.list.parentNode.querySelector(":scope > br"));
      s.nodes.push(s.sentinel);
      s.observer.observe(s.sentinel);
    }
  }

  // searchMessages: a new search from the first result (`reset`), or
  // the next page.
  function searchMessages(reset) {
    var s = current;
    if (s.controller) {
      s.controller.abort();
      s.controller = null;
    }
    if (isBlank(query)) {
      s.items = "";
      s.count = 0;
      s.offset = 0;
      s.hasMore = false;
      s.loading = false;
      draw();
      return;
    }
    s.loading = true;
    if (reset) {
      s.offset = 0;
      s.items = "";
      s.count = 0;
      draw();
    } else {
      drawProgress();
    }
    var params = new URLSearchParams({ q: query, sort: s.sort, offset: String(s.offset) });
    var controller = window.AbortController ? new AbortController() : null;
    s.controller = controller;
    fetch(s.root.dataset.url + "?" + params.toString(), {
      credentials: "same-origin",
      signal: controller ? controller.signal : undefined,
    })
      .then(function (r) {
        if (!r.ok) {
          throw new Error(r.statusText);
        }
        return r.text().then(function (html) {
          return { html: html, more: r.headers.get("x-chat-search-has-more") === "true" };
        });
      })
      .then(function (page) {
        if (current !== s || s.controller !== controller) {
          return;
        }
        var template = document.createElement("template");
        template.innerHTML = page.html;
        var added = template.content.querySelectorAll(".chat-message-search-entry").length;
        var wasEmpty = !s.count;
        s.items += page.html;
        s.count += added;
        s.offset += added;
        s.hasMore = page.more;
        s.loading = false;
        s.controller = null;
        if (wasEmpty || !s.list) {
          draw();
        } else {
          s.list.insertAdjacentHTML("beforeend", page.html);
          drawProgress();
        }
      })
      .catch(function (error) {
        if (current !== s || s.controller !== controller || (error && error.name === "AbortError")) {
          return;
        }
        s.loading = false;
        s.controller = null;
        draw();
      });
  }

  // checkQueryChange
  function checkQueryChange() {
    if (current.lastQuery !== query) {
      current.lastQuery = query;
      searchMessages(true);
    }
  }

  function visitMessage(entry) {
    var url = withoutPrefix(entry.dataset.url);
    if (Discourse.chatDrawer && Discourse.chatDrawer.transitionTo) {
      Discourse.chatDrawer.transitionTo(url);
    } else {
      window.location.assign(basePath() + url);
    }
  }

  // The sort menu (DMenu): its items floated under the trigger.
  var menu = null;

  function closeMenu() {
    if (!menu) {
      return;
    }
    menu.trigger.setAttribute("aria-expanded", "false");
    menu.trigger.classList.remove("-expanded");
    menu.content.remove();
    menu = null;
  }

  function openMenu(trigger) {
    closeMenu();
    var portals = document.getElementById("d-menu-portals");
    if (!portals) {
      portals = document.createElement("div");
      portals.id = "d-menu-portals";
      document.body.appendChild(portals);
    }
    var content = document.createElement("div");
    content.className = "fk-d-menu search-sort-options-content -animated -expanded";
    content.setAttribute("role", "dialog");
    content.dataset.identifier = "search-sort-options";
    content.dataset.content = "";
    content.dataset.strategy = "absolute";
    content.dataset.placement = "bottom";
    var inner = document.createElement("div");
    inner.className = "fk-d-menu__inner-content";
    inner.appendChild(current.root.querySelector("template.search-sort-options-template").content.cloneNode(true));
    content.appendChild(inner);
    portals.appendChild(content);
    var rect = trigger.getBoundingClientRect();
    content.style.position = "absolute";
    content.style.left = rect.left + window.scrollX + "px";
    content.style.top = rect.bottom + window.scrollY + 4 + "px";
    trigger.setAttribute("aria-expanded", "true");
    trigger.classList.add("-expanded");
    menu = { trigger: trigger, content: content };
  }

  document.addEventListener("input", function (event) {
    if (!current || !event.target.matches(".c-search .filter-input")) {
      return;
    }
    var value = event.target.value;
    clearTimeout(filterTimer);
    filterTimer = setTimeout(function () {
      // debounceFilterChange
      query = value === "" ? null : value;
      if (current) {
        checkQueryChange();
      }
    }, INPUT_DELAY);
  });

  document.addEventListener("click", function (event) {
    var s = current;
    if (!s || !s.root.isConnected) {
      return;
    }
    var trigger = event.target.closest(".c-search .search-sort-options-trigger");
    if (trigger) {
      if (menu && menu.trigger === trigger) {
        closeMenu();
      } else {
        openMenu(trigger);
      }
      return;
    }
    // setCurrentSorting
    var option = event.target.closest(".search-sort-options-content [data-sort]");
    if (option && menu) {
      s.sort = option.dataset.sort;
      menu.trigger.dataset.sort = s.sort;
      menu.trigger.querySelector(".d-button-label").textContent = option.textContent;
      closeMenu();
      searchMessages(true);
      return;
    }
    if (menu && !event.target.closest(".search-sort-options-content")) {
      closeMenu();
    }
    var entry = event.target.closest(".c-search .chat-message-search-entry[data-url]");
    if (entry) {
      event.preventDefault();
      visitMessage(entry);
    }
  });

  // handleKeypress
  document.addEventListener("keydown", function (event) {
    var entry = event.target.closest && event.target.closest(".c-search .chat-message-search-entry[data-url]");
    if (entry && (event.key === "Enter" || event.key === " ")) {
      event.preventDefault();
      visitMessage(entry);
    }
  });

  function mount() {
    unmount();
    var root = document.querySelector(".c-search[data-url]");
    if (!root) {
      return;
    }
    current = {
      root: root,
      sort: "relevance",
      items: "",
      count: 0,
      offset: 0,
      hasMore: false,
      loading: false,
      lastQuery: null,
      controller: null,
      nodes: [],
      list: null,
      spinner: null,
      sentinel: null,
      observer: null,
    };
    var s = current;
    s.observer = window.IntersectionObserver
      ? new IntersectionObserver(function (entries) {
          // loadMore
          if (entries.some(function (e) { return e.isIntersecting; }) && s.hasMore && !s.loading && current === s) {
            searchMessages(false);
          }
        })
      : { observe: function () {}, unobserve: function () {}, disconnect: function () {} };
    // @value={{@query}} and dAutoFocus.
    var input = root.querySelector(".filter-input");
    input.value = query || "";
    input.focus({ preventScroll: true });
    checkQueryChange();
  }

  function unmount() {
    closeMenu();
    if (!current) {
      return;
    }
    if (current.controller) {
      current.controller.abort();
    }
    current.observer.disconnect();
    current = null;
  }

  // For chat-drawer.js: its search route drawn, or gone.
  Discourse.chatSearch = { mount: mount, unmount: unmount };
})();
