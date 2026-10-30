// The chat drawer (ChatDrawer in the below-footer outlet), for a member
// who can chat, on desktop. The server draws each of the drawer's routes
// (GET /live/chat/drawer, plugins/chat/drawer.rs); this keeps what Ember's
// services keep in the browser:
//
//   - ChatStateManager: the drawer open or closed, expanded or collapsed
//     (lost on a reload, as Ember's are), the full page or drawer
//     preference (localStorage discourse_chat_preferred_mode), the last
//     chat URL and the last forum URL
//   - ChatHistory: the routes the drawer visited, which the channel
//     route's back button and the footer read
//   - the chat route's beforeModel: a navigation to a chat URL the drawer
//     handles opens it in the drawer when the drawer is preferred and the
//     page isn't full-page chat (the header icon, the sidebar's channels,
//     any link); with full page preferred, or a URL the drawer doesn't
//     handle, the page goes to full-page chat and the drawer closes
//   - the header icon's active state and target, the navbar's toggle,
//     full page and close buttons and its click to toggle, full-page
//     chat's open drawer button, the "-" and Esc shortcuts
//   - the drawer's size (discourse_chat_drawer_size_*) and its offset
//     beside an open composer (_checkSize)
//   - chat-sidebar.js's active link for the drawer's channel, and
//     ChatHeader's back to the forum link (the last forum page)
//
// The outlet is kept across page swaps (hx-preserve), so the drawer stays
// open as the member moves around the forum. A channel in the drawer is
// static/js/chat.js's, loaded with its scripts when the drawer first
// opens.
(function () {
  "use strict";

  if (!Discourse.once("chat-drawer")) {
    return;
  }

  var PREFERRED_MODE = "discourse_chat_preferred_mode";
  var FULL_PAGE_CHAT = "FULL_PAGE_CHAT";
  var DRAWER_CHAT = "DRAWER_CHAT";
  var SIZE = "discourse_chat_drawer_size_";
  var MIN_RIGHT_MARGIN = 15;

  var state = {
    active: false,
    expanded: false,
    // The drawer's route: its URL, the history entry and the previous
    // route its back button follows.
    url: null,
    previous: "",
    history: [],
    chatURL: null,
    appURL: null,
    channelId: null,
    // ChatHeader's previousURL: the last forum page, for the mobile chat
    // header's back to the forum link.
    forumURL: null,
    request: 0,
  };

  // The current page's chat-drawer.js tag: its attributes follow the page.
  function config() {
    var script = document.querySelector("script[src*='/chat-drawer.js']");
    return script ? script.dataset : {};
  }

  function basePath() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  // withoutPrefix
  function withoutPrefix(url) {
    var base = basePath();
    if (base && url.indexOf(base) === 0) {
      url = url.slice(base.length);
    }
    return url || "/";
  }

  // KeyValueStore: JSON values.
  function read(key) {
    try {
      return JSON.parse(localStorage.getItem(key));
    } catch (e) {
      return null;
    }
  }

  function write(key, value) {
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch (e) {
      // No storage: the preference lasts for this page.
    }
  }

  function mobileView() {
    return document.documentElement.classList.contains("mobile-view");
  }

  function isFullPagePreferred() {
    return mobileView() || read(PREFERRED_MODE) === FULL_PAGE_CHAT;
  }

  function isDrawerPreferred() {
    return !isFullPagePreferred();
  }

  // isFullPageActive: the page is one of chat's routes.
  function isFullPageActive() {
    return !!document.querySelector(".full-page-chat");
  }

  function currentURL() {
    return withoutPrefix(location.pathname + location.search);
  }

  function lastKnownAppURL() {
    var url = state.appURL;
    if (!url || url === "/") {
      url = config().homepage || "/";
    }
    return url;
  }

  function lastKnownChatURL() {
    if (state.chatURL) {
      return state.chatURL;
    }
    if (isDrawerPreferred() && config().starred === "true") {
      return "/chat/starred-channels";
    }
    return "/chat";
  }

  function storeAppURL() {
    var url = currentURL();
    state.appURL = url.indexOf("/chat") === 0 ? "/" : url;
  }

  function path(url) {
    return url.replace(/[?#].*$/, "").replace(/\/+$/, "") || "/";
  }

  function isChatURL(url) {
    var p = path(url);
    return p === "/chat" || p.indexOf("/chat/") === 0;
  }

  // chatDrawerRouter.canHandleRoute: the routes in its ROUTES table.
  function canHandle(url) {
    var parts = path(url).split("/").slice(1);
    if (parts[0] !== "chat") {
      return false;
    }
    var rest = parts.slice(1);
    var n = rest.length;
    if (n === 0) {
      return true;
    }
    if (n === 1) {
      return ["channels", "direct-messages", "starred-channels", "threads", "search", "browse"].indexOf(rest[0]) !== -1;
    }
    if (rest[0] === "browse") {
      return n === 2 && ["all", "open", "closed", "archived"].indexOf(rest[1]) !== -1;
    }
    if (rest[0] !== "c" || n < 3) {
      return false;
    }
    var tail = rest.slice(3);
    if (tail.length === 0) {
      return true;
    }
    if (tail.length === 1) {
      return tail[0] === "t" || tail[0] === "pins" || /^\d+$/.test(tail[0]);
    }
    if (tail.length === 2) {
      return tail[0] === "t" || (tail[0] === "info" && (tail[1] === "settings" || tail[1] === "members"));
    }
    if (tail.length === 3) {
      return tail[0] === "t" || tail[1] === "t";
    }
    return false;
  }

  // The page for a URL, swapped in as a link's (hx-boost); `then` runs
  // once it is in.
  var arrival = null;
  function go(url, then) {
    arrival = then || null;
    if (!window.htmx) {
      window.location.assign(basePath() + url);
      return;
    }
    var link = document.createElement("a");
    link.href = basePath() + url;
    link.hidden = true;
    document.getElementById("main-outlet").appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  // router.transitionTo(url) as the chat route's beforeModel sees it.
  function transitionTo(url) {
    if (isChatURL(url) && !isFullPageActive()) {
      if (isDrawerPreferred() && canHandle(url)) {
        openURL(url);
        return;
      }
      close();
    }
    go(url);
  }

  // The size, at least MIN_WIDTH by MIN_HEIGHT once resized.
  function size() {
    return { width: read(SIZE + "width") || 400, height: read(SIZE + "height") || 530 };
  }

  function container() {
    return document.querySelector(".chat-drawer-outlet-container");
  }

  // The bodyClass and htmlClass the drawer adds while open.
  function applyClasses() {
    var html = document.documentElement.classList;
    var body = document.body.classList;
    ["has-drawer-chat", "has-chat"].forEach(function (name) {
      html.toggle(name, state.active || (name === "has-chat" && isFullPageActive()));
      body.toggle(name, state.active || (name === "has-chat" && isFullPageActive()));
    });
    body.toggle("chat-drawer-active", state.active);
    body.toggle("chat-drawer-expanded", state.active && state.expanded);
  }

  // ChatHeaderIcon: active while the drawer is open; its href the URL a
  // click goes to.
  function updateIcon() {
    var link = document.querySelector(".chat-header-icon a");
    if (!link) {
      return;
    }
    link.classList.toggle("active", state.active || isFullPageActive());
    link.setAttribute("href", basePath() + (state.active ? "/chat" : lastKnownChatURL()));
  }

  // chat-sidebar.js's classNames: the link of the channel the expanded
  // drawer shows is active (full-page chat's comes from the server).
  function markSidebar() {
    if (isFullPageActive()) {
      return;
    }
    var id = state.active && state.expanded ? state.channelId : null;
    document.querySelectorAll(".sidebar-section-link[class*='channel-']").forEach(function (link) {
      var match = /(?:^|\s)channel-(\d+)(?:\s|$)/.exec(link.className);
      if (!match) {
        return;
      }
      var active = match[1] === String(id);
      if (active === link.classList.contains("sidebar-section-link--active")) {
        return;
      }
      if (active) {
        link.className = link.className.replace(/(^|\s)(channel-\d+)/, "$1sidebar-section-link--active $2");
      } else {
        link.classList.remove("sidebar-section-link--active");
      }
    });
  }

  // chat.js and the scripts it needs, loaded once.
  var scripts = null;
  function loadScripts() {
    if (scripts) {
      return scripts;
    }
    var srcs = (config().scripts || "").split(" ").filter(Boolean);
    scripts = srcs.reduce(function (chain, src) {
      return chain.then(function () {
        if (document.querySelector("script[src='" + src + "']")) {
          return null;
        }
        return new Promise(function (resolve, reject) {
          var script = document.createElement("script");
          script.src = src;
          script.onload = resolve;
          script.onerror = reject;
          document.body.appendChild(script);
        });
      });
    }, Promise.resolve());
    return scripts;
  }

  function unmountChat() {
    if (Discourse.chat) {
      Discourse.chat.unmount();
    }
  }

  function mountChat() {
    loadScripts().then(function () {
      if (Discourse.chat && state.active) {
        Discourse.chat.mount();
      }
    });
  }

  // ChatHistory#visit
  function visit(name, url) {
    var current = state.history[state.history.length - 1];
    if (current && current.name === name && current.url === url) {
      return;
    }
    state.history = state.history.slice(-9).concat([{ name: name, url: url }]);
  }

  function routeURL() {
    var params = new URLSearchParams({
      url: state.url,
      expanded: String(state.expanded),
      previous: state.previous,
    });
    return basePath() + "/live/chat/drawer?" + params.toString();
  }

  function failed() {
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = "Something went wrong.";
      error.hidden = false;
    }
  }

  // The current route's markup, as the server now draws it (chat.js's
  // refresh after a change), or null when it can't.
  function fetchRoute() {
    if (!state.active || !state.url) {
      return Promise.resolve(null);
    }
    return fetch(routeURL(), { credentials: "same-origin" }).then(function (r) {
      return r.ok ? r.text() : null;
    });
  }

  // chatDrawerRouter.stateFor: the route drawn for the drawer's URL. One
  // the server refuses leaves the drawer on the route it showed.
  function load(fallback) {
    var token = ++state.request;
    return fetch(routeURL(), { credentials: "same-origin" })
      .then(function (r) {
        if (!r.ok) {
          throw new Error(r.statusText);
        }
        return r.text().then(function (html) {
          return { html: html, routes: r.headers.get("x-chat-drawer-routes") || "", channel: r.headers.get("x-chat-drawer-channel") };
        });
      })
      .then(function (drawn) {
        if (token !== state.request || !state.active) {
          return;
        }
        drawn.routes
          .split(" ")
          .filter(Boolean)
          .forEach(function (name) {
            visit(name, state.url);
          });
        render(drawn.html, drawn.channel);
      })
      .catch(function () {
        if (token !== state.request) {
          return;
        }
        if (fallback) {
          state.url = fallback.url;
          state.previous = fallback.previous;
          state.chatURL = fallback.url;
        }
        if (!container() || !container().querySelector(".chat-drawer")) {
          close();
        }
        failed();
      });
  }

  function render(html, channelId) {
    var outlet = container();
    if (!outlet) {
      return;
    }
    unmountChat();
    var drawer = outlet.querySelector(".chat-drawer");
    if (!drawer) {
      drawer = document.createElement("div");
      drawer.innerHTML = '<div class="chat-drawer-container"><div class="chat-drawer-resizer"></div></div>';
      outlet.appendChild(drawer);
    }
    drawer.className = "chat-drawer " + (state.expanded ? "is-expanded" : "is-collapsed");
    state.channelId = channelId || null;
    if (channelId) {
      drawer.setAttribute("data-chat-channel-id", channelId);
    } else {
      drawer.removeAttribute("data-chat-channel-id");
    }
    var s = size();
    drawer.setAttribute("style", "width: " + s.width + "px;height: " + s.height + "px;");
    var inner = drawer.querySelector(".chat-drawer-container");
    var old = inner.querySelector(".c-drawer-routes");
    if (old) {
      old.remove();
    }
    inner.insertAdjacentHTML("beforeend", html);
    if (window.htmx) {
      window.htmx.process(inner);
    }
    applyClasses();
    updateIcon();
    markSidebar();
    mountChat();
    checkSize();
  }

  // ChatDrawer#openURL and ChatStateManager#didOpenDrawer
  function openURL(url) {
    url = url || "/";
    var fallback = state.active && state.url ? { url: state.url, previous: state.previous } : null;
    var current = state.history[state.history.length - 1];
    if (current && current.url === url) {
      var before = state.history[state.history.length - 2];
      state.previous = before ? before.name : "";
    } else {
      state.previous = current ? current.name : "";
    }
    state.url = url;
    state.active = true;
    state.expanded = true;
    state.chatURL = url;
    applyClasses();
    updateIcon();
    load(fallback);
  }

  // didCloseDrawer
  function close() {
    if (!state.active) {
      return;
    }
    state.active = false;
    state.expanded = false;
    state.request++;
    unmountChat();
    var outlet = container();
    var drawer = outlet && outlet.querySelector(".chat-drawer");
    if (drawer) {
      drawer.remove();
    }
    state.channelId = null;
    applyClasses();
    updateIcon();
    markSidebar();
  }

  // didToggleDrawer
  function toggle() {
    state.expanded = !state.expanded;
    state.active = true;
    applyClasses();
    markSidebar();
    load();
  }

  // ChatDrawer#openInFullPage
  function openInFullPage() {
    storeAppURL();
    write(PREFERRED_MODE, FULL_PAGE_CHAT);
    transitionTo(lastKnownChatURL());
  }

  // ChatNavbarOpenDrawerButton#openDrawer
  function openDrawer() {
    write(PREFERRED_MODE, DRAWER_CHAT);
    var chatURL = lastKnownChatURL();
    go(lastKnownAppURL(), function () {
      transitionTo(chatURL);
    });
  }

  // _performCheckSize: beside an open composer, or above it when the
  // space to its end is narrower than the drawer.
  function checkSize() {
    var outlet = container();
    var composer = document.getElementById("reply-control");
    if (!outlet) {
      return;
    }
    var peek = document.body.classList.contains("peek-mode-active");
    if (!composer || composer.classList.contains("closed") || peek) {
      outlet.style.setProperty("--composer-right", MIN_RIGHT_MARGIN + "px");
      outlet.classList.remove("above-composer");
      return;
    }
    var rtl = document.documentElement.classList.contains("rtl");
    var spaceToEnd = rtl
      ? composer.offsetLeft
      : document.documentElement.clientWidth - composer.offsetLeft - composer.offsetWidth;
    var above = spaceToEnd < (size().width || 400);
    outlet.style.setProperty("--composer-right", (above ? Math.max(MIN_RIGHT_MARGIN, spaceToEnd) : MIN_RIGHT_MARGIN) + "px");
    outlet.classList.toggle("above-composer", above);
  }

  // The composer's open, closed and resized states (Ember's composer
  // events) come as its classes and style change.
  var composerObserver = null;
  function watchComposer() {
    if (composerObserver) {
      composerObserver.disconnect();
    }
    var composer = document.getElementById("reply-control");
    if (composer && window.MutationObserver) {
      composerObserver = new MutationObserver(checkSize);
      composerObserver.observe(composer, { attributes: true, attributeFilter: ["class", "style"] });
    }
  }
  window.addEventListener("resize", function () {
    if (state.active) {
      window.requestAnimationFrame(checkSize);
    }
  });

  // A boosted navigation to a chat URL: the chat route's beforeModel, and
  // its activate (the forum URL left) and deactivate (the chat URL left).
  document.addEventListener("htmx:beforeRequest", function (event) {
    var detail = event.detail;
    var config = detail.requestConfig || {};
    if (!(detail.boosted || config.boosted) || (config.verb && config.verb !== "get")) {
      return;
    }
    var url = withoutPrefix(detail.pathInfo ? detail.pathInfo.requestPath : config.path);
    var fullPage = isFullPageActive();
    if (isChatURL(url)) {
      if (fullPage) {
        return;
      }
      if (isDrawerPreferred() && canHandle(url)) {
        event.preventDefault();
        openURL(url);
        return;
      }
      close();
      storeAppURL();
    } else if (fullPage) {
      state.chatURL = withoutPrefix(location.pathname);
    }
  });

  document.addEventListener(
    "click",
    function (event) {
      if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
        return;
      }
      var target = event.target;

      // ChatHeaderIcon#openChat
      var icon = target.closest(".chat-header-icon a");
      if (icon && !mobileView()) {
        event.preventDefault();
        event.stopPropagation();
        if (isDrawerPreferred()) {
          write(PREFERRED_MODE, DRAWER_CHAT);
        }
        transitionTo(state.active ? "/chat" : lastKnownChatURL());
        return;
      }

      if (target.closest(".c-navbar__open-drawer-button")) {
        event.preventDefault();
        event.stopPropagation();
        openDrawer();
        return;
      }

      var drawer = target.closest(".chat-drawer");
      if (!drawer) {
        return;
      }
      var button = target.closest("button");
      if (button && button.classList.contains("c-navbar__toggle-drawer-button")) {
        event.stopPropagation();
        toggle();
        return;
      }
      if (button && button.classList.contains("c-navbar__full-page-button")) {
        event.stopPropagation();
        openInFullPage();
        return;
      }
      if (button && button.classList.contains("c-navbar__close-drawer-button")) {
        event.stopPropagation();
        close();
        return;
      }
      // DButton @route (the footer's items, the empty state's button).
      var routed = target.closest("[data-href]");
      if (routed && !target.closest("a[href]")) {
        event.preventDefault();
        event.stopPropagation();
        transitionTo(withoutPrefix(routed.dataset.href));
        return;
      }
      // A link to a chat route: a transition, which the drawer takes.
      var link = target.closest("a[href]");
      if (link && link.origin === location.origin && !link.target && isChatURL(withoutPrefix(link.pathname))) {
        event.preventDefault();
        event.stopPropagation();
        transitionTo(withoutPrefix(link.pathname + link.search));
        return;
      }
      // The navbar toggles the drawer on a click that isn't a button's.
      if (!button && !link && target.closest(".c-navbar-container.-clickable")) {
        toggle();
      }
    },
    true
  );

  // The "-" shortcut toggles the drawer, Esc closes it (after what has
  // the key first: the composer, a menu, the emoji picker, which stop it
  // at the document).
  function isInputSelection(el) {
    var tag = el && el.tagName ? el.tagName.toLowerCase() : "";
    return ["input", "textarea", "select", "button"].indexOf(tag) !== -1 || !!(el && el.closest && el.closest('[contenteditable="true"]'));
  }

  window.addEventListener("keydown", function (event) {
    if (event.ctrlKey || event.metaKey || event.altKey) {
      return;
    }
    if (event.key === "-" && !event.shiftKey) {
      if (isInputSelection(event.target)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      if (state.active) {
        close();
      } else {
        write(PREFERRED_MODE, DRAWER_CHAT);
        transitionTo(lastKnownChatURL());
      }
      return;
    }
    if (event.key === "Escape" && !event.shiftKey && state.active) {
      var src = event.target;
      if (src && src.className && String(src.className).indexOf("lightbox") !== -1) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      close();
    }
  });

  // chat-keyboard-shortcuts, the ones not above or in the composer:
  // Alt+Up and Alt+Down switch to the previous or next channel in the
  // sidebar's order (starred, channels, DMs; switchChannelUpOrDown),
  // with Shift only among those with unread activity; Ctrl/Cmd+B, I and
  // E surround the chat composer's selection (modifySelection); and
  // Shift+Esc marks every channel read while chat is open. All are
  // global, so they work from the composer too.
  function sidebarChannels() {
    var out = [];
    ["chat-starred-channels", "chat-channels", "chat-dms"].forEach(function (name) {
      document
        .querySelectorAll('.sidebar-section[data-section-name="' + name + '"] a.sidebar-section-link')
        .forEach(function (link) {
          var match = /(?:^|\s)channel-(\d+)(?:\s|$)/.exec(link.className);
          if (match) {
            out.push({
              id: match[1],
              url: withoutPrefix(link.pathname),
              hasUnread: !!link.querySelector(
                ".sidebar-section-link-content-badge.unread, .sidebar-section-link-content-badge.urgent"
              ),
            });
          }
        });
    });
    return out;
  }

  // #getOrderedChannelsWithActivity: those with unread activity, the
  // active one (without any) after the last of them above it.
  function channelsWithActivity(ordered, activeId) {
    var withActivity = ordered.filter(function (c) {
      return c.hasUnread;
    });
    var activeIndex = -1;
    ordered.forEach(function (c, i) {
      if (c.id === activeId) {
        activeIndex = i;
      }
    });
    if (activeIndex !== -1 && !ordered[activeIndex].hasUnread) {
      var insertAfter = -1;
      for (var i = activeIndex - 1; i >= 0; i--) {
        var at = withActivity.indexOf(ordered[i]);
        if (at !== -1) {
          insertAfter = at;
          break;
        }
      }
      withActivity.splice(insertAfter + 1, 0, ordered[activeIndex]);
    }
    return withActivity;
  }

  function switchChannel(up, unreadOnly) {
    var active = document.querySelector(".chat-channel[data-id]");
    if (!active) {
      return;
    }
    var activeId = active.dataset.id;
    var ordered = sidebarChannels();
    var channels = unreadOnly ? channelsWithActivity(ordered, activeId) : ordered;
    if (channels.length === 0) {
      return;
    }
    var current = -1;
    channels.forEach(function (c, i) {
      if (c.id === activeId) {
        current = i;
      }
    });
    var next;
    if (current === -1) {
      next = up ? channels.length - 1 : 0;
    } else {
      next = current + (up ? -1 : 1);
      if (next < 0) {
        next = channels.length - 1;
      } else if (next >= channels.length) {
        next = 0;
      }
    }
    if (channels[next] && channels[next].id !== activeId) {
      transitionTo(channels[next].url);
    }
  }

  // applySurround on the selection, or the example text selected: each
  // line of a selection over several lines wrapped by itself.
  function surround(input, head, tail, example) {
    var start = input.selectionStart;
    var end = input.selectionEnd;
    var selected = input.value.slice(start, end);
    var text;
    var selectFrom;
    var selectTo;
    if (selected === "") {
      text = head + example + tail;
      selectFrom = start + head.length;
      selectTo = selectFrom + example.length;
    } else if (selected.indexOf("\n") !== -1) {
      text = selected
        .split("\n")
        .map(function (line) {
          return line.length ? head + line + tail : line;
        })
        .join("\n");
      selectFrom = start;
      selectTo = start + text.length;
    } else {
      text = head + selected + tail;
      selectFrom = start + head.length;
      selectTo = selectFrom + selected.length;
    }
    input.focus();
    input.setSelectionRange(start, end);
    if (!document.execCommand("insertText", false, text)) {
      input.setRangeText(text, start, end, "end");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }
    input.setSelectionRange(selectFrom, selectTo);
  }

  // chatChannelsManager.markAllChannelsRead: PUT /chat/api/channels/read;
  // the tracking state comes back over the live stream.
  function markAllChannelsRead() {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request.
    }
    headers["X-Requested-With"] = "XMLHttpRequest";
    fetch(basePath() + "/chat/api/channels/read", {
      method: "PUT",
      credentials: "same-origin",
      headers: headers,
    }).catch(function () {});
  }

  var SURROUNDS = { b: ["**", "bold"], i: ["_", "italic"], e: ["`", "code"] };

  window.addEventListener("keydown", function (event) {
    if (event.altKey && !event.ctrlKey && !event.metaKey && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
      event.preventDefault();
      event.stopPropagation();
      switchChannel(event.key === "ArrowUp", event.shiftKey);
      return;
    }
    var modifier = /Mac|iPhone|iPad/.test(navigator.platform) ? event.metaKey : event.ctrlKey;
    var key = event.key && event.key.toLowerCase();
    if (modifier && !event.altKey && !event.shiftKey && SURROUNDS[key]) {
      var input = event.target;
      if (!input || !input.classList || !input.classList.contains("chat-composer__input")) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      var wrapper = input.closest("[data-surround]");
      var examples = {};
      try {
        examples = JSON.parse((wrapper && wrapper.dataset.surround) || "{}");
      } catch (e) {
        // No examples: an empty selection is wrapped as it is.
      }
      var mark = SURROUNDS[key][0];
      surround(input, mark, mark, examples[SURROUNDS[key][1]] || "");
      return;
    }
    if (event.key === "Escape" && event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey) {
      event.preventDefault();
      event.stopPropagation();
      // chatStateManager.isActive: full-page chat or the open drawer.
      if (isFullPageActive() || state.active) {
        markAllChannelsRead();
      }
    }
  });

  Discourse.chatDrawer = {
    fetchRoute: fetchRoute,
    // router.transitionTo for chat-new-message.js: a channel opens in the
    // drawer when it is preferred, else in full-page chat.
    transitionTo: transitionTo,
    hidePresence: function () {
      return config().hidePresence === "true";
    },
    isActive: function () {
      return state.active;
    },
    markSidebar: markSidebar,
  };

  // Each page: the drawer's classes and the header icon (the page's own
  // came without them), the composer it sits beside, and what waits for
  // the page (the open drawer button's chat URL).
  Discourse.onPage(function () {
    // routeDidChange: the routes after the first, which renders the
    // header after its transition.
    if (Discourse.swapped && !isChatURL(currentURL())) {
      state.forumURL = currentURL();
    }
    var forum = document.querySelector(".c-header .back-to-forum");
    if (forum) {
      forum.setAttribute("href", basePath() + (state.forumURL || "/"));
    }
    applyClasses();
    updateIcon();
    markSidebar();
    watchComposer();
    checkSize();
    // A page that came without the outlet lost the drawer: drawn again.
    var outlet = container();
    if (state.active && outlet && !outlet.querySelector(".chat-drawer")) {
      load();
    }
    if (arrival) {
      var then = arrival;
      arrival = null;
      then();
    }
  });

  // The header icon and the sidebar come again over the live stream
  // (their unread counts).
  document.addEventListener("htmx:oobAfterSwap", function () {
    updateIcon();
    markSidebar();
  });
})();
