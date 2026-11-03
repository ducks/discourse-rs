// A chat thread in the browser (ChatThread, ChatThreadHeader, ChatSidePanel)
// and the threads lists (ChatThreadList, UserThreads). chat.js runs the
// thread's composer (ChatComposerThread) and its messages' actions.
//
//   - its messages' times and date separators, as chat.js draws the
//     channel's (Discourse.chat.redraw)
//   - its live messages: the thread's bus channel (/chat/:id/thread/:id),
//     each message drawn again by the server as the pane draws it
//     (GET /live/chat/:channel_id/:message_id?context=thread)
//   - its read state: the last message shown marks the thread read
//     (PUT .../threads/:thread_id/read)
//   - its paging: older and newer pages as the member scrolls, the target
//     message scrolled to and highlighted, jumps to a message of the
//     thread, the scroll to bottom arrow (GET /live/chat/:channel_id
//     /messages?thread_id=)
//   - the header's back link from the previous chat route (ChatHistory),
//     the settings modal (ChatModalThreadSettings) from the title or the
//     gear, the title prompt's toast on a phone (ChatThreadTitlePrompt)
//   - the side panel's resizer, its width kept
//     (discourse_chat_side_panel_size_width)
//   - the threads lists: an item opens its thread, the next page loads at
//     the end, and the list follows the channels' bus
(function () {
  "use strict";

  if (!Discourse.once("chat-thread")) {
    return;
  }

  function basePath() {
    var page = document.querySelector(".full-page-chat[data-base-path]");
    if (page) {
      return page.dataset.basePath;
    }
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function csrfHeaders() {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    headers["Content-Type"] = "application/x-www-form-urlencoded";
    headers["X-Requested-With"] = "XMLHttpRequest";
    return headers;
  }

  function request(method, path, params) {
    return fetch(basePath() + path, {
      method: method,
      credentials: "same-origin",
      headers: csrfHeaders(),
      body: params ? new URLSearchParams(params).toString() : undefined,
    });
  }

  // popupAjaxError
  function failed(r) {
    return r
      .json()
      .catch(function () {
        return {};
      })
      .then(function (body) {
        window.alert((body.errors || [r.statusText]).join("\n"));
      });
  }

  function thread() {
    return document.querySelector(".chat-thread[data-id]");
  }

  function list(pane) {
    return pane && pane.querySelector(".chat-messages-container");
  }

  function scrollerOf(pane) {
    return pane.querySelector(".chat-messages-scroller");
  }

  function redraw(pane) {
    if (Discourse.chat && Discourse.chat.redraw) {
      Discourse.chat.redraw(pane);
    }
  }

  function items(container) {
    return Array.prototype.filter.call(container.children, function (node) {
      return node.matches("template.chat-message-expanded, .chat-message-container[data-id]");
    });
  }

  function find(container, id) {
    var found = null;
    items(container).forEach(function (node) {
      if (node.dataset.id === String(id) && (!found || found.matches("template"))) {
        found = node;
      }
    });
    return found;
  }

  function persisted(container) {
    return items(container).filter(function (node) {
      return !node.classList.contains("-staged") && Number(node.dataset.id) > 0;
    });
  }

  function fragment(html) {
    var template = document.createElement("template");
    template.innerHTML = html;
    return template.content.firstElementChild;
  }

  // The scroller runs bottom up: 0 is the latest message.
  function atBottom(pane) {
    var scroller = scrollerOf(pane);
    return !scroller || Math.abs(scroller.scrollTop) < 50;
  }

  function fetchMessage(pane, id, previous) {
    var url =
      basePath() + "/live/chat/" + pane.dataset.channelId + "/" + id + "?context=thread";
    if (previous) {
      url += "&previous=" + (previous.classList.contains("-staged") ? "staged" : previous.dataset.id);
    }
    return fetch(url, { credentials: "same-origin" }).then(function (r) {
      return r.ok ? r.text() : null;
    });
  }

  // ChatThread's pane state: a page loading, the next scroll ours, the
  // messages that came while scrolled away.
  var PAST = "past";
  var FUTURE = "future";
  var state = { loading: false, ignoreNextScroll: false, pending: 0, fillTimer: null };

  // handleSentMessage: the staged message becomes the server's where it
  // is, else the message comes at the end (pending while scrolled away).
  function onSent(pane, container, data) {
    var id = data.chat_message.id;
    if (find(container, id)) {
      return Promise.resolve();
    }
    // Newer pages still to load: the message comes with them.
    var staged = data.staged_id && find(container, data.staged_id);
    if (!staged && canLoadMore(pane, FUTURE)) {
      state.pending += 1;
      updateArrow(pane);
      return Promise.resolve();
    }
    var all = items(container);
    var previous = staged ? all[all.indexOf(staged) - 1] || null : all[all.length - 1] || null;
    return fetchMessage(pane, id, previous).then(function (html) {
      if (!html || find(container, id)) {
        return;
      }
      // Where the view is once the message is fetched (chat.js onSent).
      var bottom = atBottom(pane);
      var node = fragment(html);
      if (staged && staged.isConnected) {
        staged.replaceWith(node);
      } else {
        container.appendChild(node);
      }
      pane.dataset.lastMessageId = String(id);
      redraw(pane);
      if (bottom || staged) {
        state.ignoreNextScroll = true;
        scrollerOf(pane).scrollTop = 0;
      } else {
        state.pending += 1;
        updateArrow(pane);
      }
    });
  }

  // A message drawn again keeps its place in select mode: selectable,
  // its checkbox, and whether it is selected.
  function keepSelection(old, next) {
    if (!old.classList.contains("-selectable")) {
      return;
    }
    var selected = old.classList.contains("-selected");
    next.classList.add("-selectable");
    next.classList.toggle("-selected", selected);
    var box = document.createElement("input");
    box.className = "chat-message-selector";
    box.type = "checkbox";
    box.checked = selected;
    next.insertBefore(box, next.firstChild);
  }

  // The message drawn again where it is.
  function onChanged(pane, container, id) {
    var node = find(container, id);
    if (!node || node.matches("template")) {
      return Promise.resolve();
    }
    var all = items(container);
    var previous = all[all.indexOf(node) - 1] || null;
    return fetchMessage(pane, id, previous).then(function (html) {
      if (html && node.isConnected) {
        var next = fragment(html);
        keepSelection(node, next);
        node.replaceWith(next);
        redraw(pane);
      }
    });
  }

  // handleDeleteMessage: its author and staff keep it, deleted; others
  // lose it.
  function onDelete(pane, container, id) {
    var node = find(container, id);
    if (!node || node.matches("template")) {
      return Promise.resolve();
    }
    return fetchMessage(pane, id, null).then(function (html) {
      if (!node.isConnected) {
        return;
      }
      if (html) {
        node.replaceWith(fragment(html));
      } else {
        node.remove();
      }
      redraw(pane);
    });
  }

  function onBus(data) {
    var pane = thread();
    var container = list(pane);
    if (!container) {
      return Promise.resolve();
    }
    switch (data.type) {
      case "sent":
        return onSent(pane, container, data);
      case "processed":
      case "edit":
      case "refresh":
      case "restore":
        return onChanged(pane, container, data.chat_message.id);
      case "reaction":
        return onChanged(pane, container, data.chat_message_id);
      case "delete":
        return onDelete(pane, container, data.deleted_id);
    }
    return Promise.resolve();
  }

  // A bus subscription over /bus/events, its messages handled one at a
  // time and in order.
  function subscribe(channels, handle, onGap) {
    if (!window.EventSource) {
      return null;
    }
    var meta = document.querySelector("meta[name='bus-position']");
    var url =
      basePath() +
      "/bus/events?channels=" +
      encodeURIComponent(channels.join(",")) +
      (meta && meta.content ? "&position=" + encodeURIComponent(meta.content) : "");
    var source = new EventSource(url, { withCredentials: true });
    var queue = Promise.resolve();
    source.addEventListener("message", function (event) {
      var message = JSON.parse(event.data);
      queue = queue
        .then(function () {
          return handle(message);
        })
        .catch(function () {});
    });
    source.addEventListener("gap", onGap);
    return source;
  }

  var sources = [];

  function startLive(pane) {
    var source = subscribe(
      ["/chat/" + pane.dataset.channelId + "/thread/" + pane.dataset.id],
      function (message) {
        return onBus(message.data);
      },
      function () {
        window.location.reload();
      }
    );
    if (source) {
      sources.push(source);
    }
  }

  // updateLastRead: the last persisted message whose bottom shows marks
  // the thread read, never back.
  function markRead(pane) {
    if (document.visibilityState !== "visible") {
      return;
    }
    var scroller = scrollerOf(pane);
    if (!scroller) {
      return;
    }
    var view = scroller.getBoundingClientRect();
    var found = null;
    pane.querySelectorAll(".chat-message-container[data-id]:not(.-staged)").forEach(function (message) {
      var rect = message.getBoundingClientRect();
      if (rect.bottom <= view.bottom + 1 && rect.bottom >= view.top) {
        found = message;
      }
    });
    if (!found) {
      return;
    }
    var id = Number(found.dataset.id);
    if (!Number.isInteger(id) || id <= Number(pane.dataset.lastRead || 0)) {
      return;
    }
    pane.dataset.lastRead = String(id);
    request(
      "PUT",
      "/chat/api/channels/" + pane.dataset.channelId + "/threads/" + pane.dataset.id + "/read",
      { message_id: id }
    ).catch(function () {});
  }

  // ChatMessagesLoader over the thread: true, false, or null (unknown
  // after a page the other way, which loadMore still tries).
  function canLoadMore(pane, direction) {
    var value = pane.dataset[direction === PAST ? "canLoadMorePast" : "canLoadMoreFuture"];
    return value === "true" ? true : value === "false" ? false : null;
  }

  function fetchPage(pane, params) {
    var query = new URLSearchParams({ thread_id: pane.dataset.id });
    Object.keys(params).forEach(function (key) {
      if (params[key] !== null && params[key] !== undefined) {
        query.set(key, params[key]);
      }
    });
    var url = basePath() + "/live/chat/" + pane.dataset.channelId + "/messages?" + query.toString();
    return fetch(url, { credentials: "same-origin" }).then(function (r) {
      if (!r.ok) {
        return null;
      }
      return r.text().then(function (html) {
        return {
          html: html,
          past: r.headers.get("x-chat-can-load-more-past"),
          future: r.headers.get("x-chat-can-load-more-future"),
        };
      });
    });
  }

  function popupError(pane) {
    var text = pane.dataset.labelNotFound;
    if (text && Discourse.dialog) {
      Discourse.dialog.popupAjaxError(text);
    }
  }

  function highlight(node) {
    node.classList.add("-highlighted");
    setTimeout(function () {
      node.classList.remove("-highlighted");
    }, 2000);
  }

  // scrollListToMessage: a collapsed deleted message is expanded first
  // (autoExpand), then highlighted and scrolled into view.
  function scrollToMessage(pane, id, opts) {
    opts = opts || {};
    var container = list(pane);
    var node = container && find(container, id);
    if (!node) {
      return;
    }
    if (node.matches("template") || node.querySelector(":scope > .chat-message-text.-deleted")) {
      if (!opts.autoExpand || !Discourse.chat || !Discourse.chat.expand) {
        return;
      }
      var collapsed = node.matches("template") ? null : node;
      if (!collapsed) {
        var next = node.nextElementSibling;
        while (next && next.matches("template.chat-message-expanded")) {
          next = next.nextElementSibling;
        }
        collapsed = next && next.querySelector(":scope > .chat-message-text.-deleted") ? next : null;
      }
      if (collapsed) {
        Discourse.chat.expand(collapsed);
      }
      node = find(container, id);
      if (!node || node.matches("template")) {
        return;
      }
    }
    if (opts.highlight) {
      highlight(node);
    }
    state.ignoreNextScroll = true;
    node.scrollIntoView({ behavior: "auto", block: opts.position || "start" });
  }

  function scrollToBottom(pane) {
    state.ignoreNextScroll = true;
    scrollerOf(pane).scrollTo({ top: 0, behavior: "auto" });
    state.pending = 0;
    updateArrow(pane);
  }

  // scrollListToTop: the first message, the scroller running bottom up.
  function scrollToTop(pane) {
    var scroller = scrollerOf(pane);
    state.ignoreNextScroll = true;
    scroller.scrollTop = -scroller.scrollHeight;
  }

  // hasPendingContentBelow
  function updateArrow(pane) {
    var button = pane.querySelector(".chat-scroll-to-bottom__button");
    var scroller = scrollerOf(pane);
    if (!button || !scroller) {
      return;
    }
    var canScroll = scroller.scrollHeight - scroller.clientHeight > 1;
    var away = canLoadMore(pane, FUTURE) || -scroller.scrollTop > 250;
    button.classList.toggle("visible", canScroll && (state.pending > 0 || away));
  }

  function settle(pane, container) {
    if (Discourse.chat && Discourse.chat.collapseRuns) {
      Discourse.chat.collapseRuns(pane, container);
    }
    redraw(pane);
  }

  // fetchMessages: the page around a message (or from the first), in
  // place of the one shown.
  function loadMessages(pane, target) {
    if (state.loading) {
      return Promise.resolve();
    }
    state.loading = true;
    pane.classList.add("--loading");
    return fetchPage(pane, { target_message_id: target })
      .then(function (page) {
        var container = list(pane);
        if (!page) {
          popupError(pane);
          return;
        }
        if (!container || !pane.isConnected) {
          return;
        }
        pane.dataset.canLoadMorePast = String(page.past);
        pane.dataset.canLoadMoreFuture = String(page.future);
        container.innerHTML = page.html;
        settle(pane, container);
        if (target) {
          scrollToMessage(pane, target, { highlight: true });
        } else {
          scrollToTop(pane);
        }
        scheduleFill(pane);
      })
      .catch(function () {})
      .finally(function () {
        state.loading = false;
        pane.classList.remove("--loading");
        updateArrow(pane);
      });
  }

  // fetchMoreMessages: the next page before the first message or after
  // the last; the first drawn again after the older ones.
  function loadMore(pane, direction) {
    if (state.loading || canLoadMore(pane, direction) === false) {
      return Promise.resolve();
    }
    var container = list(pane);
    var shown = container ? persisted(container) : [];
    if (!shown.length) {
      return Promise.resolve();
    }
    var anchor = direction === PAST ? shown[0] : shown[shown.length - 1];
    var anchorId = anchor.dataset.id;
    state.loading = true;
    pane.classList.add("--loading");
    return fetchPage(pane, { target_message_id: anchorId, direction: direction })
      .then(function (page) {
        if (!page || !container.isConnected) {
          return;
        }
        pane.dataset.canLoadMorePast = String(page.past);
        pane.dataset.canLoadMoreFuture = String(page.future);
        var template = document.createElement("template");
        template.innerHTML = page.html;
        var redrawn = template.content.querySelector("template.chat-message-anchor");
        if (redrawn) {
          redrawn.remove();
        }
        var added = template.content.querySelectorAll(":scope > .chat-message-container, :scope > template").length;
        if (direction === PAST) {
          var first = find(container, anchorId);
          if (redrawn && first && !first.matches("template") && !first.querySelector(":scope > .chat-message-text.-deleted")) {
            var next = redrawn.content.firstElementChild;
            next.className = first.className;
            first.replaceWith(next);
          }
          container.insertBefore(template.content, container.firstChild);
        } else {
          var staged = container.querySelector(":scope > .chat-message-container.-staged");
          container.insertBefore(template.content, staged);
        }
        settle(pane, container);
        if (added) {
          scrollToMessage(pane, anchorId, { position: direction === FUTURE ? "end" : "start" });
          scheduleFill(pane);
        }
      })
      .catch(function () {})
      .finally(function () {
        state.loading = false;
        pane.classList.remove("--loading");
        updateArrow(pane);
      });
  }

  // fillPaneAttempt: while the first message's top shows (and under 200
  // messages), the older page.
  function scheduleFill(pane) {
    clearTimeout(state.fillTimer);
    state.fillTimer = setTimeout(function () {
      var container = list(pane);
      var scroller = scrollerOf(pane);
      if (!container || !scroller || !pane.isConnected || !canLoadMore(pane, PAST)) {
        return;
      }
      var shown = persisted(container);
      if (!shown.length || shown.length > 200 || shown[0].matches("template")) {
        return;
      }
      if (shown[0].getBoundingClientRect().top - scroller.getBoundingClientRect().top >= -1) {
        loadMore(pane, PAST);
      }
    }, 500);
  }

  // highlightOrFetchMessage
  function jumpToMessage(pane, id) {
    var container = list(pane);
    if (container && find(container, id)) {
      scrollToMessage(pane, id, { highlight: true, position: "start", autoExpand: true });
      return Promise.resolve();
    }
    return loadMessages(pane, id);
  }

  // scrollToLatestMessage: around the thread's last message while newer
  // pages are still to load.
  function scrollToLatest(pane) {
    if (canLoadMore(pane, FUTURE)) {
      return loadMessages(pane, pane.dataset.lastMessageId || null).then(function () {
        scrollToBottom(pane);
      });
    }
    scrollToBottom(pane);
    return Promise.resolve();
  }

  // ChatThread#onScroll and #onScrollEnd over ChatMessagesScroller: the
  // older page under 40% from the top, the newer at the bottom.
  function setupPane(pane, scheduleRead) {
    var scroller = scrollerOf(pane);
    if (!scroller) {
      return function () {};
    }
    state.pending = 0;
    var last = scroller.scrollTop;
    var throttled = null;
    var endTimer = null;
    var measure = function () {
      var top = scroller.scrollTop;
      var height = scroller.scrollHeight - scroller.clientHeight;
      var toTop = height - Math.abs(top);
      return {
        up: top === last ? false : top < last,
        atTop: toTop <= 1,
        atBottom: -top <= 1,
        toTopPercentage: height > 0 ? Math.round((toTop / height) * 100) : 100,
      };
    };
    var onScroll = function (s) {
      if (state.ignoreNextScroll) {
        state.ignoreNextScroll = false;
        return;
      }
      updateArrow(pane);
      scheduleRead();
      var ios = document.documentElement.classList.contains("ios-device");
      if (s.atTop || (!ios && s.up && s.toTopPercentage < 40)) {
        loadMore(pane, PAST);
      } else if (s.atBottom) {
        loadMore(pane, FUTURE);
      }
    };
    var onScrollEnd = function (s) {
      if (s.atBottom) {
        state.pending = 0;
        loadMore(pane, FUTURE);
      }
      updateArrow(pane);
    };
    var compute = function () {
      if (!throttled) {
        var s = measure();
        last = scroller.scrollTop;
        setTimeout(function () {
          onScroll(s);
        }, 0);
        throttled = setTimeout(function () {
          throttled = null;
        }, 50);
      }
      clearTimeout(endTimer);
      endTimer = setTimeout(function () {
        onScrollEnd(measure());
      }, 250);
    };
    scroller.addEventListener("scroll", compute, { passive: true });
    scroller.addEventListener("wheel", compute, { passive: true });
    var arrow = pane.querySelector(".chat-scroll-to-bottom__button");
    var onArrow = function () {
      scrollToLatest(pane);
    };
    if (arrow) {
      arrow.addEventListener("click", onArrow);
    }
    // fetchMessages: the target (the url's, else the last read message)
    // highlighted, else the first message.
    if (pane.dataset.target) {
      scrollToMessage(pane, pane.dataset.target, { highlight: true });
    } else {
      scrollToTop(pane);
    }
    scheduleFill(pane);
    updateArrow(pane);
    return function () {
      clearTimeout(throttled);
      clearTimeout(endTimer);
      clearTimeout(state.fillTimer);
      scroller.removeEventListener("scroll", compute);
      scroller.removeEventListener("wheel", compute);
      if (arrow) {
        arrow.removeEventListener("click", onArrow);
      }
    };
  }

  // A link to a message of the thread shown (chat.channel.thread
  // .near-message): the pane jumps to it in place.
  document.addEventListener(
    "click",
    function (event) {
      if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
        return;
      }
      var link = event.target.closest("a[href]");
      var pane = thread();
      if (!link || !pane || link.target || link.origin !== location.origin) {
        return;
      }
      var base = basePath();
      var path = link.pathname.indexOf(base) === 0 ? link.pathname.slice(base.length) : link.pathname;
      var match = /^\/chat\/c\/[^/]+\/(\d+)\/t\/(\d+)\/(\d+)\/?$/.exec(path);
      if (!match || match[1] !== pane.dataset.channelId || match[2] !== pane.dataset.id) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      jumpToMessage(pane, match[3]);
    },
    true
  );

  // ChatHistory: the chat routes visited in this tab, the last ten, each
  // once in a row (visit), by the route Ember recognizes for the url.
  var HISTORY_KEY = "discourse_chat_history";

  function routeOf(pathname) {
    var base = basePath();
    var path = pathname.indexOf(base) === 0 ? pathname.slice(base.length) : pathname;
    var m;
    if ((m = /^\/chat\/c\/[^/]+\/(\d+)\/(\d+)\/t\/(\d+)\/?$/.exec(path))) {
      return { name: "chat.channel.near-message-with-thread", params: [m[1], m[2], m[3]] };
    }
    if ((m = /^\/chat\/c\/[^/]+\/(\d+)\/t\/(\d+)\/(\d+)\/?$/.exec(path))) {
      return { name: "chat.channel.thread.near-message", params: [m[1], m[2], m[3]] };
    }
    if ((m = /^\/chat\/c\/[^/]+\/(\d+)\/t\/(\d+)\/?$/.exec(path))) {
      return { name: "chat.channel.thread.index", params: [m[1], m[2]] };
    }
    if ((m = /^\/chat\/c\/[^/]+\/(\d+)\/t\/?$/.exec(path))) {
      return { name: "chat.channel.threads", params: [m[1]] };
    }
    if ((m = /^\/chat\/c\/[^/]+\/(\d+)\/pins\/?$/.exec(path))) {
      return { name: "chat.channel.pins", params: [m[1]] };
    }
    if ((m = /^\/chat\/c\/[^/]+\/(\d+)\/info(?:\/(\w+))?\/?$/.exec(path))) {
      return { name: "chat.channel-info." + (m[2] || "index"), params: [m[1]] };
    }
    if ((m = /^\/chat\/c\/[^/]+\/(\d+)\/(\d+)\/?$/.exec(path))) {
      return { name: "chat.channel.near-message", params: [m[1], m[2]] };
    }
    if ((m = /^\/chat\/c\/[^/]+\/(\d+)\/?$/.exec(path))) {
      return { name: "chat.channel.index", params: [m[1]] };
    }
    if ((m = /^\/chat\/([\w-]+)/.exec(path))) {
      return { name: "chat." + m[1], params: [] };
    }
    if (/^\/chat\/?$/.test(path)) {
      return { name: "chat.index", params: [] };
    }
    return null;
  }

  function readHistory() {
    try {
      return JSON.parse(sessionStorage.getItem(HISTORY_KEY) || "[]");
    } catch (e) {
      return [];
    }
  }

  function visit(pathname) {
    var route = routeOf(pathname);
    if (!route) {
      return;
    }
    var history = readHistory();
    var current = history[history.length - 1];
    if (current && current.name === route.name && current.params.join("/") === route.params.join("/")) {
      return;
    }
    history = history.slice(-9).concat([route]);
    try {
      sessionStorage.setItem(HISTORY_KEY, JSON.stringify(history));
    } catch (e) {
      // No storage: the back link stays the server's.
    }
  }

  // ChatThreadHeader#backLink: the threads list after the threads list,
  // or after the channel on desktop; my threads after my threads; else
  // the server's (the threads while some are unread, else the channel).
  function drawBackLink() {
    var back = document.querySelector(".c-routes.--channel-thread .c-navbar__back-button[data-channel-href]");
    if (!back) {
      return;
    }
    var history = readHistory();
    var previous = history.length > 1 ? history[history.length - 2].name : null;
    var desktop = !document.documentElement.classList.contains("mobile-view");
    var target = null;
    if (previous === "chat.channel.threads" || (previous === "chat.channel.index" && desktop)) {
      target = "threads";
    } else if (previous === "chat.threads") {
      target = "my-threads";
    }
    if (!target) {
      return;
    }
    var href = target === "threads" ? back.dataset.threadsHref : back.dataset.myThreadsHref;
    back.setAttribute("href", href);
    back.title = target === "threads" ? back.dataset.labelThreads : back.dataset.labelMyThreads;
    var indicator = back.querySelector(".chat-thread-header-unread-indicator");
    if (indicator) {
      indicator.remove();
    }
    if (target === "threads" && back.dataset.unreadIndicator) {
      back.insertAdjacentHTML("afterbegin", back.dataset.unreadIndicator);
    }
  }

  // ChatModalThreadSettings: the title saved (PUT .../threads/:id), the
  // header and the heading drawn again.
  function openSettings() {
    var pane = thread();
    var tpl = document.querySelector("template.chat-modal-thread-settings-template");
    if (!pane || !tpl || !Discourse.modal) {
      return;
    }
    var saving = false;
    var input = null;
    var buttons = [];
    var close = Discourse.modal.show({
      className: "chat-modal-thread-settings",
      title: tpl.dataset.title,
      body: function (body) {
        body.appendChild(tpl.content.querySelector(".chat-modal-thread-settings__body").cloneNode(true));
        input = body.querySelector(".chat-modal-thread-settings__title-input");
        var length = body.querySelector(".thread-title-length span");
        input.addEventListener("input", function () {
          length.textContent = String(input.value.length);
        });
        input.addEventListener("keydown", function (event) {
          if (event.key === "Enter") {
            event.preventDefault();
            save();
          }
        });
      },
      footer: function (footer) {
        footer.appendChild(tpl.content.querySelector(".chat-modal-thread-settings__footer .btn").cloneNode(true));
        buttons = Array.prototype.slice.call(footer.querySelectorAll(".btn"));
        buttons.forEach(function (button) {
          button.addEventListener("click", save);
        });
      },
    });
    function save() {
      if (saving) {
        return;
      }
      saving = true;
      buttons.forEach(function (b) {
        b.disabled = true;
      });
      var title = input.value;
      request("PUT", "/chat/api/channels/" + pane.dataset.channelId + "/threads/" + pane.dataset.id, { title: title })
        .then(function (r) {
          if (!r.ok) {
            return failed(r);
          }
          tpl.content.querySelector(".chat-modal-thread-settings__title-input").setAttribute("value", title);
          close();
          return redrawHeader();
        })
        .catch(function () {})
        .finally(function () {
          saving = false;
          buttons.forEach(function (b) {
            b.disabled = false;
          });
        });
    }
  }

  // The thread's title as the server now draws it: the navbar's and the
  // heading.
  function redrawHeader() {
    return fetch(location.href, { credentials: "same-origin" })
      .then(function (r) {
        return r.text();
      })
      .then(function (html) {
        var next = new DOMParser().parseFromString(html, "text/html");
        var title = document.querySelector(".c-routes.--channel-thread .c-navbar__title");
        var nextTitle = next.querySelector(".c-routes.--channel-thread .c-navbar__title");
        if (title && nextTitle) {
          title.replaceWith(document.importNode(nextTitle, true));
        }
        var scroller = document.querySelector(".chat-thread .chat-messages-scroller");
        var heading = scroller && scroller.querySelector(".chat-thread__heading");
        var nextHeading = next.querySelector(".chat-thread .chat-thread__heading");
        if (heading) {
          heading.remove();
        }
        if (scroller && nextHeading) {
          scroller.appendChild(document.importNode(nextHeading, true));
        }
      });
  }

  document.addEventListener("click", function (event) {
    if (
      event.target.closest(".c-navbar__thread-settings-button") ||
      event.target.closest(".c-routes.--channel-thread .c-navbar__title.clickable")
    ) {
      openSettings();
    }
  });

  // ChatThreadTitlePrompt: the toast, long with its progress bar, and the
  // prompt marked seen (updateThreadTitlePrompt).
  function titlePrompt(pane) {
    var tpl = document.querySelector("template.chat-thread-title-toast-template");
    if (pane.dataset.titlePrompt !== "true" || !tpl) {
      return;
    }
    pane.dataset.titlePrompt = "shown";
    var section = document.querySelector("section.fk-d-toasts");
    if (!section) {
      section = document.createElement("section");
      section.className = "fk-d-toasts";
      document.body.appendChild(section);
    }
    var toast = tpl.content.querySelector(".fk-d-toast").cloneNode(true);
    section.appendChild(toast);
    var timer = setTimeout(close, 5000);
    function close() {
      clearTimeout(timer);
      toast.remove();
    }
    toast.querySelector(".fk-d-default-toast__close-container button").addEventListener("click", close);
    // disableFutureThreadTitlePrompts
    toast.querySelector(".toast-hide").addEventListener("click", function () {
      close();
      request("PUT", "/u/" + encodeURIComponent(tpl.dataset.username) + ".json", {
        show_thread_title_prompts: "false",
      }).catch(function () {});
    });
    toast.querySelector(".toast-action").addEventListener("click", function () {
      close();
      openSettings();
    });
    request(
      "POST",
      "/chat/api/channels/" + pane.dataset.channelId + "/threads/" + pane.dataset.id + "/mark-thread-title-prompt-seen/me"
    ).catch(function () {});
  }

  // ChatSidePanel: on desktop full-page chat, the stored width (250 at
  // least, leaving the channel 250), and the resizer's drag
  // (resizableNode, didResize).
  var MIN_PANEL_WIDTH = 250;
  var SIZE_KEY = "discourse_chat_side_panel_size_width";

  function storedWidth() {
    try {
      var value = JSON.parse(localStorage.getItem(SIZE_KEY));
      return typeof value === "number" ? value : null;
    } catch (e) {
      return null;
    }
  }

  function storeWidth(width) {
    try {
      localStorage.setItem(SIZE_KEY, JSON.stringify(width));
    } catch (e) {
      // No storage: the width lasts the page.
    }
  }

  function setupPanel() {
    var panel = document.querySelector(".full-page-chat .chat-side-panel");
    var resizer = panel && panel.querySelector(".chat-side-panel-resizer");
    var main = document.getElementById("main-chat-outlet");
    if (!panel || !main || document.documentElement.classList.contains("mobile-view")) {
      return function () {};
    }
    var width = Math.min(storedWidth() || MIN_PANEL_WIDTH, main.clientWidth - MIN_PANEL_WIDTH);
    panel.style.width = width + "px";
    storeWidth(width);
    if (!resizer) {
      return function () {};
    }
    var start = null;
    var pageX = function (event) {
      return event.changedTouches ? event.changedTouches[0].pageX : event.pageX;
    };
    var onMove = function (event) {
      var diff = pageX(event) - start.x;
      var rtl = document.documentElement.classList.contains("rtl");
      var next = Math.ceil(rtl ? start.width + diff : start.width - diff);
      if (next >= MIN_PANEL_WIDTH && main.clientWidth - next >= MIN_PANEL_WIDTH) {
        panel.style.width = next + "px";
        storeWidth(next);
      }
    };
    var onUp = function () {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      window.removeEventListener("touchmove", onMove);
      window.removeEventListener("touchend", onUp);
    };
    var onDown = function (event) {
      event.preventDefault();
      start = { x: pageX(event), width: parseFloat(getComputedStyle(panel).width) };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
      window.addEventListener("touchmove", onMove);
      window.addEventListener("touchend", onUp);
    };
    resizer.addEventListener("mousedown", onDown);
    resizer.addEventListener("touchstart", onDown);
    return function () {
      onUp();
      resizer.removeEventListener("mousedown", onDown);
      resizer.removeEventListener("touchstart", onDown);
    };
  }

  // A threads list's next page at its end (the load more's intersection),
  // until a page comes back empty; and the loaded pages drawn again.
  function fetchThreads(more, offset) {
    return fetch(more.dataset.loadMoreUrl + "?offset=" + offset, { credentials: "same-origin" }).then(function (r) {
      if (!r.ok) {
        return null;
      }
      return r.text().then(function (html) {
        return { html: html, count: Number(r.headers.get("x-chat-count") || 0) };
      });
    });
  }

  function listItems(more) {
    return more.parentNode.querySelectorAll(":scope > .chat-thread-list-item, :scope > .c-user-thread");
  }

  // The items' container: the channel's list holds its load more, the
  // member's threads sit before theirs.
  function itemsBox(more) {
    return more.classList.contains("c-user-threads__load-more")
      ? more.parentNode.querySelector(".c-user-threads")
      : more.parentNode;
  }

  function loadMoreThreads(more) {
    if (more.dataset.loading === "true" || more.dataset.done === "true") {
      return Promise.resolve();
    }
    more.dataset.loading = "true";
    return fetchThreads(more, Number(more.dataset.offset || 0))
      .then(function (page) {
        if (!page || page.count === 0) {
          more.dataset.done = "true";
          return;
        }
        var box = itemsBox(more);
        var template = document.createElement("template");
        template.innerHTML = page.html;
        box.insertBefore(template.content, box === more.parentNode ? more : null);
        more.dataset.offset = String(Number(more.dataset.offset || 0) + page.count);
        if (!box.isConnected) {
          return;
        }
        // Still in view: the next one.
        var rect = more.getBoundingClientRect();
        if (rect.top < window.innerHeight) {
          more.dataset.loading = "false";
          return loadMoreThreads(more);
        }
      })
      .catch(function () {})
      .finally(function () {
        more.dataset.loading = "false";
      });
  }

  // The loaded pages again, in place of the items (the list's sort and
  // filter as the server applies them now).
  function reloadThreads(more, only) {
    var loaded = Math.max(Number(more.dataset.offset || 0), 10);
    var pages = [];
    for (var offset = 0; offset < loaded; offset += 10) {
      pages.push(fetchThreads(more, offset));
    }
    return Promise.all(pages).then(function (results) {
      if (results.some(function (r) {
        return !r;
      })) {
        return;
      }
      var template = document.createElement("template");
      template.innerHTML = results
        .map(function (r) {
          return r.html;
        })
        .join("");
      var box = itemsBox(more);
      if (only) {
        // UserThreads: the thread's preview, where it is.
        var next = template.content.querySelector(".c-user-thread[data-id='" + only + "']");
        var shown = box.querySelector(".c-user-thread[data-id='" + only + "']");
        if (next && shown) {
          shown.replaceWith(next);
        }
        return;
      }
      listItems(more).forEach(function (node) {
        node.remove();
      });
      var count = template.content.querySelectorAll(":scope > .chat-thread-list-item").length;
      var empty = box.querySelector(".chat-thread-list__no-threads");
      if (empty) {
        empty.hidden = count > 0;
      }
      box.insertBefore(template.content, more);
      more.dataset.offset = String(count);
    });
  }

  function setupLists() {
    var cleanups = [];
    document.querySelectorAll(".chat-thread-list__load-more, .c-user-threads__load-more").forEach(function (more) {
      if (!more.dataset.loadMoreUrl) {
        return;
      }
      if (window.IntersectionObserver) {
        var observer = new IntersectionObserver(function (entries) {
          if (entries.some(function (e) {
            return e.isIntersecting;
          })) {
            loadMoreThreads(more);
          }
        });
        observer.observe(more);
        cleanups.push(function () {
          observer.disconnect();
        });
      }
    });
    // ChatThreadList#subscribe: the channel's bus; an original message
    // deleted or restored, or a thread's preview changed, draws the list
    // again.
    var channelList = document.querySelector(".chat-thread-list[data-channel-id]");
    var channelMore = channelList && channelList.querySelector(".chat-thread-list__load-more");
    if (channelMore) {
      var timer = null;
      var source = subscribe(
        ["/chat/" + channelList.dataset.channelId],
        function (message) {
          var type = message.data && message.data.type;
          if (["delete", "restore", "update_thread_original_message", "thread_created"].indexOf(type) !== -1) {
            clearTimeout(timer);
            timer = setTimeout(function () {
              reloadThreads(channelMore).catch(function () {});
            }, 250);
          }
        },
        function () {
          reloadThreads(channelMore).catch(function () {});
        }
      );
      if (source) {
        sources.push(source);
      }
    }
    // UserThreads#trackChannel: each channel shown, a thread's preview
    // drawn again where it is.
    var userMore = document.querySelector(".c-user-threads__load-more");
    if (userMore) {
      var tracked = {};
      document.querySelectorAll(".c-user-thread[data-channel-id]").forEach(function (node) {
        tracked[node.dataset.channelId] = true;
      });
      var ids = Object.keys(tracked);
      if (ids.length) {
        var userSource = subscribe(
          ids.map(function (id) {
            return "/chat/" + id;
          }),
          function (message) {
            var data = message.data || {};
            if (data.type === "update_thread_original_message" && data.thread_id) {
              return reloadThreads(userMore, String(data.thread_id));
            }
          },
          function () {}
        );
        if (userSource) {
          sources.push(userSource);
        }
      }
    }
    return function () {
      cleanups.forEach(function (fn) {
        fn();
      });
    };
  }

  document.addEventListener("click", function (event) {
    // ChatThreadListItem#openThread
    var item = event.target.closest(".chat-thread-list-item__open-button[data-href]");
    if (item) {
      window.location.assign(item.dataset.href);
    }
  });

  var cleanup = null;

  // `focus`: the composer takes the focus (not when chat.js draws the page
  // again in place).
  function mountPane(pane, focus) {
    redraw(pane);
    startLive(pane);
    var timer = null;
    var scheduleRead = function () {
      clearTimeout(timer);
      timer = setTimeout(function () {
        markRead(pane);
      }, 1000);
    };
    document.addEventListener("visibilitychange", scheduleRead);
    var paneCleanup = setupPane(pane, scheduleRead);
    scheduleRead();
    var input = pane.querySelector("#thread-composer");
    if (focus && input && !input.disabled && !document.documentElement.classList.contains("mobile-view")) {
      input.focus({ preventScroll: true });
    }
    setTimeout(function () {
      titlePrompt(pane);
    }, 0);
    return function () {
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", scheduleRead);
      paneCleanup();
    };
  }

  function mount(focus) {
    var pane = thread();
    if (pane) {
      // The near-message routes replace themselves with the thread's
      // (ChatChannelThreadNearMessage, ChatChannelNearMessageWithThread);
      // the drawer's routes aren't in the address bar.
      var close = document.querySelector(".c-navbar__close-thread-button");
      var threadPath = location.pathname.replace(/\/t\/(\d+)\/\d+$/, "/t/$1");
      var channelHere = location.pathname.match(/^(.*\/chat\/c\/[^/]+\/\d+)\/\d+\/t\/(\d+)$/);
      if (channelHere) {
        threadPath = channelHere[1] + "/t/" + channelHere[2];
      }
      // They redirect in beforeModel, so ChatHistory never visits them.
      if (!pane.closest(".chat-drawer") && threadPath !== location.pathname && (close || !channelHere)) {
        history.replaceState(history.state, "", threadPath + location.search);
      }
    }
    visit(location.pathname);
    if (pane) {
      drawBackLink();
    }
    var parts = [setupPanel(), setupLists()];
    if (pane) {
      parts.push(mountPane(pane, focus));
    }
    cleanup = function () {
      parts.forEach(function (fn) {
        fn();
      });
    };
  }

  function unmount() {
    if (cleanup) {
      cleanup();
      cleanup = null;
    }
    sources.forEach(function (source) {
      source.close();
    });
    sources = [];
  }

  // For chat.js: the page drawn again in place (its refresh), the pane
  // set up again on the new nodes. For chat-drawer.js: the drawer's
  // thread route drawn, or gone.
  Discourse.chatThread = {
    remount: function () {
      unmount();
      mount(false);
    },
    mount: function () {
      unmount();
      mount(true);
    },
    unmount: unmount,
  };

  // Not the focus for the drawer's thread when a forum page swaps in
  // beside it.
  Discourse.onPage(function () {
    var pane = thread();
    mount(!(pane && pane.closest(".chat-drawer")));
    return unmount;
  });
})();
