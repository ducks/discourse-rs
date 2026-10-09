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

  // The parts of the page a membership change redraws (the chat page,
  // chat's sidebar sections), from the page as the server now draws it:
  // what Ember's tracked state redraws in place.
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
          teardown();
          page.replaceWith(nextPage);
          setup();
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

  // ChatComposer: the send button follows hasContent, the draft is saved
  // two seconds after typing stops (persistDraft), and the send shortcut
  // (Enter, or Ctrl/Cmd+Enter with meta_enter) or the button sends.
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
    };
  }

  function hasContent(parts) {
    var min = Number(parts.wrapper.dataset.minLength || 1) || 1;
    return parts.input.value.length >= min;
  }

  function updateSendState(parts) {
    var enabled = hasContent(parts) && !parts.input.disabled;
    parts.composer.classList.toggle("is-send-enabled", enabled);
    parts.composer.classList.toggle("is-send-disabled", !enabled);
    parts.send.disabled = !enabled;
    parts.send.tabIndex = enabled ? 0 : -1;
  }

  var draftTimer = null;

  function saveDraft(parts) {
    clearTimeout(draftTimer);
    parts.composer.classList.remove("is-draft-saved");
    parts.composer.classList.add("is-draft-unsaved");
    var channelId = parts.channel.dataset.id;
    var message = parts.input.value;
    draftTimer = setTimeout(function () {
      // toJSONDraft: null for an empty draft, which removes it.
      var params = message.length ? { "data[message]": message } : undefined;
      api("POST", "/channels/" + channelId + "/drafts", params)
        .then(function () {
          parts.composer.classList.remove("is-draft-unsaved");
          parts.composer.classList.add("is-draft-saved");
        })
        .catch(function () {});
    }, 2000);
  }

  function sendMessage(parts) {
    if (!hasContent(parts) || parts.input.disabled || parts.composer.classList.contains("is-sending")) {
      return;
    }
    clearTimeout(draftTimer);
    var message = parts.input.value;
    var stagedId = "staged-" + Date.now() + "-" + Math.floor(Math.random() * 1e6);
    // The composer empties as the message goes (Ember stages it), and gets
    // its text back if sending fails.
    parts.input.value = "";
    updateSendState(parts);
    parts.composer.classList.add("is-sending");
    fetch(basePath() + "/chat/" + parts.channel.dataset.id, {
      method: "POST",
      credentials: "same-origin",
      headers: csrfHeaders(),
      body: new URLSearchParams({
        message: message,
        staged_id: stagedId,
        client_created_at: new Date().toISOString(),
      }).toString(),
    })
      .then(function (r) {
        if (r.ok) {
          // What was typed while it went stays in the composer.
          var typed = parts.input.value;
          return refresh().then(function () {
            var now = composerParts();
            if (now && typed) {
              now.input.value = typed;
            }
          });
        }
        return r
          .json()
          .catch(function () {
            return {};
          })
          .then(function (body) {
            parts.input.value = message + parts.input.value;
            window.alert((body.errors || [r.statusText]).join("\n"));
          });
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
    }
  });

  document.addEventListener("keydown", function (event) {
    if (event.target.id !== "channel-composer" || event.key !== "Enter" || event.isComposing) {
      return;
    }
    var parts = composerParts();
    if (!parts) {
      return;
    }
    var metaEnter = parts.wrapper.dataset.sendShortcut === "meta_enter";
    var sends = metaEnter ? event.ctrlKey || event.metaKey : !event.shiftKey && !event.ctrlKey && !event.metaKey;
    if (sends) {
      event.preventDefault();
      sendMessage(parts);
    }
  });

  document.addEventListener("click", function (event) {
    if (event.target.closest(".chat-composer-button.-send")) {
      var parts = composerParts();
      if (parts) {
        sendMessage(parts);
      }
    }
  });

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
    return teardown;
  });
})();
