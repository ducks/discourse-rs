// Full-page chat and a channel in the drawer (chat-drawer.js) in the
// browser: what Ember's chat components compute from the browser's clock
// and zone, drawn over the server's markup.
//
//   - each message's time (formatChatDate: moment in the local zone), and
//     its full date as the title
//   - the date separators before each day's first message (Today,
//     Yesterday, else the long date), joined with the "last visit" line
//     on the first unread message (ChatMessageSeparator), and their sticky
//     spans (DatesSeparatorsPositioner)
//   - links in messages opening in a new tab (forceLinksToOpenNewTab)
//   - the browser's joined filter over the loaded cards
(function () {
  "use strict";

  if (!Discourse.once("chat")) {
    return;
  }

  var lang = document.documentElement.lang || "en";

  function pad(n) {
    return n < 10 ? "0" + n : String(n);
  }

  // The moment tokens Discourse's English date formats use.
  function format(date, pattern) {
    var hours = date.getHours();
    var tokens = {
      YYYY: String(date.getFullYear()),
      MMMM: date.toLocaleString(lang, { month: "long" }),
      MMM: date.toLocaleString(lang, { month: "short" }),
      D: String(date.getDate()),
      h: String(hours % 12 || 12),
      mm: pad(date.getMinutes()),
      a: hours < 12 ? "am" : "pm",
    };
    return pattern.replace(/YYYY|MMMM|MMM|D|h|mm|a/g, function (token) {
      return tokens[token];
    });
  }

  function sameDay(a, b) {
    return (
      a.getFullYear() === b.getFullYear() &&
      a.getMonth() === b.getMonth() &&
      a.getDate() === b.getDate()
    );
  }

  // moment().calendar with chat's sameDay, lastDay and LL.
  function calendar(channel, date) {
    var today = new Date();
    var yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
    if (sameDay(date, today)) {
      return channel.dataset.labelToday;
    }
    if (sameDay(date, yesterday)) {
      return channel.dataset.labelYesterday;
    }
    return format(date, channel.dataset.formatDate);
  }

  function el(tag, className, text) {
    var node = document.createElement(tag);
    if (className) {
      node.className = className;
    }
    if (text) {
      node.textContent = text;
    }
    return node;
  }

  function separatorLine() {
    var container = el("div", "chat-message-separator__line-container");
    container.appendChild(el("div", "chat-message-separator__line"));
    return container;
  }

  function lastVisitSpan(channel) {
    var span = el("span", "chat-message-separator__last-visit");
    span.appendChild(el("span", "chat-message-separator__last-visit-separator", "-"));
    span.appendChild(document.createTextNode(channel.dataset.labelLastVisit));
    return span;
  }

  function drawSeparators(channel) {
    // Drawn again from scratch when the messages change.
    channel
      .querySelectorAll(
        ".chat-messages-container > .chat-message-separator, .chat-messages-container > .chat-message-separator__line-container"
      )
      .forEach(function (node) {
        node.remove();
      });
    var previous = null;
    channel.querySelectorAll(".chat-message-container[data-created-at]").forEach(function (message) {
      var at = new Date(message.dataset.createdAt);
      var newest = message.hasAttribute("data-newest");
      var firstOfDay = !previous || !sameDay(previous, at);
      previous = at;
      if (!firstOfDay && !newest) {
        return;
      }
      var separator;
      if (firstOfDay) {
        separator = el("div", "chat-message-separator chat-message-separator-date" + (newest ? " with-last-visit" : ""));
        separator.setAttribute("role", "button");
        var text = el("span", "chat-message-separator__text", calendar(channel, at));
        if (newest) {
          text.appendChild(lastVisitSpan(channel));
        }
        var textContainer = el("div", "chat-message-separator__text-container");
        textContainer.appendChild(text);
        separator.appendChild(textContainer);
      } else {
        separator = el("div", "chat-message-separator chat-message-separator-new");
        var newText = el("div", "chat-message-separator__text-container");
        newText.appendChild(el("span", "chat-message-separator__text", channel.dataset.labelLastVisit));
        separator.appendChild(newText);
        separator.appendChild(separatorLine());
      }
      separator.dataset.id = message.dataset.id;
      message.before(separator);
      if (firstOfDay) {
        message.before(separatorLine());
      }
    });
  }

  // DatesSeparatorsPositioner.apply
  function positionSeparators(channel) {
    var container = channel.querySelector(".chat-messages-container");
    if (!container) {
      return;
    }
    var dates = Array.prototype.slice.call(container.querySelectorAll(".chat-message-separator-date")).reverse();
    var height = container.clientHeight;
    dates.forEach(function (date, index) {
      var line = date.nextElementSibling;
      var bottom = 0;
      var spanned;
      if (index > 0) {
        bottom = height - dates[index - 1].nextElementSibling.offsetTop;
      }
      if (dates.length === 1) {
        spanned = height;
      } else if (index === 0) {
        spanned = height - line.offsetTop;
      } else {
        spanned = height - line.offsetTop - (height - dates[index - 1].nextElementSibling.offsetTop);
      }
      date.style.bottom = bottom + "px";
      date.style.height = spanned + "px";
    });
  }

  function drawTimes(channel) {
    channel.querySelectorAll("a.chat-time[data-mode]").forEach(function (link) {
      var message = link.closest("[data-created-at]");
      if (!message) {
        return;
      }
      var at = new Date(message.dataset.createdAt);
      link.title = format(at, channel.dataset.formatTitle);
      link.textContent = format(at, link.dataset.mode === "tiny" ? channel.dataset.formatTiny : channel.dataset.formatTime);
    });
  }

  function decorateLinks(channel) {
    channel.querySelectorAll(".chat-cooked a:not([target]), .chat-cooked a[target]:not([target='_blank'])").forEach(function (link) {
      if (link.origin !== location.origin) {
        link.setAttribute("target", "_blank");
      }
    });
    // decorateChatMessage's highlightSyntax, on the same messages.
    if (window.Discourse && Discourse.highlightSyntax) {
      channel.querySelectorAll(".chat-cooked").forEach(function (cooked) {
        Discourse.highlightSyntax(cooked);
      });
    }
    // chat-plugin-decorators' local dates.
    if (window.Discourse && Discourse.applyLocalDates) {
      channel.querySelectorAll(".chat-cooked").forEach(function (cooked) {
        Discourse.applyLocalDates(cooked);
      });
    }
    // discourse-math's decorateChatMessage.
    if (window.Discourse && Discourse.renderMath) {
      channel.querySelectorAll(".chat-cooked").forEach(function (cooked) {
        Discourse.renderMath(cooked);
      });
    }
  }

  function filterCards(select) {
    var value = select.value;
    document.querySelectorAll(".chat-browse-view__cards .chat-channel-card").forEach(function (card) {
      var following = card.dataset.following === "true";
      card.hidden = (value === "joined" && !following) || (value === "not-joined" && following);
    });
    select.querySelectorAll("option").forEach(function (option) {
      option.classList.toggle("--selected", option.value === value);
    });
    select.setAttribute("aria-label", select.options[select.selectedIndex].text.trim());
  }

  document.addEventListener("change", function (event) {
    if (event.target.matches(".chat-browse-view .d-filter-controls__dropdown")) {
      filterCards(event.target);
    }
  });

  // New cards (a filter, the next page) take the current joined filter.
  document.addEventListener("htmx:afterSettle", function () {
    var select = document.querySelector(".chat-browse-view .d-filter-controls__dropdown");
    if (select) {
      filterCards(select);
    }
  });

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

  // A chat API request; errors as popupAjaxError shows them.
  function api(method, path, params) {
    return fetch(basePath() + "/chat/api" + path, {
      method: method,
      credentials: "same-origin",
      headers: csrfHeaders(),
      body: params ? new URLSearchParams(params).toString() : undefined,
    }).then(function (r) {
      if (r.ok) {
        return r.json();
      }
      return r
        .json()
        .catch(function () {
          return {};
        })
        .then(function (body) {
          window.alert((body.errors || [r.statusText]).join("\n"));
          throw new Error(r.statusText);
        });
    });
  }

  // The parts of the page a change redraws (the chat page, chat's sidebar
  // sections), from the page as the server now draws it: what Ember's
  // tracked state redraws in place. The composer stays as it is: its
  // text, reply and edit live in the browser, as Ember's draft does.
  // The chat page (or the drawer's route) swapped for its new drawing,
  // the composer kept.
  function swapPage(page, nextPage) {
    var composer = page.querySelector(".chat-composer__wrapper");
    var nextComposer = nextPage.querySelector(".chat-composer__wrapper");
    var focused = composer && composer.contains(document.activeElement);
    teardown();
    if (composer && nextComposer) {
      nextComposer.replaceWith(composer);
    }
    page.replaceWith(nextPage);
    if (window.htmx && nextPage.closest(".chat-drawer")) {
      window.htmx.process(nextPage);
    }
    drawChannelDates();
    setup(true);
    if (focused) {
      var input = document.getElementById("channel-composer");
      if (input) {
        input.focus({ preventScroll: true });
      }
    }
  }

  function refreshPage() {
    return fetch(location.href, { credentials: "same-origin" })
      .then(function (r) {
        return r.text();
      })
      .then(function (html) {
        var next = new DOMParser().parseFromString(html, "text/html");
        var page = document.querySelector(".full-page-chat");
        var nextPage = next.querySelector(".full-page-chat");
        if (page && nextPage) {
          swapPage(page, nextPage);
        }
        refreshSidebar(next);
      });
  }

  // In the drawer (chat-drawer.js): its route drawn again, then the
  // page's sidebar.
  function refresh() {
    var route = document.querySelector(".chat-drawer .c-drawer-routes");
    if (document.querySelector(".full-page-chat") || !route || !Discourse.chatDrawer) {
      return refreshPage();
    }
    return Discourse.chatDrawer.fetchRoute().then(function (html) {
      var current = document.querySelector(".chat-drawer .c-drawer-routes");
      if (html && current) {
        var template = document.createElement("template");
        template.innerHTML = html;
        var next = template.content.querySelector(".c-drawer-routes");
        if (next) {
          swapPage(current, next);
        }
      }
      return fetch(location.href, { credentials: "same-origin" })
        .then(function (r) {
          return r.text();
        })
        .then(function (page) {
          refreshSidebar(new DOMParser().parseFromString(page, "text/html"));
        });
    });
  }

  function refreshSidebar(next) {
    var sections = document.querySelector(".sidebar-sections");
    if (!sections) {
      return;
    }
    var collapsed = {};
    var old = sections.querySelectorAll(".sidebar-section[data-section-name^='chat-']");
    old.forEach(function (section) {
      collapsed[section.dataset.sectionName] = section.classList.contains("sidebar-section--collapsed");
    });
    var anchor = old.length ? old[0].previousElementSibling : sections.lastElementChild;
    old.forEach(function (section) {
      section.remove();
    });
    next.querySelectorAll(".sidebar-sections > .sidebar-section[data-section-name^='chat-']").forEach(function (section) {
      if (anchor) {
        anchor.after(section);
      } else {
        sections.prepend(section);
      }
      anchor = section;
      if (collapsed[section.dataset.sectionName] && Discourse.setSidebarSectionExpanded) {
        Discourse.setSidebarSectionExpanded(section, false);
      }
    });
    if (Discourse.chatDrawer) {
      Discourse.chatDrawer.markSidebar();
    }
  }

  function cardChannelId(button) {
    var card = button.closest("[data-channel-id]");
    if (card) {
      return card.dataset.channelId;
    }
    var channel = document.querySelector(".chat-channel[data-id]");
    return channel ? channel.dataset.id : null;
  }

  // In the capture phase, so the link around a DM list row's leave button
  // (and htmx's boost on it) never sees its click.
  document.addEventListener("click", function (event) {
    var button = event.target.closest(
      ".toggle-channel-membership-button, .c-navbar__star-channel-button"
    );
    if (!button || button.disabled) {
      return;
    }
    // A DM list row's leave button: the row is the link around it.
    var row = button.closest(".chat-channel-row[data-chat-channel-id]");
    if (row) {
      event.preventDefault();
      event.stopPropagation();
    }
    var id = row ? row.dataset.chatChannelId : cardChannelId(button);
    if (!id) {
      return;
    }
    var request;
    if (button.classList.contains("c-navbar__star-channel-button")) {
      // toggleStarred
      request = api("PUT", "/channels/" + id + "/memberships/me", {
        starred: !button.classList.contains("--starred"),
      });
    } else if (button.classList.contains("-join")) {
      // followChannel
      request = api("POST", "/channels/" + id + "/memberships/me");
    } else {
      // unfollowChannel
      request = api("DELETE", "/channels/" + id + "/memberships/me/follows");
    }
    button.disabled = true;
    request
      .then(function () {
        if (row) {
          row.remove();
        }
        return refresh();
      })
      .catch(function () {
        button.disabled = false;
      });
  }, true);

  // updateLastReadMessage: the last message whose bottom shows, once the
  // pane has settled, marks the channel read up to it.
  function lastVisibleMessage(channel) {
    var scroller = channel.querySelector(".chat-messages-scroller");
    if (!scroller) {
      return null;
    }
    var view = scroller.getBoundingClientRect();
    var found = null;
    // A staged message's data-id is its staged_id, which messagesManager
    // never finds: only persisted messages mark the channel read.
    channel.querySelectorAll(".chat-message-container[data-id]:not(.-staged)").forEach(function (message) {
      var rect = message.getBoundingClientRect();
      if (rect.bottom <= view.bottom + 1 && rect.bottom >= view.top) {
        found = message;
      }
    });
    return found;
  }

  function markRead(channel) {
    if (channel.dataset.following !== "true" || document.visibilityState !== "visible") {
      return;
    }
    var message = lastVisibleMessage(channel);
    if (!message) {
      return;
    }
    var id = Number(message.dataset.id);
    if (!Number.isInteger(id) || id <= Number(channel.dataset.lastRead || 0)) {
      return;
    }
    channel.dataset.lastRead = String(id);
    // The membership's lastReadMessageId moves, and its -last-read class.
    channel.querySelectorAll(".chat-message-container.-last-read").forEach(function (other) {
      other.classList.remove("-last-read");
    });
    message.classList.add("-last-read");
    api("PUT", "/channels/" + channel.dataset.id + "/read?message_id=" + id).then(function () {
      // The sidebar's unread dot goes once the last message is read.
      return fetch(location.href, { credentials: "same-origin" })
        .then(function (r) {
          return r.text();
        })
        .then(function (html) {
          refreshSidebar(new DOMParser().parseFromString(html, "text/html"));
        });
    });
  }

  // ChatComposer: the send button follows hasContent (or an edit), the
  // draft is saved two seconds after typing stops (persistDraft, the
  // draft's JSON as toJSONDraft makes it), and the send shortcut (Enter,
  // or Ctrl/Cmd+Enter with meta_enter) or the button sends, edits (an
  // emptied edit deletes the message) or replies, per the message details
  // bar (ChatComposerMessageDetails).
  function composerParts() {
    var wrapper = document.querySelector(".chat-composer__wrapper");
    if (!wrapper) {
      return null;
    }
    return {
      wrapper: wrapper,
      composer: wrapper.querySelector(".chat-composer"),
      input: wrapper.querySelector("#channel-composer"),
      send: wrapper.querySelector(".chat-composer-button.-send"),
      channel: document.querySelector(".chat-channel[data-id]"),
      details: wrapper.querySelector(".chat-composer-message-details"),
    };
  }

  function editing(parts) {
    return parts.details && parts.details.dataset.action === "edit" ? parts.details : null;
  }

  function replying(parts) {
    return parts.details && parts.details.dataset.action === "reply" ? parts.details : null;
  }

  function hasContent(parts) {
    var min = Number(parts.wrapper.dataset.minLength || 1) || 1;
    return parts.input.value.length >= min;
  }

  function updateSendState(parts) {
    var enabled = (hasContent(parts) || !!editing(parts)) && !parts.input.disabled;
    parts.composer.classList.toggle("is-send-enabled", enabled);
    parts.composer.classList.toggle("is-send-disabled", !enabled);
    parts.send.disabled = !enabled;
    parts.send.tabIndex = enabled ? 0 : -1;
  }

  // ChatMessage#toJSONDraft
  function draftJSON(parts) {
    var message = parts.input.value;
    var reply = replying(parts);
    var edit = editing(parts);
    if (message.length === 0 && !reply) {
      return null;
    }
    var data = {};
    if (message.length > 0) {
      data.message = message;
    }
    if (reply) {
      data.replyToMsg = {
        id: Number(reply.dataset.id),
        excerpt: reply.dataset.excerpt,
        user: JSON.parse(reply.dataset.user || "{}"),
      };
    }
    if (edit) {
      data.editing = true;
      data.id = Number(edit.dataset.id);
      data.excerpt = edit.dataset.excerpt;
    }
    return JSON.stringify(data);
  }

  var draftTimer = null;

  function saveDraft(parts) {
    clearTimeout(draftTimer);
    parts.composer.classList.remove("is-draft-saved");
    parts.composer.classList.add("is-draft-unsaved");
    var channelId = parts.channel.dataset.id;
    var data = draftJSON(parts);
    draftTimer = setTimeout(function () {
      api("POST", "/channels/" + channelId + "/drafts", { data: data === null ? "" : data })
        .then(function () {
          parts.composer.classList.remove("is-draft-unsaved");
          parts.composer.classList.add("is-draft-saved");
        })
        .catch(function () {});
    }, 2000);
  }

  // The details bar for a message (ChatComposerMessageDetails), from the
  // toolbar template's prototype and the message's data.
  function setDetails(parts, action, message) {
    if (parts.details) {
      parts.details.remove();
      parts.details = null;
    }
    if (!action) {
      return;
    }
    var tpl = actionsTemplate();
    if (!tpl) {
      return;
    }
    var bar = tpl.content.querySelector(".chat-composer-message-details").cloneNode(true);
    var reply = bar.querySelector(".chat-reply");
    var icon = bar.querySelector("template[data-action='" + action + "']");
    bar.querySelectorAll("template").forEach(function (t) {
      t.remove();
    });
    reply.insertBefore(icon.content.cloneNode(true), reply.firstChild);
    var user = messageUser(message);
    bar.dataset.action = action;
    bar.dataset.id = message.dataset.id;
    bar.dataset.excerpt = message.dataset.excerpt || "";
    bar.dataset.user = JSON.stringify(user);
    reply.appendChild(chatUserAvatar(user, 24));
    reply.appendChild(el("span", "chat-reply__username", user.username));
    var excerpt = el("span", "chat-reply__excerpt");
    excerpt.innerHTML = message.dataset.excerpt || "";
    reply.appendChild(excerpt);
    parts.wrapper.insertBefore(bar, parts.wrapper.firstChild);
    parts.details = bar;
  }

  function messageUser(message) {
    var user = {
      id: Number(message.dataset.userId),
      avatar_template: message.dataset.avatarTemplate,
      username: message.dataset.username,
    };
    if (message.dataset.name !== undefined) {
      user = {
        id: user.id,
        name: message.dataset.name,
        avatar_template: user.avatar_template,
        username: user.username,
      };
    }
    return user;
  }

  // ChatUserAvatar, marked online when its user is in /chat/online.
  function chatUserAvatar(user, size) {
    var wrapper = el("div", "chat-user-avatar");
    if (isOnline(user.username)) {
      wrapper.classList.add("is-online");
    }
    wrapper.dataset.username = user.username;
    var link = el("a", "chat-user-avatar__container");
    link.dataset.userCard = user.username;
    link.href = basePath() + "/u/" + user.username;
    var img = document.createElement("img");
    img.alt = "";
    img.width = size;
    img.height = size;
    img.src = (user.avatar_template || "").replace("{size}", String(size));
    img.className = "avatar";
    img.title = user.username;
    link.appendChild(img);
    wrapper.appendChild(link);
    return wrapper;
  }

  // TextareaInteractor#refreshHeight: the composer grows with its text.
  function refreshHeight(input) {
    input.style.height = "auto";
    input.style.height = input.scrollHeight + 1 + "px";
  }

  // isFocused
  document.addEventListener("focusin", function (event) {
    if (event.target.id === "channel-composer") {
      event.target.closest(".chat-composer").classList.add("is-focused");
    }
  });

  document.addEventListener("focusout", function (event) {
    if (event.target.id === "channel-composer") {
      event.target.closest(".chat-composer").classList.remove("is-focused");
    }
  });

  function focusComposer(parts) {
    parts.input.focus();
    var end = parts.input.value.length;
    parts.input.setSelectionRange(end, end);
    refreshHeight(parts.input);
  }

  // ChatChannelComposer#edit and #replyTo
  function startEdit(message) {
    var parts = composerParts();
    if (!parts) {
      return;
    }
    setDetails(parts, "edit", message);
    parts.input.value = message.dataset.message || "";
    updateSendState(parts);
    focusComposer(parts);
  }

  function startReply(message) {
    var parts = composerParts();
    if (!parts) {
      return;
    }
    setDetails(parts, "reply", message);
    updateSendState(parts);
    focusComposer(parts);
  }

  // resetDraft: the composer emptied, its details gone.
  function resetComposer(parts) {
    parts.input.value = "";
    setDetails(parts, null);
    updateSendState(parts);
    refreshHeight(parts.input);
  }

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

  function sendMessage(parts) {
    if (parts.input.disabled || parts.send.disabled || parts.composer.classList.contains("is-sending")) {
      return;
    }
    clearTimeout(draftTimer);
    var message = parts.input.value;
    var channelId = parts.channel.dataset.id;
    var edit = editing(parts);
    if (edit && message.length === 0) {
      // #deleteEmptyMessage
      var id = edit.dataset.id;
      resetComposer(parts);
      api("DELETE", "/channels/" + channelId + "/messages/" + id).then(changed).catch(function () {});
      return;
    }
    // reactingToLastMessage: "+emoji" (or "+:code:") reacts to the last
    // message instead.
    var trimmed = message.trim();
    if (trimmed.charAt(0) === "+") {
      parts.composer.classList.add("is-sending");
      Discourse.emojiPicker
        .shortcutReaction(trimmed)
        .catch(function () {
          return null;
        })
        .then(function (code) {
          parts.composer.classList.remove("is-sending");
          var last = lastMessage();
          if (code && last) {
            resetComposer(parts);
            react(last, code, "add");
          } else {
            deliver(parts, message, channelId, edit);
          }
        });
      return;
    }
    deliver(parts, message, channelId, edit);
  }

  // onSendMessage: the message sent, or the edit saved.
  function deliver(parts, message, channelId, edit) {
    var max = Number(parts.wrapper.dataset.maxLength || 0);
    if (max && message.length > max) {
      var tpl = actionsTemplate();
      window.alert((tpl ? tpl.dataset.labelTooLong : "").replace("%{count}", String(max)));
      return;
    }
    var reply = replying(parts);
    parts.composer.classList.add("is-sending");
    var done = function () {
      var now = composerParts();
      if (now) {
        now.composer.classList.remove("is-sending");
        updateSendState(now);
        now.input.focus();
      }
    };
    if (edit) {
      // #sendEditMessage: the composer reset, the edit saved; a failed one
      // says why (popupAjaxError).
      resetComposer(parts);
      fetch(basePath() + "/chat/api/channels/" + channelId + "/messages/" + edit.dataset.id, {
        method: "PUT",
        credentials: "same-origin",
        headers: csrfHeaders(),
        body: new URLSearchParams({ message: message }).toString(),
      })
        .then(function (r) {
          return r.ok ? changed() : failed(r);
        })
        .finally(done);
      return;
    }
    // #sendNewMessage: cooked in the browser and staged, the composer
    // reset, then sent; the bus swaps the server's message in.
    var params = {
      message: message,
      // guid()
      staged_id: window.crypto && window.crypto.randomUUID
        ? window.crypto.randomUUID()
        : "staged-" + Date.now() + "-" + Math.floor(Math.random() * 1e6),
      client_created_at: new Date().toISOString(),
    };
    if (reply) {
      params.in_reply_to_id = reply.dataset.id;
    }
    cookStaged(message)
      .then(function (cooked) {
        var staged = stage(cooked, params, reply);
        resetComposer(parts);
        return sendStaged(channelId, params, staged);
      })
      .finally(done);
  }

  // ChatMessage#cook in the browser: the markdown crate's chat rules, as
  // the composer's preview loads it; the text escaped when it can't.
  var chatRenderer = null;

  function cookStaged(message) {
    var tpl = document.querySelector("template.chat-staged-template[data-wasm]");
    var escaped = function () {
      var p = document.createElement("p");
      p.textContent = message;
      return p.outerHTML;
    };
    if (!tpl || !window.WebAssembly) {
      return Promise.resolve(escaped());
    }
    if (!chatRenderer) {
      chatRenderer = Promise.all([
        WebAssembly.instantiateStreaming(fetch(tpl.dataset.wasm), {}),
        fetch(tpl.dataset.settings, { credentials: "same-origin" }).then(function (r) {
          if (!r.ok) {
            throw new Error("settings " + r.status);
          }
          return r.text();
        }),
      ]).then(function (loaded) {
        var wasm = loaded[0].instance.exports;
        var call = function (fn, text) {
          var bytes = new TextEncoder().encode(text);
          var ptr = wasm.alloc(bytes.length);
          new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
          var status = fn(ptr, bytes.length);
          wasm.dealloc(ptr, bytes.length);
          var out = new TextDecoder().decode(
            new Uint8Array(wasm.memory.buffer, wasm.output_ptr(), wasm.output_len())
          );
          if (status !== 0) {
            throw new Error(out);
          }
          return out;
        };
        call(wasm.configure, loaded[1]);
        return function (raw) {
          return call(wasm.preview, raw);
        };
      });
    }
    return chatRenderer
      .then(function (cook) {
        return cook(message);
      })
      .catch(function () {
        return escaped();
      });
  }

  // ChatChannel#stageMessage: the member's message at the end of the list
  // (-staged, -not-processed), its author line hidden after their own
  // recent message (hideUserInfo).
  function stage(cooked, params, reply) {
    var channel = document.querySelector(".chat-channel[data-id]");
    var list = channel && channel.querySelector(".chat-messages-container");
    if (!list) {
      return null;
    }
    if (channel.classList.contains("is-empty")) {
      // The empty state gives way to the first message.
      channel.classList.remove("is-empty");
      list.innerHTML = "";
    }
    var items = listItems(list);
    var previous = items[items.length - 1] || null;
    var now = new Date();
    var hidden =
      !!previous &&
      !previous.matches("template") &&
      !previous.classList.contains("-deleted") &&
      previous.dataset.userId === channel.dataset.viewerId &&
      Math.abs(now - new Date(previous.dataset.createdAt)) <= 300000 &&
      (!reply || reply.dataset.id === previous.dataset.id);
    var tpl = document.querySelector(
      'template.chat-staged-template[data-variant="' + (hidden ? "hidden" : "info") + '"]'
    );
    if (!tpl) {
      return null;
    }
    var node = tpl.content.querySelector(".chat-message-container").cloneNode(true);
    node.classList.remove("-persisted", "-processed", "-last-read");
    node.classList.add("-staged", "-not-processed");
    node.dataset.id = params.staged_id;
    node.dataset.createdAt = now.toISOString();
    node.dataset.message = params.message;
    node.dataset.excerpt = params.message;
    node.querySelector(".chat-cooked").innerHTML = cooked;
    var bottom = atBottom(channel);
    list.appendChild(node);
    redraw(channel);
    // Its time, not yet a link to it.
    node.querySelectorAll("a.chat-time").forEach(function (link) {
      var span = document.createElement("span");
      span.setAttribute("title", link.title);
      span.setAttribute("tabindex", "-1");
      span.className = "chat-time";
      span.textContent = link.textContent;
      link.replaceWith(span);
    });
    if (bottom) {
      channel.querySelector(".chat-messages-scroller").scrollTop = 0;
    }
    return node;
  }

  // The send, and setSendError on the staged message when it fails: the
  // server's reason, or a network error with "Send again?".
  function sendStaged(channelId, params, staged) {
    if (staged) {
      staged.classList.remove("-errored");
      var old = staged.querySelector(".chat-message-error");
      if (old) {
        old.remove();
      }
    }
    return fetch(basePath() + "/chat/" + channelId, {
      method: "POST",
      credentials: "same-origin",
      headers: csrfHeaders(),
      body: new URLSearchParams(params).toString(),
    }).then(
      function (r) {
        if (r.ok) {
          return changed();
        }
        if (!staged) {
          return failed(r);
        }
        return r
          .json()
          .catch(function () {
            return {};
          })
          .then(function (body) {
            sendError(staged, channelId, params, (body.errors || [])[0]);
          });
      },
      function () {
        if (staged) {
          sendError(staged, channelId, params, null);
        }
      }
    );
  }

  // ChatMessageError
  function sendError(staged, channelId, params, reason) {
    var tpl = document.querySelector("template.chat-staged-template[data-wasm]");
    staged.classList.add("-errored");
    var error = el("div", "chat-message-error");
    if (reason) {
      error.textContent = reason;
    } else {
      var button = el("button", "btn no-text btn-icon chat-message-error__retry-btn");
      button.type = "button";
      button.innerHTML = tpl.content.querySelector("template.chat-retry-icon").innerHTML;
      button.appendChild(el("span", "chat-message-error__retry-btn-title", tpl.dataset.labelNetworkError));
      button.appendChild(el("span", "chat-message-error__retry-btn-action", tpl.dataset.labelSendAgain));
      button.addEventListener("click", function () {
        // resendStagedMessage
        sendStaged(channelId, params, staged);
      });
      error.appendChild(button);
    }
    staged.querySelector(".chat-message-content").appendChild(error);
  }

  document.addEventListener("input", function (event) {
    if (event.target.id !== "channel-composer") {
      return;
    }
    var parts = composerParts();
    if (parts) {
      updateSendState(parts);
      saveDraft(parts);
      refreshHeight(parts.input);
      reportReplying(parts);
    }
  });

  document.addEventListener("keydown", function (event) {
    // An open autocomplete has the keys (#isAutocompleteDisplayed).
    if (
      event.target.id !== "channel-composer" ||
      event.isComposing ||
      event.altKey ||
      (Discourse.autocomplete && Discourse.autocomplete.isOpen())
    ) {
      return;
    }
    var parts = composerParts();
    if (!parts) {
      return;
    }
    if (event.key === "Escape" && !event.shiftKey) {
      // handleEscape: a reply is dropped, else an edit, else the composer
      // loses focus.
      event.stopPropagation();
      if (replying(parts)) {
        setDetails(parts, null);
        updateSendState(parts);
      } else if (editing(parts)) {
        resetComposer(parts);
      } else {
        parts.input.blur();
      }
      return;
    }
    if (event.key === "Enter") {
      var metaEnter = parts.wrapper.dataset.sendShortcut === "meta_enter";
      var sends = (!metaEnter && !event.shiftKey) || event.ctrlKey || event.metaKey;
      if (sends) {
        event.preventDefault();
        sendMessage(parts);
      }
      return;
    }
    if (event.key === "ArrowUp" && !hasContent(parts) && !editing(parts)) {
      // Shift+Up replies to the last message, Up edits the member's last.
      if (event.shiftKey) {
        var last = lastMessage();
        if (last && canReply(last)) {
          event.preventDefault();
          startReply(last);
        }
      } else {
        var own = lastOwnMessage();
        if (own && canEdit(own)) {
          event.preventDefault();
          startEdit(own);
        }
      }
    }
  });

  document.addEventListener("click", function (event) {
    if (event.target.closest(".chat-composer-button.-send")) {
      var parts = composerParts();
      if (parts) {
        sendMessage(parts);
      }
      return;
    }
    if (event.target.closest(".chat-composer-message-details .cancel-message-action")) {
      var now = composerParts();
      if (now) {
        resetComposer(now);
        now.input.focus();
      }
    }
  });

  // ChatMessageInteractor and ChatMessageActionsDesktop: the toolbar of
  // the message under the pointer, from the page's template.
  function actionsTemplate() {
    return document.querySelector("template.chat-message-actions-template");
  }

  function messages() {
    return Array.prototype.slice.call(
      document.querySelectorAll(".chat-messages-container > .chat-message-container[data-id]")
    );
  }

  function lastMessage() {
    var all = messages().filter(function (m) {
      return !m.classList.contains("-deleted");
    });
    return all[all.length - 1] || null;
  }

  function lastOwnMessage() {
    var tpl = actionsTemplate();
    var all = messages().filter(function (m) {
      return tpl && m.dataset.userId === tpl.dataset.viewerId && !m.classList.contains("-deleted");
    });
    return all[all.length - 1] || null;
  }

  function flag(tpl, name) {
    return tpl.dataset[name] === "true";
  }

  function isOwn(message) {
    var tpl = actionsTemplate();
    return !!tpl && message.dataset.userId === tpl.dataset.viewerId;
  }

  // canInteractWithMessage
  function canInteract(message) {
    var tpl = actionsTemplate();
    return (
      !!tpl &&
      !message.classList.contains("-deleted") &&
      flag(tpl, "canModify") &&
      flag(tpl, "following")
    );
  }

  function canReply(message) {
    return canInteract(message);
  }

  function canEdit(message) {
    var tpl = actionsTemplate();
    return !!tpl && !message.classList.contains("-deleted") && isOwn(message) && flag(tpl, "canModify");
  }

  // The secondary actions that apply to a message, in their order.
  function secondaryActions(message) {
    var tpl = actionsTemplate();
    var own = isOwn(message);
    var deleted = message.classList.contains("-deleted");
    var canModify = flag(tpl, "canModify");
    var pinned = message.dataset.pinned !== undefined;
    var actions = ["copyLink"];
    if (canEdit(message)) {
      actions.push("edit");
    }
    actions.push("select");
    if (flag(tpl, "canPin") && !pinned) {
      actions.push("pin");
    }
    if (flag(tpl, "canPin") && pinned) {
      actions.push("unpin");
    }
    if (!own && message.dataset.userFlagStatus === undefined && flag(tpl, "canFlag") && !deleted) {
      actions.push("flag");
    }
    if ((own ? flag(tpl, "canDeleteSelf") : flag(tpl, "canDeleteOthers")) && !deleted && canModify) {
      actions.push("delete");
    }
    if (
      deleted &&
      (flag(tpl, "staff") || flag(tpl, "canModerate") || (own && message.dataset.deletedById === tpl.dataset.viewerId)) &&
      canModify
    ) {
      actions.push("restore");
    }
    if (flag(tpl, "staff") && canModify) {
      actions.push("rebake");
    }
    return actions;
  }

  // isSkinTonableEmoji: a tonable emoji not toned yet.
  function tonable(tpl, emoji) {
    return !/:t[1-6]:?$/.test(emoji) && (tpl.dataset.tonable || "").split("|").indexOf(emoji.split(":")[0]) !== -1;
  }

  // quickReactionEmojis: the member's custom ones, then the most used
  // (EmojiStore's, in emoji-picker.js), then the site's defaults in the
  // chosen skin tone; three of them.
  function quickReactions(tpl) {
    var custom = (tpl.dataset.quickCustom || "").split("|").filter(Boolean);
    var frequent = Discourse.emojiStore.favorites("chat", function (emoji) {
      return tonable(tpl, emoji);
    });
    var tone = Discourse.emojiStore.diversity();
    var defaults = (tpl.dataset.quickDefaults || "").split("|").map(function (emoji) {
      return tone !== 1 && tonable(tpl, emoji) ? emoji + ":t" + tone : emoji;
    });
    var all = custom.concat(frequent).concat(defaults);
    return all
      .filter(function (item, index) {
        return all.indexOf(item) === index;
      })
      .filter(Boolean)
      .slice(0, 3);
  }

  // emojiUrlFor: a custom emoji's upload, else the standard image.
  function emojiUrl(tpl, emoji) {
    var custom = JSON.parse(tpl.dataset.customEmoji || "{}");
    var code = emoji.toLowerCase();
    if (custom[code]) {
      return custom[code];
    }
    return tpl.dataset.emojiUrl.replace("%{name}", code.replace(":t", "/"));
  }

  // ChatMessageReaction without its count: the message's own reaction
  // when it has one, else a placeholder.
  function quickReaction(tpl, message, emoji, index) {
    var existing = null;
    message.querySelectorAll(".chat-message-reaction-list .chat-message-reaction").forEach(function (button) {
      if (button.dataset.emojiName === emoji) {
        existing = button;
      }
    });
    var reacted = !!existing && existing.classList.contains("reacted");
    var label = (reacted ? tpl.dataset.labelRemove : tpl.dataset.labelAdd).replace("%{emoji}", emoji);
    var button;
    var nodes = [];
    if (existing) {
      button = existing.cloneNode(true);
      button.removeAttribute("aria-pressed");
      var count = button.querySelector(".count");
      if (count) {
        count.remove();
      }
      var description = existing.nextElementSibling;
      if (description && description.classList.contains("sr-only")) {
        var copy = description.cloneNode(true);
        copy.id = "chat-message-reaction-description-" + index;
        button.setAttribute("aria-describedby", copy.id);
        nodes.push(copy);
      }
    } else {
      button = document.createElement("button");
      button.className = "chat-message-reaction";
      button.dataset.emojiName = emoji;
      button.title = ":" + emoji + ":";
      button.type = "button";
      var img = document.createElement("img");
      img.alt = ":" + emoji + ":";
      img.className = "emoji";
      img.height = 20;
      img.setAttribute("loading", "lazy");
      img.src = emojiUrl(tpl, emoji);
      img.width = 20;
      button.appendChild(img);
    }
    button.setAttribute("aria-label", label);
    nodes.unshift(button);
    return nodes;
  }

  // The bookmark button: BookmarkIcon's state, and its label.
  function bookmarkButton(tpl, message, button) {
    var name = message.dataset.bookmarkName;
    var reminder = message.dataset.bookmarkReminderAt;
    var bookmarked = name !== undefined;
    var state = !bookmarked ? "none" : reminder ? "reminder" : "bookmarked";
    button.querySelectorAll(".svg-icon-title[data-state]").forEach(function (span) {
      if (span.dataset.state === state) {
        span.removeAttribute("data-state");
      } else {
        span.remove();
      }
    });
    var label = tpl.dataset.labelBookmark;
    if (bookmarked) {
      var title;
      if (reminder) {
        title = tpl.dataset.labelCreatedReminder
          .replace("%{date}", reminderTime(tpl, new Date(reminder)))
          .replace("%{name}", name || "");
      } else {
        title = tpl.dataset.labelCreatedGeneric.replace("%{name}", name || "");
      }
      button.querySelector(".svg-icon-title").title = title;
      label = title || tpl.dataset.labelBookmarkEdit;
    }
    button.setAttribute("aria-label", label);
    button.title = label;
  }

  // formattedReminderTime, in the browser's zone.
  function reminderTime(tpl, date) {
    var channel = document.querySelector(".chat-channel[data-id]");
    var time = format(date, channel.dataset.formatTime);
    var now = new Date();
    var tomorrow = new Date(now.getTime());
    tomorrow.setDate(now.getDate() + 1);
    if (sameDay(date, tomorrow)) {
      return tpl.dataset.labelReminderTomorrow.replace("%{time}", time);
    }
    if (sameDay(date, now)) {
      return tpl.dataset.labelReminderToday.replace("%{time}", time);
    }
    return tpl.dataset.labelReminderAt.replace("%{date_time}", format(date, channel.dataset.formatTitle));
  }

  function buildToolbar(message) {
    var tpl = actionsTemplate();
    var container = tpl.content.querySelector(".chat-message-actions-container").cloneNode(true);
    var toolbar = container.querySelector(".chat-message-actions");
    container.dataset.id = message.dataset.id;
    var scroller = message.closest(".chat-messages-scroller");
    var full = !scroller || scroller.clientWidth >= 500;
    container.classList.toggle("is-size-full", full);
    container.classList.toggle("is-size-reduced", !full);
    var interact = canInteract(message);
    var first = toolbar.firstChild;
    if (full && flag(tpl, "following")) {
      quickReactions(tpl).forEach(function (emoji, index) {
        quickReaction(tpl, message, emoji, index).forEach(function (node) {
          toolbar.insertBefore(node, first);
        });
      });
    }
    if (!interact) {
      toolbar.querySelector(".react-btn").remove();
      toolbar.querySelector(".reply-btn").remove();
    }
    var bookmark = toolbar.querySelector(".bookmark-btn");
    if (flag(tpl, "canModify")) {
      bookmarkButton(tpl, message, bookmark);
    } else {
      bookmark.remove();
    }
    var actions = secondaryActions(message);
    var index = 0;
    container.querySelectorAll(".select-kit-row").forEach(function (row) {
      if (actions.indexOf(row.dataset.value) === -1) {
        row.remove();
      } else {
        row.dataset.index = String(index++);
      }
    });
    if (!actions.length) {
      toolbar.classList.add("has-no-secondary-actions");
      container.querySelector(".more-buttons").remove();
    }
    // The roving toolbar: one tab stop, the first control.
    toolbar.querySelectorAll(":scope > button, :scope > details > summary").forEach(function (control, i) {
      control.tabIndex = i === 0 ? 0 : -1;
    });
    return container;
  }

  var hoverTimer = null;

  // The menu or the emoji picker of the active message is open
  // (interactedChatMessage.emojiPickerOpen): it stays active.
  function menuOpen() {
    return (
      !!document.querySelector(".chat-message-actions-container .more-buttons.is-expanded") ||
      (!!pickerMessage && Discourse.emojiPicker.isOpen())
    );
  }

  function clearActive() {
    document.querySelectorAll(".chat-message-actions-container").forEach(function (node) {
      node.remove();
    });
    document.querySelectorAll(".chat-message-container.-active").forEach(function (node) {
      node.classList.remove("-active");
    });
  }

  // _setActiveMessage: an expanded message the member can act on.
  function activate(message) {
    if (!actionsTemplate() || message.classList.contains("-active")) {
      return;
    }
    if (message.querySelector(".chat-message-expand") || message.classList.contains("-staged")) {
      return;
    }
    clearActive();
    message.classList.add("-active");
    message.appendChild(buildToolbar(message));
  }

  document.addEventListener("mouseover", function (event) {
    var message = event.target.closest(".chat-messages-container > .chat-message-container[data-id]");
    if (!message || message.classList.contains("-active") || menuOpen()) {
      return;
    }
    clearTimeout(hoverTimer);
    hoverTimer = setTimeout(function () {
      activate(message);
    }, 250);
  });

  document.addEventListener("mouseout", function (event) {
    var message = event.target.closest(".chat-message-container[data-id]");
    if (!message || message.contains(event.relatedTarget)) {
      return;
    }
    clearTimeout(hoverTimer);
    if (!menuOpen() && message.classList.contains("-active")) {
      clearActive();
    }
  });

  // The secondary actions' dropdown, positioned at its header.
  document.addEventListener(
    "toggle",
    function (event) {
      var details = event.target;
      if (!details.classList || !details.classList.contains("more-actions-chat")) {
        return;
      }
      details.classList.toggle("is-expanded", details.open);
      var body = details.querySelector(".select-kit-body");
      if (!details.open) {
        body.removeAttribute("style");
        return;
      }
      var header = details.querySelector("summary").getBoundingClientRect();
      body.style.position = "fixed";
      body.style.minWidth = "220px";
      body.style.top = "0px";
      body.style.left = "0px";
      body.style.visibility = "hidden";
      // Placed left of its header (placement "left"), from wherever an
      // ancestor puts the fixed origin.
      var origin = body.getBoundingClientRect();
      var width = Math.max(body.offsetWidth, 220);
      var left = Math.max(5, header.left - width);
      var top = Math.min(header.top, window.innerHeight - body.offsetHeight - 5);
      body.style.transform =
        "translate(" + Math.round(left - origin.left) + "px, " + Math.round(top - origin.top) + "px)";
      body.style.visibility = "visible";
      body.style.pointerEvents = "auto";
    },
    true
  );

  document.addEventListener("click", function (event) {
    var open = document.querySelector(".more-actions-chat[open]");
    if (open && !open.contains(event.target)) {
      open.open = false;
      var message = open.closest(".chat-message-container");
      if (message && !message.matches(":hover")) {
        clearActive();
      }
    }
  });

  document.addEventListener("keydown", function (event) {
    var open = document.querySelector(".more-actions-chat[open]");
    if (event.key === "Escape" && open) {
      open.open = false;
    }
  });

  // The message whose emoji picker is open.
  var pickerMessage = null;

  // ChatMessageInteractor#openEmojiPicker: the picked emoji is added as a
  // reaction (selectReaction).
  function openReactionPicker(trigger) {
    var message = trigger.closest(".chat-message-container[data-id]");
    if (!message) {
      return;
    }
    pickerMessage = message;
    Discourse.emojiPicker.open(trigger, {
      context: "chat",
      onSelect: function (emoji) {
        react(message, emoji, "add");
      },
      onClose: function () {
        pickerMessage = null;
        if (!message.matches(":hover")) {
          clearActive();
        }
      },
    });
  }

  // TextareaTextManipulation#emojiSelected: a `:term` before the caret
  // completed, else the code added (after a space when text precedes it).
  function emojiSelected(input, code) {
    var start = input.selectionStart;
    var end = input.selectionEnd;
    var pre = input.value.substring(0, start);
    var captures = pre.match(/\B:([\p{L}\p{N}_]*)$/u);
    if (captures) {
      input.setRangeText(code + ":", start - captures[1].length, end, "end");
    } else {
      input.setRangeText(/\S$/.test(pre) ? " :" + code + ":" : ":" + code + ":", start, end, "end");
    }
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }

  // The composer's emoji button: the picked emoji goes into the composer
  // (onSelectEmoji).
  function openComposerPicker(trigger) {
    // EmojiPicker's own DMenu, 405 wide.
    Discourse.emojiPicker.open(trigger, {
      context: "chat",
      maxWidth: 405,
      onSelect: function (emoji) {
        var parts = composerParts();
        if (parts) {
          emojiSelected(parts.input, emoji);
          parts.input.focus();
        }
      },
    });
  }

  // react: the reaction toggled, the emoji counted as used, the page drawn
  // again.
  function react(message, emoji, action) {
    var channel = document.querySelector(".chat-channel[data-id]");
    return fetch(basePath() + "/chat/" + channel.dataset.id + "/react/" + message.dataset.id, {
      method: "PUT",
      credentials: "same-origin",
      headers: csrfHeaders(),
      body: new URLSearchParams({ emoji: emoji, react_action: action }).toString(),
    }).then(function (r) {
      if (!r.ok) {
        return failed(r);
      }
      Discourse.emojiStore.track(emoji, "chat");
      return changed();
    });
  }

  // The toast (toasts.success) a copied link shows for three seconds.
  function toast(text) {
    var tpl = document.querySelector("template.chat-toast-template");
    var section = document.querySelector("section.fk-d-toasts");
    if (!tpl) {
      return;
    }
    if (!section) {
      section = document.createElement("section");
      section.className = "fk-d-toasts";
      document.body.appendChild(section);
    }
    var output = tpl.content.querySelector(".fk-d-toast").cloneNode(true);
    output.querySelector(".fk-d-default-toast__message").textContent = text;
    section.appendChild(output);
    var close = function () {
      output.remove();
    };
    output.querySelector(".fk-d-default-toast__close-container button").addEventListener("click", close);
    setTimeout(close, 3000);
  }

  function copyLink(message) {
    var tpl = actionsTemplate();
    var channel = document.querySelector(".chat-channel[data-id]");
    var url = location.protocol + "//" + location.host + basePath() + "/chat/c/-/" + channel.dataset.id + "/" + message.dataset.id;
    var done = function () {
      toast(tpl.dataset.labelLinkCopied);
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(url).then(done, done);
    } else {
      done();
    }
  }

  function secondaryAction(message, value) {
    var channelId = document.querySelector(".chat-channel[data-id]").dataset.id;
    var path = "/channels/" + channelId + "/messages/" + message.dataset.id;
    clearActive();
    switch (value) {
      case "copyLink":
        copyLink(message);
        break;
      case "edit":
        startEdit(message);
        break;
      case "delete":
        api("DELETE", path).then(changed).catch(function () {});
        break;
      case "restore":
        api("PUT", path + "/restore").then(changed).catch(function () {});
        break;
      case "rebake":
        fetch(basePath() + "/chat/" + channelId + "/" + message.dataset.id + "/rebake", {
          method: "PUT",
          credentials: "same-origin",
          headers: csrfHeaders(),
        }).then(function (r) {
          return r.ok ? changed() : failed(r);
        });
        break;
    }
  }

  // expand: the run of deleted messages the label stands for, drawn.
  function expand(message) {
    var templates = [];
    var node = message.nextElementSibling;
    if (node && node.matches("template.chat-message-expanded")) {
      templates.push(node);
    }
    node = message.previousElementSibling;
    while (node && node.matches("template.chat-message-expanded")) {
      templates.unshift(node);
      node = node.previousElementSibling;
    }
    var fragment = document.createDocumentFragment();
    templates.forEach(function (template) {
      fragment.appendChild(template.content.cloneNode(true));
      template.remove();
    });
    message.replaceWith(fragment);
    var channel = document.querySelector(".chat-channel[data-id]");
    if (channel) {
      drawTimes(channel);
      decorateLinks(channel);
    }
  }

  document.addEventListener("click", function (event) {
    var target = event.target;
    var expandButton = target.closest(".chat-message-expand");
    if (expandButton) {
      expand(expandButton.closest(".chat-message-container"));
      return;
    }
    var row = target.closest(".more-actions-chat .select-kit-row");
    if (row) {
      var details = row.closest("details");
      details.open = false;
      secondaryAction(row.closest(".chat-message-container"), row.dataset.value);
      return;
    }
    var reaction = target.closest(".chat-message-reaction");
    if (reaction && reaction.closest(".chat-message-container[data-id]") && !reaction.disabled) {
      var message = reaction.closest(".chat-message-container[data-id]");
      if (!canInteract(message) && !reaction.closest(".chat-message-actions")) {
        return;
      }
      var action = reaction.classList.contains("reacted") ? "remove" : "add";
      reaction.disabled = true;
      react(message, reaction.dataset.emojiName, action).finally(function () {
        reaction.disabled = false;
      });
      return;
    }
    if (target.closest(".chat-message-actions .reply-btn")) {
      var replied = target.closest(".chat-message-container");
      clearActive();
      startReply(replied);
      return;
    }
    var reactButton = target.closest(".chat-message-actions .react-btn, .chat-message-react-btn");
    if (reactButton) {
      openReactionPicker(reactButton);
      return;
    }
    var emojiButton = target.closest(".chat-composer-button.--emoji");
    if (emojiButton && !emojiButton.disabled) {
      openComposerPicker(emojiButton);
    }
  });

  // ChatChannelSubscriptionManager: the channel's bus messages (Chat::
  // Publisher's, over /bus/events), each message drawn again by the server
  // for this viewer (GET /live/chat/:channel_id/:message_id) and swapped
  // in, one at a time and in order. A gap in the stream draws the page
  // again.
  var live = null;

  function listItems(list) {
    return Array.prototype.filter.call(list.children, function (node) {
      return node.matches("template.chat-message-expanded, .chat-message-container[data-id]");
    });
  }

  // The message's node in the page: its container, else its deleted form.
  function findMessage(list, id) {
    var found = null;
    listItems(list).forEach(function (node) {
      if (node.dataset.id === String(id) && (!found || found.matches("template"))) {
        found = node;
      }
    });
    return found;
  }

  function previousItem(list, node) {
    var items = listItems(list);
    var index = items.indexOf(node);
    return index > 0 ? items[index - 1] : null;
  }

  function fetchMessage(channelId, id, previous) {
    var url = basePath() + "/live/chat/" + channelId + "/" + id;
    if (previous) {
      url += "?previous=" + (previous.classList.contains("-staged") ? "staged" : previous.dataset.id);
    }
    return fetch(url, { credentials: "same-origin" }).then(function (r) {
      return r.ok ? r.text() : null;
    });
  }

  function fragment(html) {
    var template = document.createElement("template");
    template.innerHTML = html;
    return template.content.firstElementChild;
  }

  // The chat-messages-scroller runs bottom up: 0 is the latest message.
  function atBottom(channel) {
    var scroller = channel.querySelector(".chat-messages-scroller");
    return !scroller || Math.abs(scroller.scrollTop) < 50;
  }

  function redraw(channel) {
    drawTimes(channel);
    drawSeparators(channel);
    decorateLinks(channel);
    positionSeparators(channel);
  }

  // A run of deleted messages shows as its last one, collapsed to a label
  // counting them (shouldRender, deletedMessageLabel); each keeps its
  // expanded form for expand.
  function collapseRuns(channel, list) {
    list.querySelectorAll(":scope > .chat-message-container").forEach(function (node) {
      if (node.querySelector(":scope > .chat-message-text.-deleted")) {
        node.remove();
      }
    });
    var items = listItems(list);
    items.forEach(function (node, index) {
      if (!node.matches("template")) {
        return;
      }
      var next = items[index + 1];
      if (next && next.matches("template")) {
        return;
      }
      var count = 1;
      for (var j = index - 1; j >= 0 && items[j].matches("template"); j--) {
        count++;
      }
      var collapsed = node.content.firstElementChild.cloneNode(false);
      collapsed.classList.remove("-active");
      var text = el("div", "chat-message-text -deleted");
      var button = el("button", "btn btn-flat chat-message-expand");
      button.type = "button";
      var label = count === 1 ? channel.dataset.labelDeletedOne : channel.dataset.labelDeletedOther;
      button.appendChild(el("span", "d-button-label", label.replace("%{count}", String(count))));
      text.appendChild(button);
      collapsed.appendChild(text);
      list.insertBefore(collapsed, node);
    });
  }

  // The member's last read message is one: a message drawn as it moves
  // there takes the mark from the one before.
  function lastReadOnly(list, node) {
    if (!node.classList.contains("-last-read")) {
      return;
    }
    list.querySelectorAll(":scope > .chat-message-container.-last-read").forEach(function (other) {
      if (other !== node) {
        other.classList.remove("-last-read");
      }
    });
  }

  // handleSentMessage: a message not in the page yet, at its end.
  function onSent(channel, list, data) {
    var id = data.chat_message.id;
    if (findMessage(list, id)) {
      return Promise.resolve();
    }
    // handleStagedMessage: the member's staged message becomes the server's,
    // where it is.
    var staged = data.staged_id && findMessage(list, data.staged_id);
    if (staged) {
      return fetchMessage(channel.dataset.id, id, previousItem(list, staged)).then(function (html) {
        if (html && staged.isConnected && !findMessage(list, id)) {
          if (staged.classList.contains("-active")) {
            clearActive();
          }
          var sent = fragment(html);
          // Processed once the bus says so.
          sent.classList.replace("-processed", "-not-processed");
          staged.replaceWith(sent);
          lastReadOnly(list, sent);
          redraw(channel);
        }
      });
    }
    if (channel.classList.contains("is-empty")) {
      return refresh();
    }
    var items = listItems(list);
    var previous = items[items.length - 1] || null;
    var bottom = atBottom(channel);
    return fetchMessage(channel.dataset.id, id, previous).then(function (html) {
      if (!html || findMessage(list, id)) {
        return;
      }
      var added = fragment(html);
      list.appendChild(added);
      lastReadOnly(list, added);
      redraw(channel);
      if (bottom) {
        channel.querySelector(".chat-messages-scroller").scrollTop = 0;
      }
    });
  }

  // handleProcessedMessage, handleEditMessage, handleReactionMessage,
  // handleRefreshMessage and handleRestoreMessage: the message drawn again
  // where it is (a restored one in its place among the others).
  function onChanged(channel, list, id, type) {
    var node = findMessage(list, id);
    var shown = node && !node.matches("template") && !node.querySelector(":scope > .chat-message-text.-deleted");
    // Only a restore brings back a message not shown.
    if (type !== "restore" && !shown) {
      return Promise.resolve();
    }
    var previous;
    if (node) {
      previous = previousItem(list, node);
    } else {
      previous = null;
      listItems(list).forEach(function (item) {
        if (Number(item.dataset.id) < Number(id)) {
          previous = item;
        }
      });
    }
    return fetchMessage(channel.dataset.id, id, previous).then(function (html) {
      if (!html) {
        return;
      }
      var next = fragment(html);
      // Its nodes now: a container, or a collapsed label and its deleted form.
      var olds = listItems(list).filter(function (item) {
        return item.dataset.id === String(id);
      });
      if (olds.some(function (item) { return item.classList.contains("-active"); })) {
        clearActive();
      }
      if (olds.length) {
        olds[0].replaceWith(next);
        olds.slice(1).forEach(function (item) {
          item.remove();
        });
      } else if (previous && previous.parentNode === list) {
        list.insertBefore(next, previous.nextSibling);
      } else {
        list.insertBefore(next, list.firstChild);
      }
      collapseRuns(channel, list);
      redraw(channel);
    });
  }

  // handleDeleteMessage: its author, staff and moderators keep it, deleted
  // and collapsed; others lose it. The last read message moves to the
  // latest one left.
  function onDelete(channel, list, data) {
    var node = findMessage(list, data.deleted_id);
    if (!node || node.matches("template") || node.querySelector(":scope > .chat-message-text.-deleted")) {
      return Promise.resolve();
    }
    if (node.classList.contains("-active")) {
      clearActive();
    }
    var keeps =
      channel.dataset.staff === "true" ||
      channel.dataset.canModerate === "true" ||
      node.dataset.userId === channel.dataset.viewerId;
    var lastRead = node.classList.contains("-last-read");
    if (keeps) {
      var deleted = node.cloneNode(true);
      deleted.classList.add("-deleted");
      deleted.classList.remove("-active", "-last-read");
      deleted.dataset.deletedById = String(data.deleted_by_id);
      var template = document.createElement("template");
      template.className = "chat-message-expanded";
      template.dataset.id = node.dataset.id;
      template.content.appendChild(deleted);
      node.replaceWith(template);
    } else {
      node.remove();
    }
    if (lastRead && data.latest_not_deleted_message_id) {
      var latest = findMessage(list, data.latest_not_deleted_message_id);
      if (latest && !latest.matches("template")) {
        latest.classList.add("-last-read");
      }
    }
    collapseRuns(channel, list);
    redraw(channel);
    return Promise.resolve();
  }

  function onBus(data) {
    var channel = document.querySelector(".chat-channel[data-id]");
    var list = channel && channel.querySelector(".chat-messages-container");
    if (!list) {
      return Promise.resolve();
    }
    switch (data.type) {
      case "sent":
        return onSent(channel, list, data);
      case "processed":
      case "edit":
      case "refresh":
      case "restore":
        return onChanged(channel, list, data.chat_message.id, data.type);
      case "reaction":
        return onChanged(channel, list, data.chat_message_id, data.type);
      case "delete":
        return onDelete(channel, list, data);
      case "bulk_delete":
        return data.deleted_ids.reduce(function (done, id) {
          return done.then(function () {
            return onDelete(channel, list, { deleted_id: id, deleted_at: data.deleted_at });
          });
        }, Promise.resolve());
    }
    return Promise.resolve();
  }

  function startLive() {
    var channel = document.querySelector(".chat-channel[data-id]");
    if (!channel || !window.EventSource) {
      return;
    }
    var meta = document.querySelector("meta[name='bus-position']");
    var url =
      basePath() +
      "/bus/events?channels=" +
      encodeURIComponent("/chat/" + channel.dataset.id) +
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
      queue = queue.then(refresh).catch(function () {});
    });
    live = source;
  }

  function stopLive() {
    if (live) {
      live.close();
      live = null;
    }
  }

  // What follows a change the member made: the bus draws it, unless the
  // stream is down, when the page is drawn again.
  function changed() {
    if (live && live.readyState === 1) {
      return Promise.resolve();
    }
    return refresh();
  }

  var cleanup = null;

  // services/chat's presenceChannel: /chat/online, entered while chat is
  // open (unless the member hides their presence) and they were active in
  // the last five minutes; the avatars of those in it are drawn online.
  var CHAT_ONLINE_OPTIONS = { userUnseenTime: 300000, browserHiddenTime: 300000 };
  var onlineChannel = null;

  function onlineUsers() {
    return (onlineChannel && onlineChannel.users()) || null;
  }

  function isOnline(username) {
    var users = onlineUsers();
    return !!users && users.some(function (u) {
      return u.username === username;
    });
  }

  function markOnline() {
    if (!onlineUsers()) {
      return;
    }
    document.querySelectorAll(".chat-user-avatar[data-username]").forEach(function (node) {
      node.classList.toggle("is-online", isOnline(node.dataset.username));
    });
  }

  function hidePresence() {
    var page = document.querySelector(".full-page-chat");
    if (page) {
      return page.dataset.hidePresence === "true";
    }
    return !!Discourse.chatDrawer && Discourse.chatDrawer.hidePresence();
  }

  // ChatService#updatePresence: while chat is active, full page or in
  // the drawer.
  function startPresence() {
    if (!window.Discourse || !Discourse.presence || !document.querySelector(".full-page-chat, .chat-drawer")) {
      return;
    }
    if (!onlineChannel) {
      onlineChannel = Discourse.presence.getChannel("/chat/online");
      onlineChannel.on(markOnline);
      onlineChannel.subscribe().catch(function () {});
    } else {
      markOnline();
    }
    if (hidePresence()) {
      onlineChannel.leave().catch(function () {});
    } else {
      onlineChannel.enter({ activeOptions: CHAT_ONLINE_OPTIONS }).catch(function () {});
    }
    startReplyingIndicator();
  }

  function stopPresence() {
    if (onlineChannel) {
      onlineChannel.leave().catch(function () {});
      onlineChannel.unsubscribe();
      onlineChannel = null;
    }
    stopReplyingIndicator();
  }

  // ComposerPresenceManager#notifyState: present in /chat-reply/:id while
  // writing (not editing), for five seconds from the first keystroke.
  var CHAT_PRESENCE_KEEP_ALIVE = 5 * 1000;
  var replyingPresence = { name: null, channel: null, timer: null };

  function leaveReplying() {
    clearTimeout(replyingPresence.timer);
    replyingPresence.timer = null;
    if (replyingPresence.channel) {
      replyingPresence.channel.leave().catch(function () {});
    }
    replyingPresence.channel = null;
    replyingPresence.name = null;
  }

  function notifyReplying(name, isReplying) {
    if (!isReplying) {
      leaveReplying();
      return;
    }
    if (hidePresence() || !Discourse.presence) {
      return;
    }
    if (replyingPresence.name !== name) {
      leaveReplying();
      replyingPresence.name = name;
      replyingPresence.channel = Discourse.presence.getChannel(name);
      replyingPresence.channel.enter().catch(function () {});
      replyingPresence.timer = setTimeout(leaveReplying, CHAT_PRESENCE_KEEP_ALIVE);
    }
  }

  // ChatComposer#reportReplyingPresence
  function reportReplying(parts) {
    if (!parts.channel) {
      return;
    }
    notifyReplying("/chat-reply/" + parts.channel.dataset.id, !editing(parts) && hasContent(parts));
  }

  // ChatReplyingIndicator: who else is typing in the channel.
  var replyingIndicator = null;

  function replyingText(usernames, labels) {
    var fill = function (text, values) {
      return text.replace(/%\{(\w+)\}/g, function (_, key) {
        return values[key] === undefined ? "" : String(values[key]);
      });
    };
    if (usernames.length === 1) {
      return fill(labels.single_user, { username: usernames[0] });
    }
    if (usernames.length < 4) {
      return fill(labels.multiple_users, {
        commaSeparatedUsernames: usernames.slice(0, -1).join(labels.comma),
        lastUsername: usernames[usernames.length - 1],
      });
    }
    var count = usernames.length - 2;
    return fill(count === 1 ? labels.many_users_one : labels.many_users_other, {
      commaSeparatedUsernames: usernames.slice(0, 2).join(labels.comma),
      count: count,
    });
  }

  function drawReplying() {
    var node = document.querySelector(".chat-replying-indicator");
    var wrapper = document.querySelector(".chat-composer__wrapper[data-replying]");
    if (!node || !wrapper || !replyingIndicator) {
      return;
    }
    var tpl = actionsTemplate();
    var viewer = tpl ? tpl.dataset.viewerId : null;
    var usernames = (replyingIndicator.users() || [])
      .filter(function (u) {
        return String(u.id) !== viewer;
      })
      .map(function (u) {
        return u.username;
      });
    node.innerHTML = "";
    if (usernames.length === 0) {
      return;
    }
    var labels = JSON.parse(wrapper.dataset.replying);
    node.appendChild(el("span", "chat-replying-indicator__text", replyingText(usernames, labels)));
    var wave = el("span", "chat-replying-indicator__wave");
    for (var i = 0; i < 3; i++) {
      wave.appendChild(el("span", "chat-replying-indicator__dot", "."));
    }
    node.appendChild(wave);
  }

  function startReplyingIndicator() {
    var channel = document.querySelector(".chat-channel[data-id]");
    var name = channel ? "/chat-reply/" + channel.dataset.id : null;
    if (replyingIndicator && replyingIndicator.name === name) {
      drawReplying();
      return;
    }
    stopReplyingIndicator();
    if (!name || !document.querySelector(".chat-replying-indicator")) {
      return;
    }
    replyingIndicator = Discourse.presence.getChannel(name);
    replyingIndicator.on(drawReplying);
    replyingIndicator.subscribe().catch(function () {});
  }

  function stopReplyingIndicator() {
    if (replyingIndicator) {
      replyingIndicator.off(drawReplying);
      replyingIndicator.unsubscribe();
      replyingIndicator = null;
    }
    leaveReplying();
  }

  // The composer's autocompletes (ChatComposer#setupAutocomplete): @
  // users and groups, : emoji, # channels, categories and tags.
  function escapeHtml(s) {
    return String(s === undefined || s === null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#x27;");
  }

  // iconHTML: the sprite's symbol, with any extra class before svg-string.
  function iconHtml(name, extraClass) {
    return (
      '<svg class="fa d-icon d-icon-' + escapeHtml(name) + " svg-icon fa-width-auto" +
      (extraClass ? " " + escapeHtml(extraClass) : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      escapeHtml(name) + '"></use></svg>'
    );
  }

  // A promise of the results unless the next search (or five seconds)
  // overtakes it, as user-search and hashtag-autocomplete cancel theirs.
  function debouncedFetch(holder, delay, request) {
    clearTimeout(holder.timer);
    if (holder.controller) {
      holder.controller.abort();
      holder.controller = null;
    }
    if (holder.resolve) {
      holder.resolve(Discourse.autocomplete.CANCELLED);
    }
    return new Promise(function (resolve) {
      holder.resolve = resolve;
      var giveUp = setTimeout(function () {
        resolve(Discourse.autocomplete.CANCELLED);
      }, 5000);
      holder.timer = setTimeout(function () {
        var controller = new AbortController();
        holder.controller = controller;
        request(controller.signal)
          .then(function (result) {
            clearTimeout(giveUp);
            resolve(result);
          })
          .catch(function () {
            clearTimeout(giveUp);
            resolve(Discourse.autocomplete.CANCELLED);
          })
          .finally(function () {
            if (holder.controller === controller) {
              holder.controller = null;
            }
          });
      }, delay);
    });
  }

  function getJson(path, signal) {
    return fetch(basePath() + path, {
      credentials: "same-origin",
      headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
      signal: signal,
    }).then(function (r) {
      if (r.status === 429) {
        var wait = Number(r.headers.get("Retry-After") || 0);
        userSearchState.rateLimitedUntil = Date.now() + wait * 1000;
      }
      if (!r.ok) {
        throw new Error(String(r.status));
      }
      return r.json();
    });
  }

  // lib/user-search: the term searched 300ms after typing stops, cached
  // for 30 seconds, ranked by organizeResults.
  var userSearchState = { cache: {}, cacheTime: 0, rateLimitedUntil: 0, holder: {} };
  var USER_IGNORE =
    /([ -⁯⸀-⹿\\'!"#$%&()*,\/:;<=>?\[\]^`{|}~])|\s\s|^\s$|^[^+]*\+[^@]*$/;

  function lowerCaseIncludes(string, term) {
    return !!(string && term && string.toLowerCase().indexOf(term.toLowerCase()) !== -1);
  }

  function organizeResults(r, term, limit) {
    if (r === Discourse.autocomplete.CANCELLED) {
      return r;
    }
    term = ((term || "").trim().split(/\s/, 1) || [])[0];
    var users = (r.users || []).map(function (user) {
      user.isUser = true;
      user.isMetadataMatch = !!term && !lowerCaseIncludes(user.username, term) && !lowerCaseIncludes(user.name, term);
      return user;
    });
    var groups = (r.groups || []).map(function (group) {
      group.isGroup = true;
      return group;
    });
    var userSet = {};
    var groupSet = {};
    var lower = function (s) {
      return s === undefined || s === null ? s : String(s).toLowerCase();
    };
    var pick = function (list, set, key, test) {
      return list.filter(function (item) {
        if (set[item[key]] || !test(item)) {
          return false;
        }
        set[item[key]] = true;
        return true;
      });
    };
    var ordered = [].concat(
      pick(users, userSet, "username", function (u) {
        return lower(u.username) === lower(term);
      }),
      pick(groups, groupSet, "name", function (g) {
        return lower(g.name) === lower(term);
      }),
      pick(users, userSet, "username", function (u) {
        return !u.isMetadataMatch && !(term && !lowerCaseIncludes(u.username, term));
      }),
      pick(groups, groupSet, "name", function (g) {
        return !(term && !lowerCaseIncludes(g.name, term));
      }),
      pick(users, userSet, "username", function (u) {
        return lower(u.name) === lower(term) && !u.isMetadataMatch;
      }),
      pick(users, userSet, "username", function (u) {
        return lowerCaseIncludes(u.name, term) && !u.isMetadataMatch;
      }),
      pick(groups, groupSet, "name", function (g) {
        return !!g.full_name && lower(g.full_name) === lower(term);
      }),
      pick(groups, groupSet, "name", function (g) {
        return !!g.full_name && lowerCaseIncludes(g.full_name, term);
      }),
      users.filter(function (u) {
        return u.isMetadataMatch;
      })
    );
    return ordered.slice(0, limit);
  }

  function userSearch(term) {
    if (term && term.charAt(0) === "@") {
      term = term.substring(1);
    }
    term = term || "";
    var s = userSearchState;
    if (Date.now() - s.cacheTime > 30000) {
      s.cache = {};
    }
    // skipSearch, without emails.
    if (term.indexOf("@") !== -1 || USER_IGNORE.test(term)) {
      return Promise.resolve([]);
    }
    return debouncedFetch(s.holder, 300, function (signal) {
      if (s.cache[term]) {
        return Promise.resolve(JSON.parse(JSON.stringify(s.cache[term])));
      }
      if (s.rateLimitedUntil > Date.now() || term === "") {
        return Promise.resolve(Discourse.autocomplete.CANCELLED);
      }
      var query = new URLSearchParams({ term: term, include_groups: "true", limit: "6" });
      return getJson("/u/search/users?" + query.toString(), signal).then(function (r) {
        s.cache[term] = r;
        s.cacheTime = Date.now();
        return JSON.parse(JSON.stringify(r));
      });
    }).then(function (r) {
      return organizeResults(r, term, 6);
    });
  }

  // UserAutocompleteResults
  function renderUsers(results, selected) {
    var out = '<div class="autocomplete ac-user"><ul>';
    results.forEach(function (result, index) {
      var classes = [];
      // Those in the chat presence channel are online.
      if (result.isUser && isOnline(result.username)) {
        classes.push("is-online");
      }
      if (index === selected) {
        classes.push("selected");
      }
      var title = result.isUser ? result.name : result.isEmail ? result.username : result.full_name;
      out += '<li><a class="' + classes.join(" ") + '" href=""' +
        (title === undefined || title === null ? "" : ' title="' + escapeHtml(title) + '"') + ">";
      if (result.isUser) {
        out +=
          '<img alt="" width="24" height="24" src="' +
          escapeHtml((result.avatar_template || "").replace("{size}", "24")) +
          '" class="avatar" title="' + escapeHtml(result.username) + '">' +
          '<span class="text-content"><span class="username">' + escapeHtml(result.username) + "</span>" +
          (result.name ? '<span class="name">' + escapeHtml(result.name) + "</span>" : "") +
          "</span>" + (result.status ? '<span class="user-status"></span>' : "");
      } else if (result.isGroup) {
        out +=
          iconHtml("users") +
          '<span class="text-content"><span class="username">' + escapeHtml(result.name) +
          '</span><span class="name">' + escapeHtml(result.full_name) + "</span></span>";
      }
      out += "</a></li>";
    });
    return out + "</ul></div>";
  }

  // EmojiAutocompleteResults
  function renderEmojis(results, selected) {
    var out = '<div class="autocomplete ac-emoji"><ul>';
    results.forEach(function (result, index) {
      out += "<li><a" + (index === selected ? ' class="selected"' : "") + ' href=""><span class="text-content">';
      if (result.src) {
        out += '<img class="emoji" src="' + escapeHtml(result.src) + '"><span class="emoji-shortname">' +
          escapeHtml(result.code) + "</span>";
      } else {
        out += escapeHtml(result.label);
      }
      out += "</span></a></li>";
    });
    return out + "</ul></div>";
  }

  // RichHashtagAutocompleteResults
  function renderHashtags(results, selected) {
    var out = '<div class="autocomplete hashtag-autocomplete"><div class="hashtag-autocomplete__fadeout"><ul>';
    results.forEach(function (result, index) {
      out +=
        '<li class="hashtag-autocomplete__option"><a class="hashtag-autocomplete__link' +
        (index === selected ? " selected" : "") + '" href="">' + result.iconHtml +
        '<span class="hashtag-autocomplete__text">' + result.text +
        (result.secondary_text
          ? '<span class="hashtag-autocomplete__meta-text">(' + escapeHtml(result.secondary_text) + ")</span>"
          : "") +
        "</span></a></li>";
    });
    return out + "</ul></div></div>";
  }

  // The hashtag types' color classes (generateColorCssClasses), added to
  // #hashtag-css-generator as each hashtag loads (onLoad).
  var loadedHashtags = {};
  // The site's categories, preloaded with the page's style.
  (function () {
    var style = document.getElementById("hashtag-css-generator");
    var ids = style && style.dataset.categoryIds;
    (ids ? ids.split(",") : []).forEach(function (id) {
      loadedHashtags["category-" + id] = true;
    });
  })();

  function hashtagCss(result) {
    var key = result.type + "-" + result.id;
    if (loadedHashtags[key] || !result.colors) {
      return;
    }
    loadedHashtags[key] = true;
    var rule = null;
    if (result.type === "category") {
      var color = result.colors.length === 1 ? result.colors[0] : result.colors[1];
      var parent = result.colors.length === 1 ? null : result.colors[0];
      if (result.style_type === "square") {
        rule = parent
          ? "background: linear-gradient(-90deg, #" + color + " 50%, #" + parent + " 50%);"
          : "background-color: #" + color + ";";
        rule = ".hashtag-color--category-" + result.id + " { " + rule + " }";
      } else if (result.style_type === "icon") {
        rule = ".hashtag-color--category-" + result.id + " { color: #" + color + "; }";
      }
    } else if (result.type === "channel") {
      rule = ".d-icon.hashtag-color--channel-" + result.id + " { color: #" + result.colors[0] + " }";
    }
    if (!rule) {
      return;
    }
    var style = document.getElementById("hashtag-css-generator");
    if (!style) {
      style = document.createElement("style");
      style.id = "hashtag-css-generator";
      style.innerHTML = ".hashtag-category-square { background-color: var(--primary-medium); }";
      document.head.appendChild(style);
    }
    style.innerHTML += "\n" + rule;
  }

  // The hashtag types' generateIconHTML.
  function hashtagIcon(result) {
    var color = "hashtag-color--" + result.type + "-" + result.id;
    if (result.type === "category") {
      var inner = "";
      if (result.style_type === "icon" && result.icon) {
        inner = iconHtml(result.icon);
      }
      if (result.style_type === "emoji" && result.emoji) {
        inner =
          '<img width="20" height="20" src="' + escapeHtml(Discourse.emojiPicker.url(result.emoji)) +
          '" title="' + escapeHtml(result.emoji) + '" alt="' + escapeHtml(result.emoji) + '" class="emoji">';
      }
      return '<span class="hashtag-category-' + escapeHtml(result.style_type) + " " + color + '">' + inner + "</span>";
    }
    if (result.type === "channel" && result.emoji) {
      return (
        '<img width="20" height="20" src="' + escapeHtml(Discourse.emojiPicker.url(result.emoji)) +
        '" title="' + escapeHtml(result.emoji) + '" alt="' + escapeHtml(result.emoji) + '" class="emoji">'
      );
    }
    return iconHtml(result.icon, color);
  }

  // lib/hashtag-autocomplete: searched 250ms after typing stops (the
  // empty term too, for the defaults), cached for 30 seconds.
  var hashtagState = { cache: {}, cacheTime: 0, holder: {} };

  function hashtagSearch(term, order) {
    if (/\s/.test(term)) {
      return null;
    }
    var s = hashtagState;
    if (Date.now() - s.cacheTime > 30000) {
      s.cache = {};
    }
    if (s.cache[term]) {
      return s.cache[term];
    }
    return debouncedFetch(s.holder, 250, function (signal) {
      var query = new URLSearchParams({ term: term });
      order.forEach(function (type) {
        query.append("order[]", type);
      });
      return getJson("/hashtags/search.json?" + query.toString(), signal).then(function (response) {
        var results = response.results;
        if (!results) {
          return Discourse.autocomplete.CANCELLED;
        }
        return Promise.all(
          results.map(function (result) {
            return Discourse.emojiPicker.unescape(escapeHtml(result.text), { shortcuts: true }).then(function (text) {
              result.text = text;
              hashtagCss(result);
              result.iconHtml = hashtagIcon(result);
              return result;
            });
          })
        ).then(function (all) {
          s.cache[term] = all;
          s.cacheTime = Date.now();
          return all;
        });
      });
    });
  }

  // inCodeBlock: the caret, marked with a private-use character, cooks
  // inside code.
  var CODE_MARKERS = /    |```|~~~|[^`]`[^`]|\[code\]/;

  function inCodeBlock(input) {
    var text = input.value;
    if (!CODE_MARKERS.test(text)) {
      return Promise.resolve(false);
    }
    var marker = null;
    for (var code = 0xe000; code <= 0xf8ff; ++code) {
      if (text.indexOf(String.fromCharCode(code)) === -1) {
        marker = String.fromCharCode(code);
        break;
      }
    }
    if (!marker) {
      return Promise.resolve(false);
    }
    var pos = input.selectionStart;
    return cookStaged(text.slice(0, pos) + marker + text.slice(pos)).then(function (html) {
      var doc = new DOMParser().parseFromString("<body>" + html + "</body>", "text/html");
      var found = false;
      doc.querySelectorAll("code").forEach(function (node) {
        if (node.textContent.indexOf(marker) !== -1) {
          found = true;
        }
      });
      return found;
    });
  }

  function setupAutocomplete() {
    var parts = composerParts();
    if (!parts || !parts.input || parts.input.chatAutocomplete) {
      return;
    }
    var input = parts.input;
    var cfg = {};
    try {
      cfg = JSON.parse(parts.wrapper.dataset.autocomplete || "{}");
    } catch (e) {
      return;
    }
    input.chatAutocomplete = true;
    var refocus = function (text, event) {
      if (event && event.preventDefault) {
        event.preventDefault();
      }
      input.focus();
    };
    if (cfg.mentions) {
      Discourse.autocomplete.setup(input, {
        key: "@",
        render: renderUsers,
        transformComplete: function (obj) {
          return obj.username || obj.name;
        },
        dataSource: userSearch,
        afterComplete: refocus,
      });
    }
    if (cfg.emoji) {
      var emojiTerm = null;
      Discourse.autocomplete.setup(input, {
        key: ":",
        render: renderEmojis,
        afterComplete: refocus,
        onKeyUp: function (text, cp) {
          var matches =
            /(?:^|[\s.\?,@\/#!%&*;:\[\]{}=\-_()+])(:(?!:).?[\w-]*:?(?!:)(?:t\d?)?:?)$/gi.exec(text.substring(0, cp));
          if (matches && matches[1]) {
            return [matches[1]];
          }
        },
        transformComplete: function (v) {
          if (v.code) {
            return v.code + ":";
          }
          // "more…": the picker, its pick in place of the term.
          var value = input.value;
          var caret = input.selectionStart;
          var incomplete = value.substring(0, caret).match(/(:[\p{L}\p{N}_-]+)$/u);
          emojiTerm = incomplete ? { start: caret - incomplete[1].length, end: caret - 1 } : null;
          Discourse.emojiPicker.open(input, {
            context: "chat",
            term: v.term,
            onSelect: function (emoji) {
              if (emojiTerm) {
                var before = input.value.substring(0, emojiTerm.start) + ":" + emoji + ": ";
                input.value = before + input.value.substring(emojiTerm.end + 1);
                input.setSelectionRange(before.length, before.length);
                input.dispatchEvent(new Event("input", { bubbles: true }));
              } else {
                emojiSelected(input, emoji);
              }
              input.focus();
            },
          });
          return "";
        },
        dataSource: function (term) {
          var full = ":" + term;
          term = term.toLowerCase();
          // No quick autocomplete: two characters at least, and not :-p
          // or other typed smileys.
          if (term.length < Math.max(cfg.emojiMinChars || 0, 2)) {
            return Promise.resolve(Discourse.autocomplete.SKIP);
          }
          if (!term.match(/[^-\{\}\[\]\(\)\*_\<\>\\\/].*[^-\{\}\[\]\(\)\*_\<\>\\\/]/)) {
            return Promise.resolve(Discourse.autocomplete.SKIP);
          }
          var denied = cfg.deniedEmojis || [];
          return Discourse.emojiPicker
            .translations()
            .then(function (translations) {
              if (translations[full]) {
                return [translations[full]];
              }
              var match = term.match(/^:?(.*?):t([2-6])?$/);
              var tonable = match
                ? Discourse.emojiPicker.isSkinTonable(match[1])
                : Promise.resolve(false);
              return tonable.then(function (yes) {
                if (yes && denied.indexOf(match[1]) === -1) {
                  return match[2]
                    ? [match[1] + ":t" + match[2]]
                    : [2, 3, 4, 5, 6].map(function (x) {
                        return match[1] + ":t" + x;
                      });
                }
                return Discourse.emojiPicker.search(term, {
                  maxResults: 5,
                  diversity: Discourse.emojiStore.diversity(),
                  exclude: denied,
                });
              });
            })
            .then(function (list) {
              if (list === Discourse.autocomplete.SKIP) {
                return list;
              }
              var out = list.map(function (code) {
                return { code: code, src: Discourse.emojiPicker.url(code) };
              });
              if (out.length) {
                out.push({ label: cfg.moreEmoji, term: term });
              }
              return out;
            });
        },
      });
    }
    Discourse.autocomplete.setup(input, {
      key: "#",
      render: renderHashtags,
      autoSelectFirstSuggestion: true,
      transformComplete: function (obj) {
        return obj.ref;
      },
      dataSource: function (term) {
        return hashtagSearch(term, cfg.hashtagOrder || []);
      },
      triggerRule: function () {
        return inCodeBlock(input).then(function (inCode) {
          return !inCode;
        });
      },
      afterComplete: refocus,
    });
  }

  // `focus`: the composer takes the focus, as ChatComposer does when the
  // channel is drawn (not when a page swaps in beside the drawer's).
  function setup(focus) {
    var channel = document.querySelector(".chat-channel[data-id]");
    if (!channel) {
      return;
    }
    // ChatChannel#setup: opening a DM one doesn't follow follows it.
    if (
      channel.dataset.directMessage === "true" &&
      channel.dataset.following !== "true" &&
      !channel.dataset.followRequested
    ) {
      channel.dataset.followRequested = "true";
      api("POST", "/channels/" + channel.dataset.id + "/memberships/me").then(refresh);
    }
    setupAutocomplete();
    drawTimes(channel);
    drawSeparators(channel);
    decorateLinks(channel);
    positionSeparators(channel);
    var timer = null;
    var scheduleRead = function () {
      clearTimeout(timer);
      timer = setTimeout(function () {
        markRead(channel);
      }, 1000);
    };
    var onResize = function () {
      positionSeparators(channel);
      scheduleRead();
    };
    var scroller = channel.querySelector(".chat-messages-scroller");
    window.addEventListener("resize", onResize);
    document.addEventListener("visibilitychange", scheduleRead);
    if (scroller) {
      scroller.addEventListener("scroll", scheduleRead, { passive: true });
    }
    scheduleRead();
    var parts = composerParts();
    if (parts) {
      updateSendState(parts);
      if (!parts.input.disabled) {
        // ChatComposer autofocuses on desktop only: on a phone it would
        // open the keyboard.
        if (focus && !document.documentElement.classList.contains("mobile-view")) {
          parts.input.focus({ preventScroll: true });
        }
        refreshHeight(parts.input);
      }
    }
    cleanup = function () {
      clearTimeout(draftTimer);
      clearTimeout(timer);
      window.removeEventListener("resize", onResize);
      document.removeEventListener("visibilitychange", scheduleRead);
      if (scroller) {
        scroller.removeEventListener("scroll", scheduleRead);
      }
    };
  }

  function teardown() {
    if (cleanup) {
      cleanup();
      cleanup = null;
    }
  }

  // ChatChannelMetadata: moment(at).calendar in the local zone, with
  // sameDay "LT", lastDay Yesterday, lastWeek "dddd" and sameElse "l".
  function drawChannelDates() {
    document.querySelectorAll(".chat-channel__metadata-date[data-at]").forEach(function (node) {
      var at = new Date(node.dataset.at);
      var today = new Date();
      var startOfDay = function (d) {
        return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
      };
      var days = Math.round((startOfDay(at) - startOfDay(today)) / 86400000);
      var text;
      if (days === 0) {
        text = at.toLocaleTimeString(lang, { hour: "numeric", minute: "2-digit" });
      } else if (days === -1) {
        text = node.dataset.labelYesterday;
      } else if (days >= -6 && days < -1) {
        text = at.toLocaleString(lang, { weekday: "long" });
      } else {
        text = at.getMonth() + 1 + "/" + at.getDate() + "/" + at.getFullYear();
      }
      node.textContent = text;
    });
  }

  // ChatChannelArchiveStatus#retryArchive (createChannelArchive), its
  // error as popupAjaxError shows it.
  document.addEventListener("click", function (event) {
    var retry = event.target.closest(".chat-channel-archive-failed-retry .btn[data-channel-id]");
    if (retry) {
      api("POST", "/channels/" + retry.dataset.channelId + "/archives").catch(function () {});
    }
  });

  // A block's button (Blocks#createInteraction): the interaction sent,
  // the button loading meanwhile; an error as popupAjaxError shows it.
  document.addEventListener("click", function (event) {
    var button = event.target.closest(".chat-message__blocks .block__button[data-message-id]");
    var channel = button && button.closest(".chat-channel[data-id]");
    if (!button || !channel || button.classList.contains("is-loading")) {
      return;
    }
    button.classList.add("is-loading");
    button.disabled = true;
    api(
      "POST",
      "/channels/" + channel.dataset.id + "/messages/" + button.dataset.messageId + "/interactions",
      { action_id: button.id }
    )
      .catch(function () {})
      .then(function () {
        button.classList.remove("is-loading");
        button.disabled = false;
      });
  });

  // The chat footer's buttons and the empty channels list's are routes
  // (DButton @route).
  document.addEventListener("click", function (event) {
    var button = event.target.closest(".c-footer__item[data-href], .--empty-channels-list .empty-state__cta .btn[data-href]");
    if (button) {
      window.location.assign(button.dataset.href);
    }
  });

  function mount(focus) {
    drawChannelDates();
    setup(focus);
    startLive();
    startPresence();
  }

  function unmount() {
    teardown();
    stopLive();
    stopPresence();
  }

  // ChatComposerDropdown: the "+" trigger's menu of the composer buttons
  // registered for the dropdown (registerChatComposerButton, position
  // "dropdown"), each {id, icon, label, displayed(context), action(context,
  // textarea)}. Core's own (upload, GIFs, local dates, the mobile emoji)
  // are not ported, so only the plugins' show; with none the trigger is
  // left alone, as DMenu renders no trigger without buttons.
  Discourse.chatComposerButtons = Discourse.chatComposerButtons || [];
  document.addEventListener("click", function (event) {
    var trigger = event.target.closest(".chat-composer-dropdown__trigger-btn");
    if (!trigger || trigger.disabled || !Discourse.float) {
      return;
    }
    var textarea = trigger.closest(".chat-composer__wrapper");
    textarea = textarea && textarea.querySelector("textarea.chat-composer__input");
    var context = textarea ? textarea.dataset.chatComposerContext : null;
    var buttons = Discourse.chatComposerButtons.filter(function (b) {
      return !b.displayed || b.displayed(context, textarea);
    });
    if (!buttons.length) {
      return;
    }
    var html = '<ul class="chat-composer-dropdown__list">';
    buttons.forEach(function (b) {
      html +=
        '<li class="chat-composer-dropdown__item ' + escapeHtml(b.id) + '">' +
        '<button class="btn btn-icon-text chat-composer-dropdown__action-btn btn-transparent ' + escapeHtml(b.id) +
        '" type="button" data-id="' + escapeHtml(b.id) + '">' + iconHtml(b.icon) +
        '<span class="d-button-label">' + escapeHtml(b.label) + "</span></button></li>";
    });
    html += "</ul>";
    var menu = Discourse.float.menu(trigger, {
      identifier: "chat-composer-dropdown__menu",
      arrow: true,
      modalForMobile: true,
      placement: "top",
      fallbackPlacements: ["bottom"],
      content: html,
    });
    var content = menu && menu.content;
    if (content) {
      content.addEventListener("click", function (e) {
        var button = e.target.closest(".chat-composer-dropdown__action-btn");
        if (!button) {
          return;
        }
        var picked = buttons.filter(function (b) {
          return b.id === button.dataset.id;
        })[0];
        menu.close();
        if (picked) {
          picked.action(context, textarea);
        }
      });
    }
  });

  // For chat-drawer.js: the drawer's route drawn, or gone.
  Discourse.chat = {
    mount: function () {
      unmount();
      mount(true);
    },
    unmount: unmount,
  };

  Discourse.onPage(function () {
    // A channel in the drawer, kept across pages, keeps its state but
    // doesn't take the focus.
    mount(!!document.querySelector(".full-page-chat"));
    return unmount;
  });
})();
