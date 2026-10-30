// A chat thread in the browser (ChatThread and ChatComposerThread): the
// side panel's thread pane that chat.js leaves alone.
//
//   - its messages' times and date separators, as chat.js draws the
//     channel's (Discourse.chat.redraw)
//   - its composer: the send shortcut or button sends to the thread (POST
//     /chat/:channel_id with thread_id), the message staged until the bus
//     brings the server's; the draft saved to the thread two seconds
//     after typing stops; Escape goes back to the channel
//   - its live messages: the thread's bus channel (/chat/:id/thread/:id),
//     each message drawn again by the server as the pane draws it
//     (GET /live/chat/:channel_id/:message_id?context=thread)
//   - its read state: the last message shown marks the thread read
//     (PUT .../threads/:thread_id/read)
//   - the threads list: an item opens its thread
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

  function thread() {
    return document.querySelector(".chat-thread[data-id]");
  }

  function list(pane) {
    return pane && pane.querySelector(".chat-messages-container");
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

  function fragment(html) {
    var template = document.createElement("template");
    template.innerHTML = html;
    return template.content.firstElementChild;
  }

  // The scroller runs bottom up: 0 is the latest message.
  function atBottom(pane) {
    var scroller = pane.querySelector(".chat-messages-scroller");
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

  // handleSentMessage: the staged message becomes the server's where it
  // is, else the message comes at the end.
  function onSent(pane, container, data) {
    var id = data.chat_message.id;
    if (find(container, id)) {
      return Promise.resolve();
    }
    var staged = data.staged_id && find(container, data.staged_id);
    var bottom = atBottom(pane);
    var all = items(container);
    var previous = staged ? all[all.indexOf(staged) - 1] || null : all[all.length - 1] || null;
    return fetchMessage(pane, id, previous).then(function (html) {
      if (!html || find(container, id)) {
        return;
      }
      var node = fragment(html);
      if (staged && staged.isConnected) {
        staged.replaceWith(node);
      } else {
        container.appendChild(node);
      }
      redraw(pane);
      if (bottom) {
        pane.querySelector(".chat-messages-scroller").scrollTop = 0;
      }
    });
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
        node.replaceWith(fragment(html));
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

  var live = null;

  function startLive(pane) {
    if (!window.EventSource) {
      return;
    }
    var meta = document.querySelector("meta[name='bus-position']");
    var channel = "/chat/" + pane.dataset.channelId + "/thread/" + pane.dataset.id;
    var url =
      basePath() +
      "/bus/events?channels=" +
      encodeURIComponent(channel) +
      (meta && meta.content ? "&position=" + encodeURIComponent(meta.content) : "");
    var source = new EventSource(url, { withCredentials: true });
    var queue = Promise.resolve();
    source.addEventListener("message", function (event) {
      var message = JSON.parse(event.data);
      queue = queue
        .then(function () {
          return onBus(message.data);
        })
        .catch(function () {});
    });
    source.addEventListener("gap", function () {
      window.location.reload();
    });
    live = source;
  }

  // updateLastRead: the last persisted message whose bottom shows marks
  // the thread read, never back.
  function markRead(pane) {
    if (document.visibilityState !== "visible") {
      return;
    }
    var scroller = pane.querySelector(".chat-messages-scroller");
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

  function parts(pane) {
    var input = pane.querySelector("#thread-composer");
    if (!input) {
      return null;
    }
    var wrapper = input.closest(".chat-composer__wrapper");
    return {
      wrapper: wrapper,
      composer: wrapper.querySelector(".chat-composer"),
      input: input,
      send: wrapper.querySelector(".chat-composer-button.-send"),
    };
  }

  function hasContent(p) {
    var min = Number(p.wrapper.dataset.minLength || 1) || 1;
    return p.input.value.length >= min;
  }

  function updateSendState(p) {
    var enabled = hasContent(p) && !p.input.disabled;
    p.composer.classList.toggle("is-send-enabled", enabled);
    p.composer.classList.toggle("is-send-disabled", !enabled);
    p.send.disabled = !enabled;
    p.send.tabIndex = enabled ? 0 : -1;
  }

  function refreshHeight(input) {
    input.style.height = "auto";
    input.style.height = input.scrollHeight + 1 + "px";
  }

  var draftTimer = null;

  // persistDraft, to the thread.
  function saveDraft(pane, p) {
    clearTimeout(draftTimer);
    p.composer.classList.remove("is-draft-saved");
    p.composer.classList.add("is-draft-unsaved");
    var message = p.input.value;
    draftTimer = setTimeout(function () {
      request(
        "POST",
        "/chat/api/channels/" + pane.dataset.channelId + "/threads/" + pane.dataset.id + "/drafts",
        { data: message.length ? JSON.stringify({ message: message }) : "" }
      )
        .then(function () {
          p.composer.classList.remove("is-draft-unsaved");
          p.composer.classList.add("is-draft-saved");
        })
        .catch(function () {});
    }, 2000);
  }

  // The member's message staged at the end of the thread until the bus
  // brings the server's (ChatThread#stageMessage).
  function stage(pane, cooked, params) {
    var container = list(pane);
    var tpl = document.querySelector('template.chat-staged-template[data-variant="info"]');
    if (!container || !tpl) {
      return null;
    }
    var node = tpl.content.querySelector(".chat-message-container").cloneNode(true);
    node.classList.remove("-persisted", "-processed", "-last-read");
    node.classList.add("-staged", "-not-processed");
    node.dataset.id = params.staged_id;
    node.dataset.createdAt = new Date().toISOString();
    node.querySelector(".chat-cooked").innerHTML = cooked;
    container.appendChild(node);
    redraw(pane);
    pane.querySelector(".chat-messages-scroller").scrollTop = 0;
    return node;
  }

  function escaped(message) {
    var p = document.createElement("p");
    p.textContent = message;
    return p.outerHTML;
  }

  function send(pane, p) {
    if (p.input.disabled || p.send.disabled || p.composer.classList.contains("is-sending")) {
      return;
    }
    clearTimeout(draftTimer);
    var message = p.input.value;
    var max = Number(p.wrapper.dataset.maxLength || 0);
    if (max && message.length > max) {
      return;
    }
    var params = {
      message: message,
      thread_id: pane.dataset.id,
      staged_id: window.crypto && window.crypto.randomUUID
        ? window.crypto.randomUUID()
        : "staged-" + Date.now() + "-" + Math.floor(Math.random() * 1e6),
      client_created_at: new Date().toISOString(),
    };
    p.composer.classList.add("is-sending");
    var cook = Discourse.chat && Discourse.chat.cook
      ? Discourse.chat.cook(message)
      : Promise.resolve(escaped(message));
    cook
      .then(function (cooked) {
        var staged = stage(pane, cooked, params);
        p.input.value = "";
        updateSendState(p);
        refreshHeight(p.input);
        return request("POST", "/chat/" + pane.dataset.channelId, params).then(function (r) {
          if (r.ok) {
            return;
          }
          return r
            .json()
            .catch(function () {
              return {};
            })
            .then(function (body) {
              if (staged) {
                staged.classList.add("-errored");
                var error = document.createElement("div");
                error.className = "chat-message-error";
                error.textContent = (body.errors || [r.statusText])[0];
                staged.querySelector(".chat-message-content").appendChild(error);
              }
            });
        });
      })
      .finally(function () {
        p.composer.classList.remove("is-sending");
        updateSendState(p);
        p.input.focus();
      });
  }

  document.addEventListener("input", function (event) {
    if (event.target.id !== "thread-composer") {
      return;
    }
    var pane = thread();
    var p = pane && parts(pane);
    if (p) {
      updateSendState(p);
      saveDraft(pane, p);
      refreshHeight(p.input);
    }
  });

  document.addEventListener("keydown", function (event) {
    if (
      event.target.id !== "thread-composer" ||
      event.isComposing ||
      event.altKey ||
      (Discourse.autocomplete && Discourse.autocomplete.isOpen())
    ) {
      return;
    }
    var pane = thread();
    var p = pane && parts(pane);
    if (!p) {
      return;
    }
    if (event.key === "Escape" && !event.shiftKey) {
      // handleEscape: the pane closes, back to the channel.
      event.stopPropagation();
      var close = document.querySelector(".c-navbar__close-thread-button");
      if (close) {
        window.location.assign(close.href);
      }
      return;
    }
    if (event.key === "Enter") {
      var metaEnter = p.wrapper.dataset.sendShortcut === "meta_enter";
      var sends = (!metaEnter && !event.shiftKey) || event.ctrlKey || event.metaKey;
      if (sends) {
        event.preventDefault();
        send(pane, p);
      }
    }
  });

  document.addEventListener("click", function (event) {
    var button = event.target.closest(".chat-thread .chat-composer-button.-send");
    if (button) {
      var pane = thread();
      var p = pane && parts(pane);
      if (p) {
        send(pane, p);
      }
      return;
    }
    // ChatThreadListItem#openThread
    var item = event.target.closest(".chat-thread-list-item__open-button[data-href]");
    if (item) {
      window.location.assign(item.dataset.href);
    }
  });

  var cleanup = null;

  function mount() {
    var pane = thread();
    if (!pane) {
      return;
    }
    // The near-message routes replace themselves with the thread's
    // (ChatChannelThreadNearMessage, ChatChannelNearMessageWithThread).
    var close = document.querySelector(".c-navbar__close-thread-button");
    var threadPath = location.pathname.replace(/\/t\/(\d+)\/\d+$/, "/t/$1");
    var channelHere = location.pathname.match(/^(.*\/chat\/c\/[^/]+\/\d+)\/\d+\/t\/(\d+)$/);
    if (channelHere) {
      threadPath = channelHere[1] + "/t/" + channelHere[2];
    }
    if (threadPath !== location.pathname && (close || !channelHere)) {
      history.replaceState(history.state, "", threadPath + location.search);
    }
    redraw(pane);
    startLive(pane);
    var timer = null;
    var scheduleRead = function () {
      clearTimeout(timer);
      timer = setTimeout(function () {
        markRead(pane);
      }, 1000);
    };
    var scroller = pane.querySelector(".chat-messages-scroller");
    if (scroller) {
      scroller.addEventListener("scroll", scheduleRead, { passive: true });
    }
    document.addEventListener("visibilitychange", scheduleRead);
    scheduleRead();
    var p = parts(pane);
    if (p) {
      updateSendState(p);
      if (!p.input.disabled && !document.documentElement.classList.contains("mobile-view")) {
        p.input.focus({ preventScroll: true });
      }
    }
    cleanup = function () {
      clearTimeout(timer);
      clearTimeout(draftTimer);
      document.removeEventListener("visibilitychange", scheduleRead);
      if (scroller) {
        scroller.removeEventListener("scroll", scheduleRead);
      }
    };
  }

  function unmount() {
    if (cleanup) {
      cleanup();
      cleanup = null;
    }
    if (live) {
      live.close();
      live = null;
    }
  }

  Discourse.onPage(function () {
    mount();
    return unmount;
  });
})();
