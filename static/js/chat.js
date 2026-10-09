// Full-page chat in the browser: what Ember's chat components compute
// from the browser's clock and zone, drawn over the server's markup.
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
    return page ? page.dataset.basePath : "";
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
  function refresh() {
    return fetch(location.href, { credentials: "same-origin" })
      .then(function (r) {
        return r.text();
      })
      .then(function (html) {
        var next = new DOMParser().parseFromString(html, "text/html");
        var page = document.querySelector(".full-page-chat");
        var nextPage = next.querySelector(".full-page-chat");
        if (page && nextPage) {
          var composer = page.querySelector(".chat-composer__wrapper");
          var nextComposer = nextPage.querySelector(".chat-composer__wrapper");
          var focused = composer && composer.contains(document.activeElement);
          teardown();
          if (composer && nextComposer) {
            nextComposer.replaceWith(composer);
          }
          page.replaceWith(nextPage);
          setup();
          if (focused) {
            var input = document.getElementById("channel-composer");
            if (input) {
              input.focus({ preventScroll: true });
            }
          }
        }
        refreshSidebar(next);
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
  }

  function cardChannelId(button) {
    var card = button.closest("[data-channel-id]");
    if (card) {
      return card.dataset.channelId;
    }
    var channel = document.querySelector(".chat-channel[data-id]");
    return channel ? channel.dataset.id : null;
  }

  document.addEventListener("click", function (event) {
    var button = event.target.closest(
      ".toggle-channel-membership-button, .c-navbar__star-channel-button"
    );
    if (!button || button.disabled) {
      return;
    }
    var id = cardChannelId(button);
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
    request.then(refresh).catch(function () {
      button.disabled = false;
    });
  });

  // updateLastReadMessage: the last message whose bottom shows, once the
  // pane has settled, marks the channel read up to it.
  function lastVisibleMessage(channel) {
    var scroller = channel.querySelector(".chat-messages-scroller");
    if (!scroller) {
      return null;
    }
    var view = scroller.getBoundingClientRect();
    var found = null;
    channel.querySelectorAll(".chat-message-container[data-id]").forEach(function (message) {
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
    if (id <= Number(channel.dataset.lastRead || 0)) {
      return;
    }
    channel.dataset.lastRead = String(id);
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

  // ChatUserAvatar, the viewer's own marked online.
  function chatUserAvatar(user, size) {
    var tpl = actionsTemplate();
    var wrapper = el("div", "chat-user-avatar");
    if (tpl && String(user.id) === tpl.dataset.viewerId) {
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
    var max = Number(parts.wrapper.dataset.maxLength || 0);
    if (max && message.length > max) {
      var tpl = actionsTemplate();
      window.alert((tpl ? tpl.dataset.labelTooLong : "").replace("%{count}", String(max)));
      return;
    }
    var reply = replying(parts);
    // The composer empties as the message goes (Ember stages it, and
    // resets the draft), and gets its text back if sending fails.
    resetComposer(parts);
    parts.composer.classList.add("is-sending");
    var request;
    if (edit) {
      request = fetch(basePath() + "/chat/api/channels/" + channelId + "/messages/" + edit.dataset.id, {
        method: "PUT",
        credentials: "same-origin",
        headers: csrfHeaders(),
        body: new URLSearchParams({ message: message }).toString(),
      });
    } else {
      var params = {
        message: message,
        staged_id: "staged-" + Date.now() + "-" + Math.floor(Math.random() * 1e6),
        client_created_at: new Date().toISOString(),
      };
      if (reply) {
        params.in_reply_to_id = reply.dataset.id;
      }
      request = fetch(basePath() + "/chat/" + channelId, {
        method: "POST",
        credentials: "same-origin",
        headers: csrfHeaders(),
        body: new URLSearchParams(params).toString(),
      });
    }
    request
      .then(function (r) {
        if (r.ok) {
          return changed();
        }
        // A failed edit stays reset (popupAjaxError); a failed message
        // comes back to the composer.
        if (!edit) {
          parts.input.value = message + parts.input.value;
        }
        return failed(r);
      })
      .finally(function () {
        var now = composerParts();
        if (now) {
          now.composer.classList.remove("is-sending");
          updateSendState(now);
          now.input.focus();
        }
      });
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
    }
  });

  document.addEventListener("keydown", function (event) {
    if (event.target.id !== "channel-composer" || event.isComposing) {
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

  // The emoji store (EmojiStore, the same localStorage keys): the chat
  // context's recent emoji and the chosen skin tone.
  var STORE = "discourse_emoji_reaction_";

  function storeGet(key) {
    try {
      var value = window.localStorage.getItem(STORE + key);
      return value === null ? null : JSON.parse(value);
    } catch (e) {
      return null;
    }
  }

  function storeSet(key, value) {
    try {
      window.localStorage.setItem(STORE + key, JSON.stringify(value));
    } catch (e) {
      // No storage: the quick reactions stay the defaults.
    }
  }

  function diversity() {
    return storeGet("emojiSelectedDiversity") || 1;
  }

  function trackEmoji(emoji) {
    var recent = storeGet("chat_emojiUsage") || [];
    recent.unshift(emoji.replace(/(^:)|(:$)/g, ""));
    recent.length = Math.min(recent.length, 40);
    storeSet("chat_emojiUsage", recent);
  }

  function tonable(tpl, emoji) {
    return (tpl.dataset.tonable || "").split("|").indexOf(emoji.split(":")[0]) !== -1;
  }

  function withTone(tpl, emoji) {
    var tone = diversity();
    return tone !== 1 && tonable(tpl, emoji) ? emoji + ":t" + tone : emoji;
  }

  // quickReactionEmojis: the member's custom ones, then the most used,
  // then the site's defaults; three of them.
  function quickReactions(tpl) {
    var custom = (tpl.dataset.quickCustom || "").split("|").filter(Boolean);
    var counters = {};
    (storeGet("chat_emojiUsage") || []).forEach(function (emoji) {
      counters[emoji] = (counters[emoji] || 0) + 1;
    });
    var frequent = Object.keys(counters)
      .sort(function (a, b) {
        return counters[b] - counters[a];
      })
      .slice(0, 20)
      .map(function (emoji) {
        return withTone(tpl, emoji);
      });
    var defaults = (tpl.dataset.quickDefaults || "").split("|").map(function (emoji) {
      return withTone(tpl, emoji);
    });
    var all = custom.concat(frequent).concat(defaults);
    return all
      .filter(function (item, index) {
        return all.indexOf(item) === index;
      })
      .filter(Boolean)
      .slice(0, 3);
  }

  function emojiUrl(tpl, emoji) {
    return tpl.dataset.emojiUrl.replace("%{name}", emoji.replace(":t", "/"));
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

  function menuOpen() {
    return !!document.querySelector(".chat-message-actions-container .more-buttons.is-expanded");
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
    if (message.querySelector(".chat-message-expand")) {
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
      trackEmoji(emoji);
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
      url += "?previous=" + previous.dataset.id;
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

  // handleSentMessage: a message not in the page yet, at its end.
  function onSent(channel, list, data) {
    var id = data.chat_message.id;
    if (findMessage(list, id)) {
      return Promise.resolve();
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
      list.appendChild(fragment(html));
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

  function setup() {
    var channel = document.querySelector(".chat-channel[data-id]");
    if (!channel) {
      return;
    }
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
        parts.input.focus({ preventScroll: true });
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

  Discourse.onPage(function () {
    setup();
    startLive();
    return function () {
      teardown();
      stopLive();
    };
  });
})();
