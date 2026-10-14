// The search menu (components/search-menu.gjs) in the header and in the
// welcome banner, with the search service's state they share: the term,
// the type filter, the topic's "in this topic" context. The header's
// search button opens it (Header#toggleSearchMenu): a drop-down below the
// icons, a slide-in in the narrow desktop view (SearchMenuWrapper); the
// banner's input opens its results below itself. Typing asks the server
// for the results area (GET /search-menu, crate::search_menu_view) as
// SearchMenu#perform would render it: at once while topics are shown,
// else after 400ms, never in a topic's context until Enter. Enter searches
// topics, a second Enter (or Ctrl/Cmd+Enter) opens the full page; the
// assistant's items put their term in the field; results go to their page
// and log the click; the arrows move through the results. Escape, a click
// outside or the button closes it. "/" and Ctrl+Alt+F open it (or focus
// the banner's field while that is in view). With a welcome banner the
// header's button shows only while the banner's field is under the header
// (WelcomeBanner#checkViewport). In a topic's context the posts' cooked
// text highlights the term searched (decorateSearchHighlight).
(function () {
  "use strict";

  if (!Discourse.once("search-menu")) {
    return;
  }

  var DEFAULT_TYPE_FILTER = "exclude_topics";
  var SECOND_ENTER_MAX_DELAY = 15000;

  // The search service.
  var state = {
    term: "",
    typeFilter: DEFAULT_TYPE_FILTER,
    inTopicContext: false,
    pmCleared: false,
    // Whether the results shown have topics (includesTopics).
    topics: false,
    logId: null,
    loading: false,
    highlightTerm: "",
    // The context the state was for: a new one clears the topic's.
    contextKey: "",
  };
  var lastEnter = null;
  var debouncer = null;
  var request = null;
  var searchCleared = true;
  var enterHandledOnKeydown = false;

  function config() {
    return document.querySelector("script[data-search-menu]");
  }

  function data(name) {
    var script = config();
    return script ? script.dataset[name] || "" : "";
  }

  var stringsCache = null;
  function str(key) {
    if (!stringsCache) {
      try {
        stringsCache = JSON.parse(data("strings") || "{}");
      } catch (_) {
        stringsCache = {};
      }
    }
    return stringsCache[key] || key;
  }

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  function csrf() {
    var meta = document.querySelector('meta[name="csrf-token"]');
    return meta ? meta.content : "";
  }

  function mobileView() {
    return document.documentElement.classList.contains("mobile-view");
  }

  // Site#narrowDesktopView
  function narrowDesktop() {
    return !mobileView() && !window.matchMedia("(min-width: 48rem)").matches;
  }

  function contextType() {
    return data("contextType");
  }

  function contextId() {
    return data("contextId");
  }

  function escapeHTML(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;");
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

  function wantsNewWindow(event) {
    return event.ctrlKey || event.metaKey || event.shiftKey || event.button === 1;
  }

  // The page's context, and a new one clears what was for the last
  // (Search#searchContext's setter).
  function syncContext() {
    var key = contextType() + ":" + contextId();
    if (key !== state.contextKey) {
      if (state.inTopicContext) {
        state.inTopicContext = false;
        state.topics = false;
      }
      state.contextKey = key;
    }
  }

  function inPMInboxContext() {
    return !state.pmCleared && contextType() === "private_messages";
  }

  // SearchMenu#fullSearchUrl
  function fullSearchUrl(expanded) {
    var params = new URLSearchParams();
    if (state.term) {
      var q = state.term;
      if (state.inTopicContext && contextType() === "topic") {
        q += " topic:" + contextId();
      } else if (inPMInboxContext()) {
        q += " in:messages";
      }
      params.set("q", q);
    }
    if (expanded) {
      params.set("expanded", "true");
    }
    var query = params.toString();
    return basePath() + "/search" + (query ? "?" + query : "");
  }

  // The open menu: the header's or the banner's container.
  var active = null;

  function headerWrapper() {
    return document.querySelector(".d-header .search-menu.glimmer-search-menu");
  }

  function searchButton() {
    return document.getElementById("search-button");
  }

  function bannerInput() {
    return document.getElementById("welcome-banner-search-input");
  }

  function containerOf(el) {
    return el ? el.closest(".search-menu-container") : null;
  }

  function inputOf(container) {
    return container ? container.querySelector(".search-term__input") : null;
  }

  // The input row's context button and the searching area (clear,
  // advanced, or the spinner while loading).
  function renderInputRow(container) {
    var wrapper = container.querySelector(".search-input");
    if (!wrapper) {
      return;
    }
    var context = wrapper.querySelector(".search-context");
    var wantsContext = null;
    if (state.inTopicContext && contextType() === "topic") {
      wantsContext = ["in_this_topic", "in_this_topic_tooltip", "topic"];
    } else if (inPMInboxContext()) {
      wantsContext = ["in_messages", "in_messages_tooltip", "pm"];
    }
    if (wantsContext && wrapper.classList.contains("search-input--header")) {
      var html =
        '<button class="btn btn-icon-text btn-default btn-small search-context" title="' +
        escapeHTML(str(wantsContext[1])) +
        '" type="button" data-context="' +
        wantsContext[2] +
        '">' +
        icon("xmark") +
        '<span class="d-button-label">' +
        escapeHTML(str(wantsContext[0])) +
        "</span></button>";
      if (context) {
        context.outerHTML = html;
      } else {
        wrapper.insertAdjacentHTML("afterbegin", html);
      }
    } else if (context) {
      context.remove();
    }
    var searching = wrapper.querySelector(".searching");
    if (!searching) {
      return;
    }
    if (state.loading) {
      searching.innerHTML = "<div class='spinner'></div>";
    } else {
      var html2 = "";
      if (state.term) {
        html2 +=
          '<a aria-label="' +
          escapeHTML(str("clear_search")) +
          '" class="clear-search" href="" title="' +
          escapeHTML(str("clear_search")) +
          '">' +
          icon("xmark") +
          "</a>";
      }
      html2 +=
        '<button class="btn no-text btn-icon show-advanced-search btn-transparent" title="' +
        escapeHTML(str("open_advanced")) +
        '" type="button">' +
        icon("sliders") +
        '<span aria-hidden="true">​</span></button>';
      searching.innerHTML = html2;
    }
    container.classList.toggle("loading", state.loading);
  }

  // Where the results go: inline in the header's container, in a menu
  // panel below the banner's input.
  function resultsHost(container) {
    if (!container.classList.contains("menu-panel-results")) {
      return container;
    }
    var panel = container.querySelector(":scope > .menu-panel");
    if (!panel) {
      container.insertAdjacentHTML(
        "beforeend",
        '<div class="menu-panel search-menu-panel" data-max-width="500"><div class="panel-body"><div class="panel-body-contents"></div></div></div>'
      );
      panel = container.querySelector(":scope > .menu-panel");
    }
    return panel.querySelector(".panel-body-contents");
  }

  function placeResults(container, html) {
    var host = resultsHost(container);
    var current = host.querySelector(":scope > .conditional-loading-section");
    var template = document.createElement("template");
    template.innerHTML = html;
    var section = template.content.firstElementChild;
    if (current) {
      current.replaceWith(section);
    } else {
      host.appendChild(section);
    }
    highlightResults(section);
  }

  // HighlightedSearch: the term in the topic titles and the blurbs.
  function highlightResults(section) {
    section.querySelectorAll(".topic-title > span, .blurb__text > span").forEach(function (span) {
      highlight(span, state.term, "search-highlight");
    });
  }

  // highlightHTML: the words as one phrase, case-insensitive, between
  // non-word characters, in every text node under the element.
  function highlight(element, term, className, skip) {
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
        if (node.tagName === "SPAN" && node.className === className) {
          return 0;
        }
        if (skip && node.matches(skip)) {
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
      span.className = className;
      span.innerText = match[2];
      var matched = node.splitText(match.index + match[1].length);
      matched.splitText(match[2].length);
      matched.parentNode.replaceChild(span, matched);
      return 1;
    })(element);
  }

  // decorateSearchHighlight: the posts' cooked text, for a term of three
  // characters or more; unhighlightHTML when it goes.
  function highlightPosts() {
    document.querySelectorAll(".topic-post .cooked span.highlighted").forEach(function (span) {
      var parent = span.parentNode;
      parent.replaceChild(document.createTextNode(span.textContent), span);
      parent.normalize();
    });
    if (state.highlightTerm && state.highlightTerm.length > 2) {
      document.querySelectorAll(".topic-post .cooked").forEach(function (cooked) {
        // Not the solved plugin's accepted answers, which Ember draws
        // outside the post's cooked text.
        highlight(cooked, state.highlightTerm, "highlighted", ".accepted-answers");
      });
    }
  }

  // GET /search-menu for the state, into the open container.
  function perform(container) {
    if (request) {
      request.abort();
    }
    request = new AbortController();
    var params = new URLSearchParams();
    params.set("term", state.term);
    params.set("type_filter", state.typeFilter || "");
    if (contextType()) {
      params.set("context_type", contextType());
      params.set("context_id", contextId());
    }
    if (state.inTopicContext) {
      params.set("in_topic", "1");
    }
    if (state.pmCleared) {
      params.set("pm_cleared", "1");
    }
    if (state.topics) {
      params.set("topics", "1");
    }
    if (window.matchMedia("(pointer: fine)").matches) {
      params.set("keyboard", "1");
    }
    if (/Mac|iPhone|iPad|iPod/.test(navigator.platform || navigator.userAgent)) {
      params.set("apple", "1");
    }
    var signal = request.signal;
    return fetch(basePath() + "/search-menu?" + params.toString(), {
      credentials: "same-origin",
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
      signal: signal,
    })
      .then(function (response) {
        return response.json().then(function (body) {
          if (!response.ok) {
            var message = (body.errors || []).join(" ") || body.message || "Something went wrong.";
            throw new Error(message);
          }
          return body;
        });
      })
      .then(function (body) {
        state.topics = !!body.topics;
        state.logId = body.search_log_id || null;
        state.loading = false;
        var c = currentContainer(container);
        if (c) {
          placeResults(c, body.html);
          renderInputRow(c);
        }
      })
      .catch(function (e) {
        if (e.name === "AbortError") {
          return;
        }
        state.loading = false;
        var c = currentContainer(container);
        if (c) {
          renderInputRow(c);
        }
        var error = document.getElementById("page-error");
        if (error) {
          error.textContent = e.message;
          error.hidden = false;
        }
      });
  }

  // The container still in the page (a swap may have replaced it).
  function currentContainer(container) {
    if (container && container.isConnected) {
      return container;
    }
    return active && active.isConnected ? active : null;
  }

  // SearchMenu#triggerSearch
  function triggerSearch(container) {
    var includesTopics = state.topics || state.typeFilter !== DEFAULT_TYPE_FILTER;
    clearTimeout(debouncer);
    if (includesTopics) {
      if (state.inTopicContext && contextType() === "topic") {
        state.highlightTerm = state.term;
        highlightPosts();
      }
      state.loading = true;
      renderInputRow(container);
      perform(container);
    } else {
      state.loading = false;
      renderInputRow(container);
      if (!state.inTopicContext) {
        debouncer = setTimeout(function () {
          perform(container);
        }, 400);
      } else {
        // Results#inTopicContext: nothing until Enter.
        placeResults(
          container,
          '<div class="conditional-loading-section"><div class="results"></div></div>'
        );
      }
    }
  }

  // SearchMenu#searchTermChanged
  function searchTermChanged(container, term, opts) {
    opts = opts || {};
    state.typeFilter = opts.searchTopics ? null : DEFAULT_TYPE_FILTER;
    if (opts.setTopicContext) {
      state.inTopicContext = true;
    }
    if (opts.setPMInboxContext) {
      state.pmCleared = false;
    }
    state.term = term;
    var input = inputOf(container);
    if (input && input.value !== term) {
      input.value = term;
    }
    triggerSearch(container);
  }

  // The header's menu (SearchMenuWrapper).
  function openHeaderMenu() {
    var button = searchButton();
    if (!button || headerWrapper()) {
      return;
    }
    syncContext();
    var animation = narrowDesktop() ? "slide-in" : "drop-down";
    var html =
      '<div aria-live="polite" class="search-menu glimmer-search-menu">' +
      '<div class="menu-panel ' +
      animation +
      ' search-menu-panel" data-max-width="500"><div class="panel-body"><div class="panel-body-contents">' +
      '<div class="search-menu-container"><div class="search-input-wrapper"><div class="search-input search-input--header">' +
      '<input aria-label="' +
      escapeHTML(str("title")) +
      '" autocomplete="off" class="search-term__input" enterkeyhint="search" id="icon-search-input" placeholder="' +
      escapeHTML(str("title")) +
      '" value="" type="search">' +
      '<div class="searching"></div></div></div></div></div></div></div></div>';
    button.closest("ul").insertAdjacentHTML("afterend", html);
    button.closest("li").classList.add("active");
    button.setAttribute("aria-expanded", "");
    var container = headerWrapper().querySelector(".search-menu-container");
    active = container;
    var input = inputOf(container);
    input.value = state.term;
    renderInputRow(container);
    // The results the state had, or the initial options.
    state.loading = false;
    perform(container);
    input.focus();
    input.select();
  }

  function closeHeaderMenu(focusButton) {
    var wrapper = headerWrapper();
    if (!wrapper) {
      return;
    }
    wrapper.remove();
    clearTimeout(debouncer);
    if (request) {
      request.abort();
    }
    state.loading = false;
    var button = searchButton();
    if (button) {
      button.closest("li").classList.remove("active");
      button.removeAttribute("aria-expanded");
      if (focusButton) {
        button.focus();
      }
    }
    if (active && !active.isConnected) {
      active = null;
    }
    state.highlightTerm = "";
    highlightPosts();
  }

  // The banner's menu panel (SearchMenu without inline results).
  function openBannerMenu(container) {
    if (!bannerInViewport) {
      return;
    }
    syncContext();
    active = container;
    if (!container.querySelector(":scope > .menu-panel")) {
      perform(container);
    }
  }

  function closeBannerMenu(blur) {
    var input = bannerInput();
    var container = containerOf(input);
    if (!container) {
      return;
    }
    var panel = container.querySelector(":scope > .menu-panel");
    if (panel) {
      panel.remove();
    }
    if (blur && input) {
      input.blur();
    }
  }

  function close(container, focusButton) {
    if (container && container.closest(".welcome-banner")) {
      closeBannerMenu(true);
    } else {
      closeHeaderMenu(focusButton);
    }
  }

  function fullSearch(container) {
    state.loading = false;
    go(fullSearchUrl(false));
    close(container, false);
  }

  // Search#handleArrowUpOrDown
  function handleArrows(event) {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") {
      return false;
    }
    var focused = event.target.closest(".search-menu") ? event.target : null;
    if (!focused) {
      return false;
    }
    var menu = focused.closest(".search-menu");
    var grouped = ".results [data-search-menu-navigation-item]:not([tabindex='-1'])";
    var focusable = menu.querySelectorAll(".results a, " + grouped);
    var items = menu.querySelectorAll(".results .search-link, " + grouped);
    if (!items.length) {
      return false;
    }
    var previous, item;
    focusable.forEach(function (el) {
      if (el.classList.contains("search-link") || el.hasAttribute("data-search-menu-navigation-item")) {
        previous = el;
      }
      if (el === focused) {
        item = previous;
      }
    });
    var index = item ? Array.prototype.indexOf.call(items, item) : -1;
    if (index === -1 && event.key === "ArrowDown") {
      var first = menu.querySelector(".search-result-topic .search-link, .search-result-post .search-link") || items[0];
      first.focus();
    } else if (index === 0 && event.key === "ArrowUp") {
      var input = menu.querySelector(".search-term__input");
      if (input) {
        input.focus();
      }
    } else if (index > -1) {
      index += event.key === "ArrowDown" ? 1 : -1;
      if (index >= 0 && index < items.length) {
        items[index].focus();
      }
    }
    event.stopPropagation();
    event.preventDefault();
    return true;
  }

  // AssistantItem#itemSelected
  var itemTimer = null;
  function itemSelected(container, li) {
    clearTimeout(itemTimer);
    itemTimer = setTimeout(function () {
      var updated = li.dataset.term || "";
      var allTopics = li.hasAttribute("data-all-topics");
      var topicContext = contextType() === "topic";
      var searchTopics = !topicContext || !!state.term;
      var opts = { searchTopics: searchTopics };
      if (topicContext && !allTopics) {
        opts.setTopicContext = true;
      }
      if (!allTopics && contextType() === "private_messages") {
        opts.setPMInboxContext = true;
      }
      searchTermChanged(container, updated, opts);
      var input = inputOf(container);
      if (input) {
        input.focus();
      }
    }, 100);
  }

  function logClick(li) {
    if (!state.logId || !li) {
      return;
    }
    var body = new URLSearchParams();
    body.set("search_log_id", state.logId);
    body.set("search_result_id", li.dataset.resultId || "");
    body.set("search_result_type", li.dataset.resultType || "");
    fetch(basePath() + "/search/click", {
      method: "POST",
      credentials: "same-origin",
      headers: {
        "X-CSRF-Token": csrf(),
        "X-Requested-With": "XMLHttpRequest",
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
      },
      body: body.toString(),
    });
  }

  // The header's button, outside the mobile view (which goes to the full
  // page).
  document.addEventListener("click", function (event) {
    var button = event.target.closest("#search-button");
    if (!button || mobileView()) {
      return;
    }
    if (wantsNewWindow(event)) {
      return;
    }
    event.preventDefault();
    if (headerWrapper()) {
      closeHeaderMenu(true);
    } else {
      openHeaderMenu();
    }
    button.blur();
  });

  // Clicks in a menu.
  document.addEventListener("click", function (event) {
    var container = containerOf(event.target);
    if (!container || !container.closest(".search-menu")) {
      return;
    }
    var target = event.target;
    var contextButton = target.closest(".search-context");
    if (contextButton) {
      if (contextButton.dataset.context === "topic") {
        state.inTopicContext = false;
      } else {
        state.pmCleared = true;
      }
      renderInputRow(container);
      perform(container);
      return;
    }
    if (target.closest(".clear-search")) {
      event.preventDefault();
      event.stopPropagation();
      state.term = "";
      var input = inputOf(container);
      input.value = "";
      input.focus();
      triggerSearch(container);
      return;
    }
    if (target.closest(".show-advanced-search")) {
      state.loading = false;
      go(fullSearchUrl(true));
      close(container, false);
      return;
    }
    if (target.closest(".clear-recent-searches")) {
      fetch(basePath() + "/u/recent-searches", {
        method: "DELETE",
        credentials: "same-origin",
        headers: {
          "X-CSRF-Token": csrf(),
          "X-Requested-With": "XMLHttpRequest",
          Accept: "application/json",
        },
      }).then(function (response) {
        if (response.ok) {
          var recent = container.querySelector(".search-menu-recent");
          if (recent) {
            recent.remove();
          }
        }
      });
      return;
    }
    var tip = target.closest(".search-random-quick-tip .tip-label");
    if (tip) {
      if (target.classList.contains("tip-clickable")) {
        event.preventDefault();
        event.stopPropagation();
        searchTermChanged(container, tip.textContent.trim());
        inputOf(container).focus();
      }
      return;
    }
    var assistant = target.closest(".search-menu-assistant-item");
    if (assistant) {
      event.preventDefault();
      itemSelected(container, assistant);
      return;
    }
    var more = target.closest(".search-menu__show-more a");
    if (more) {
      event.preventDefault();
      if (more.getAttribute("href")) {
        close(container, false);
        go(more.getAttribute("href"));
      } else {
        // moreOfType
        state.typeFilter = "topic";
        triggerSearch(container);
        close(container, false);
      }
      return;
    }
    var link = target.closest(".results li.item a.search-link");
    if (link) {
      logClick(link.closest("li.item"));
      if (wantsNewWindow(event)) {
        return;
      }
      event.preventDefault();
      var href = link.getAttribute("href");
      close(container, false);
      go(href);
    }
  });

  document.addEventListener("input", function (event) {
    if (!event.target.matches(".search-menu .search-term__input")) {
      return;
    }
    var container = containerOf(event.target);
    // parseAndUpdateSearchTerm: zero-width characters out.
    var value = event.target.value.replace(/[​-‍﻿]/, "");
    if (value !== state.term) {
      searchTermChanged(container, value);
    }
  });

  document.addEventListener("focusin", function (event) {
    if (event.target.id === "welcome-banner-search-input") {
      openBannerMenu(containerOf(event.target));
    }
  });

  document.addEventListener(
    "keydown",
    function (event) {
      var input = event.target.matches && event.target.matches(".search-menu .search-term__input") ? event.target : null;
      if (input) {
        var container = containerOf(input);
        if (event.key === "Backspace") {
          searchCleared = !input.value;
        }
        if (event.key === "Escape") {
          close(container, true);
          event.preventDefault();
          event.stopPropagation();
          return;
        }
        if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
          enterHandledOnKeydown = true;
          fullSearch(container);
          event.preventDefault();
        }
        return;
      }
      var inMenu = event.target.closest && event.target.closest(".search-menu");
      if (inMenu) {
        var c = containerOf(event.target) || inMenu.querySelector(".search-menu-container");
        if (event.key === "Escape") {
          close(c, true);
          event.preventDefault();
          return;
        }
        var assistant = event.target.closest(".search-menu-assistant-item");
        if (assistant && event.key === "Enter") {
          itemSelected(c, assistant);
          event.preventDefault();
          return;
        }
        var item = event.target.closest(".results li.item");
        if (item && event.key === "Enter") {
          event.preventDefault();
          event.stopPropagation();
          logClick(item);
          var link = item.querySelector("a.search-link");
          if (link && link.href) {
            if (event.ctrlKey || event.metaKey) {
              window.open(link.href, "_blank", "noopener,noreferrer");
            } else {
              close(c, false);
              go(link.getAttribute("href"));
            }
          }
          return;
        }
        handleArrows(event);
        return;
      }
      // The header's keyboard trigger: "/" (not while typing) and
      // Ctrl+Alt+F.
      var typing =
        event.target.closest &&
        event.target.closest("input, textarea, select, [contenteditable], .d-editor");
      var slash = event.key === "/" && !typing && !event.ctrlKey && !event.metaKey && !event.altKey;
      var ctrlAltF = event.ctrlKey && event.altKey && (event.key === "f" || event.key === "F");
      if ((slash || ctrlAltF) && config()) {
        if (bannerInViewport && bannerInput()) {
          bannerInput().focus();
          event.preventDefault();
          return;
        }
        if (mobileView()) {
          return;
        }
        event.preventDefault();
        if (headerWrapper()) {
          closeHeaderMenu(true);
        } else {
          openHeaderMenu();
        }
      }
    },
    true
  );

  document.addEventListener("keyup", function (event) {
    if (!event.target.matches || !event.target.matches(".search-menu .search-term__input")) {
      return;
    }
    var input = event.target;
    var container = containerOf(input);
    if (event.key === "Enter" && enterHandledOnKeydown) {
      enterHandledOnKeydown = false;
      return;
    }
    if (container.closest(".welcome-banner")) {
      openBannerMenu(container);
    }
    if (handleArrows(event)) {
      return;
    }
    if (event.key === "Enter") {
      var recent = lastEnter && Date.now() - lastEnter < SECOND_ENTER_MAX_DELAY;
      if (state.typeFilter !== DEFAULT_TYPE_FILTER && recent) {
        fullSearch(container);
      } else {
        state.typeFilter = null;
        triggerSearch(container);
      }
      lastEnter = Date.now();
    }
    if (event.key === "Backspace" && !input.value) {
      if (searchCleared) {
        state.inTopicContext = false;
        state.pmCleared = true;
        renderInputRow(container);
      }
      searchCleared = true;
    }
    event.preventDefault();
  });

  // A press outside closes: the header's menu (dCloseOnClickOutside, the
  // button excepted) and the banner's panel (onDocumentPress).
  function onPress(event) {
    var target = event.target;
    if (headerWrapper() && !target.closest(".search-menu-panel") && !target.closest(".search-dropdown")) {
      closeHeaderMenu(false);
    }
    var input = bannerInput();
    var banner = containerOf(input);
    if (banner && banner.querySelector(":scope > .menu-panel") && !target.closest(".search-menu-container.menu-panel-results")) {
      closeBannerMenu(true);
    }
  }
  document.addEventListener("mousedown", onPress);
  document.addEventListener("touchend", onPress);

  // WelcomeBanner#checkViewport: the banner's field is usable while any
  // of it shows below the header; the header's button is there only
  // when it is not (Icons#showSearchButton, search_icon).
  var bannerInViewport = false;

  function headerOffset() {
    var header = document.querySelector(".d-header-wrap");
    return header ? header.getBoundingClientRect().bottom : 0;
  }

  function headerButtonHTML() {
    return (
      '<li class="header-dropdown-toggle search-dropdown"><button aria-label="' +
      escapeHTML(str("title")) +
      '" class="btn no-text btn-icon icon btn-flat" id="search-button" title="' +
      escapeHTML(str("title")) +
      '" aria-haspopup="true" type="button">' +
      icon("magnifying-glass") +
      ' <span aria-hidden="true"> ​ </span> </button></li>'
    );
  }

  function setBannerInViewport(value) {
    bannerInViewport = value;
    var icons = document.querySelector(".d-header .d-header-icons");
    if (!icons || mobileView() || narrowDesktop()) {
      return;
    }
    var li = icons.querySelector(".search-dropdown");
    if (value && li) {
      closeHeaderMenu(false);
      li.remove();
    } else if (!value && !li) {
      icons.insertAdjacentHTML("afterbegin", headerButtonHTML());
    }
    if (value) {
      return;
    }
    // hideResults: the banner's panel closes.
    closeBannerMenu(false);
  }

  Discourse.onPage(function () {
    syncContext();
    if (state.highlightTerm) {
      state.highlightTerm = "";
    }
    var menu = document.querySelector(".welcome-banner__search-menu");
    if (!menu || !config()) {
      bannerInViewport = false;
      return;
    }
    var check = function () {
      var usable = menu.getBoundingClientRect().bottom > headerOffset();
      if (usable !== bannerInViewport) {
        setBannerInViewport(usable);
      }
    };
    bannerInViewport = true;
    var thresholds = [];
    for (var i = 0; i <= 100; i++) {
      thresholds.push(i / 100);
    }
    var observer = new IntersectionObserver(check, { threshold: thresholds });
    observer.observe(menu.closest(".welcome-banner") || menu);
    check();
    return function () {
      observer.disconnect();
      bannerInViewport = false;
    };
  });
})();
