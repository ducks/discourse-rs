// Chat search in the browser (ChatSearch), in the drawer's search route
// and the full page at /chat/search (ChatRoutesSearch, whose query is
// the `q` param, kept in the address as it changes; replace: true)
// (plugins/chat/search_view.rs draws it empty):
//
//   - the filter: what is typed becomes the query (chatSearchQuery, kept
//     across the drawer's routes) after INPUT_DELAY, and a new query is
//     searched from the first result; DFilterInput's clear button while
//     there is a query, and its container is-focused with the input
//   - the sort menu (DMenu): relevance or latest, searched again
//   - the results GET /live/chat/search draws, LIMIT at a time, the next
//     page as the list's end shows (DLoadMore); none found, the alert
//   - a result opens its message (DiscourseURL.routeTo, which the drawer
//     takes), on a click, Enter or Space; Tab moves between the results
//     (dTabToSibling)
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
    if (s.sentinel) {
      s.observer.unobserve(s.sentinel);
      s.sentinel = null;
    }
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

  // DConditionalLoadingSpinner (no @size: "spinner " as the template
  // concatenates it, data-loading present only while loading) and
  // DLoadMore without a block: its sentinel, kept while there are more
  // results (its observer reports a change of intersection, not each
  // page), gone with the list.
  function drawProgress() {
    var s = current;
    if (s.spinner) {
      s.spinner.classList.toggle("visible", s.loading);
      if (s.loading) {
        s.spinner.setAttribute("data-loading", "");
      } else {
        s.spinner.removeAttribute("data-loading");
      }
      s.spinner.innerHTML = s.loading ? '<div class="spinner "></div>' : "";
    }
    if (s.sentinel && !(s.list && s.hasMore)) {
      s.observer.unobserve(s.sentinel);
      s.sentinel.remove();
      s.sentinel = null;
    }
    if (s.list && s.hasMore && !s.sentinel) {
      s.sentinel = document.createElement("div");
      s.sentinel.setAttribute("aria-hidden", "true");
      s.sentinel.className = "load-more-sentinel";
      s.list.parentNode.insertBefore(s.sentinel, s.list.parentNode.querySelector(":scope > br"));
      s.nodes.push(s.sentinel);
      s.observer.observe(s.sentinel);
    }
  }

  // DFilterInput's clear button, while there is a query (its @value).
  function drawClear() {
    var s = current;
    var container = s.root.querySelector(".chat-search__filters .filter-input-container");
    var button = container.querySelector(".filter-input-clear-btn");
    if (query && query.length) {
      if (!button) {
        container.appendChild(s.root.querySelector("template.filter-input-clear-template").content.cloneNode(true));
      }
    } else if (button) {
      button.remove();
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
    drawClear();
    if (current.lastQuery !== query) {
      current.lastQuery = query;
      searchMessages(true);
    }
  }

  // The full page's search, which keeps the query in the address.
  function fullPage() {
    return current && current.root.dataset.queryParams === "true";
  }

  function visitMessage(entry) {
    var url = withoutPrefix(entry.dataset.url);
    if (fullPage()) {
      // DiscourseURL.routeTo in the app.
      var link = document.createElement("a");
      link.href = basePath() + url;
      link.hidden = true;
      document.body.appendChild(link);
      if (window.htmx) {
        window.htmx.process(link);
      }
      link.click();
    } else if (Discourse.chatDrawer && Discourse.chatDrawer.transitionTo) {
      Discourse.chatDrawer.transitionTo(url);
    } else {
      window.location.assign(basePath() + url);
    }
  }

  // router.replaceWith({ queryParams: { q } }): no q for a null query (an
  // emptied input), an empty one for the clear button's "" (Ember prunes
  // only the default, undefined).
  function replaceQueryParam() {
    var params = new URLSearchParams(location.search);
    if (query !== null) {
      params.set("q", query);
    } else {
      params.delete("q");
    }
    var search = params.toString();
    history.replaceState(history.state, "", location.pathname + (search ? "?" + search : "") + location.hash);
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

  // debounceFilterChange
  function setQuery(value) {
    query = value;
    if (current) {
      if (fullPage()) {
        replaceQueryParam();
      }
      checkQueryChange();
    }
  }

  document.addEventListener("input", function (event) {
    if (!current || !event.target.matches(".c-search .filter-input")) {
      return;
    }
    var value = event.target.value;
    clearTimeout(filterTimer);
    filterTimer = setTimeout(function () {
      setQuery(value === "" ? null : value);
    }, INPUT_DELAY);
  });

  // DFilterInput's focusState: the container is-focused while the input
  // has the focus.
  document.addEventListener("focusin", function (event) {
    if (event.target.matches && event.target.matches(".c-search .filter-input")) {
      event.target.closest(".filter-input-container").classList.add("is-focused");
    }
  });
  document.addEventListener("focusout", function (event) {
    if (event.target.matches && event.target.matches(".c-search .filter-input")) {
      event.target.closest(".filter-input-container").classList.remove("is-focused");
    }
  });

  document.addEventListener("click", function (event) {
    var s = current;
    if (!s || !s.root.isConnected) {
      return;
    }
    // DFilterInput#onClearInput: debounceFilterChange("") at once, then
    // the input's focus.
    var clear = event.target.closest(".c-search .filter-input-clear-btn");
    if (clear) {
      var input = clear.closest(".filter-input-container").querySelector(".filter-input");
      setQuery("");
      // @value={{@query}}
      input.value = "";
      input.focus();
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

  // dTabToSibling's isFocusable.
  function isFocusable(element) {
    var tabindex = element.getAttribute("tabindex");
    if (tabindex === "-1" || !element.offsetParent || element.disabled) {
      return false;
    }
    return ["A", "BUTTON", "INPUT", "SELECT", "TEXTAREA", "SUMMARY"].indexOf(element.tagName) !== -1 || tabindex !== null;
  }

  // dTabToSibling: Tab and Shift+Tab from an entry (or anything in it)
  // to the next or previous focusable sibling, else the browser's.
  function tabToSibling(entry, event) {
    var property = event.shiftKey ? "previousElementSibling" : "nextElementSibling";
    var sibling = entry[property];
    while (sibling && !isFocusable(sibling)) {
      sibling = sibling[property];
    }
    if (sibling) {
      event.preventDefault();
      sibling.focus();
    }
  }

  // handleKeypress, and dTabToSibling.
  document.addEventListener("keydown", function (event) {
    var entry = event.target.closest && event.target.closest(".c-search .chat-message-search-entry[data-url]");
    if (!entry) {
      return;
    }
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      visitMessage(entry);
    } else if (event.key === "Tab") {
      tabToSibling(entry, event);
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
      loadMoreTimer: null,
      observer: null,
    };
    var s = current;
    // DLoadMore's dObserveIntersection (threshold 0, the document as its
    // root): onIntersection debounces the action by 100ms, and loadMore
    // checks for more and a search in flight when it runs.
    s.observer = window.IntersectionObserver
      ? new IntersectionObserver(
          function (entries) {
            entries.forEach(function (entry) {
              if (!entry.isIntersecting) {
                return;
              }
              clearTimeout(s.loadMoreTimer);
              s.loadMoreTimer = setTimeout(function () {
                if (current === s && s.hasMore && !s.loading) {
                  searchMessages(false);
                }
              }, 100);
            });
          },
          { threshold: 0, rootMargin: "0px 0px 0px 0px", root: document },
        )
      : { observe: function () {}, unobserve: function () {}, disconnect: function () {} };
    // The full page's @query is its q param.
    if (root.dataset.queryParams === "true") {
      query = new URLSearchParams(location.search).get("q") || null;
    }
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
    clearTimeout(current.loadMoreTimer);
    current = null;
  }

  // For chat-drawer.js: its search route drawn, or gone.
  Discourse.chatSearch = { mount: mount, unmount: unmount };

  // The full page's search, on each page that has it.
  Discourse.onPage(function () {
    if (!document.querySelector(".c-search[data-url][data-query-params='true']")) {
      return null;
    }
    mount();
    return unmount;
  });
})();
