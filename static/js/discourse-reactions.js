// discourse-reactions on the topic page (DiscourseReactionsActions and the
// components it draws): the reaction button toggles the viewer's reaction
// (their own, or the main one), hovering it opens the picker of reactions,
// and the counter opens who reacted (DiscourseReactionsUsersMenu over
// core's UsersPopup). The post comes back redrawn over the live stream
// (publish_change_to_clients :acted). An anonymous reader is sent to log
// in. Ember's animations and touch gestures are not ported.
(function () {
  "use strict";

  if (!Discourse.once("discourse-reactions")) {
    return;
  }

  var COLLAPSE_DELAY = 500;
  var PAGE_SIZE = 30;

  function csrfHeaders() {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    headers["X-Requested-With"] = "XMLHttpRequest";
    return headers;
  }

  function escapeHtml(s) {
    var div = document.createElement("div");
    div.textContent = s == null ? "" : String(s);
    return div.innerHTML;
  }

  // CustomReaction.toggle; errors as DiscourseReactionsActions#_extractErrors
  // shows them.
  function toggle(actions, reaction) {
    var base = actions.dataset.basePath || "";
    var url =
      base +
      "/discourse-reactions/posts/" +
      actions.dataset.postId +
      "/custom-reactions/" +
      encodeURIComponent(reaction) +
      "/toggle.json";
    return fetch(url, {
      method: "PUT",
      credentials: "same-origin",
      headers: csrfHeaders(),
    }).then(function (r) {
      if (r.ok) {
        return;
      }
      return r
        .json()
        .then(function (body) {
          window.alert((body.errors && body.errors[0]) || r.statusText);
        })
        .catch(function () {
          window.alert(r.statusText);
        });
    });
  }

  // The reaction button's click (toggleFromButton) and a picked reaction
  // (toggle): only where the viewer may change their reaction.
  function react(actions, reaction) {
    if (actions.dataset.reactionLoginUrl) {
      if (actions.classList.contains("can-toggle-reaction")) {
        location.href = actions.dataset.reactionLoginUrl;
      }
      return;
    }
    if (!actions.classList.contains("can-toggle-reaction")) {
      return;
    }
    collapsePicker();
    toggle(actions, reaction);
  }

  // The picker: one at a time, floated above the reaction button.
  var picker = null;
  var collapseTimer = null;

  function cancelCollapse() {
    clearTimeout(collapseTimer);
    collapseTimer = null;
  }

  function scheduleCollapse() {
    cancelCollapse();
    collapseTimer = setTimeout(collapsePicker, COLLAPSE_DELAY);
  }

  function collapsePicker() {
    cancelCollapse();
    if (picker) {
      picker.remove();
      picker = null;
    }
  }

  function expandPicker(actions) {
    cancelCollapse();
    if (picker && picker.parentElement === actions) {
      return;
    }
    collapsePicker();
    var info = JSON.parse(actions.dataset.picker);
    var html = info.reactions
      .map(function (r) {
        var classes = ["btn", "no-text", "btn-icon", "btn-flat", "pickable-reaction", r.id];
        if (r.canUndo) {
          classes.push("can-undo");
        }
        if (r.isUsed) {
          classes.push("is-used");
        }
        return (
          '<button class="' + escapeHtml(classes.join(" ")) + '" data-reaction="' +
          escapeHtml(r.id) + '" title="' + escapeHtml(r.title) + '" type="button">' +
          r.html + "</button>"
        );
      })
      .join("");
    picker = document.createElement("div");
    picker.className = "discourse-reactions-picker is-expanded";
    picker.innerHTML =
      '<div class="discourse-reactions-picker-container col-' + info.cols + '">' + html + "</div>";
    actions.prepend(picker);
    // computePosition(button, picker, { placement: "top", offset(-5), shift,
    // flip }), relative to the picker's offset parent.
    var button = actions.querySelector(".discourse-reactions-reaction-button");
    var parent = picker.offsetParent || document.body;
    var b = button.getBoundingClientRect();
    var p = parent.getBoundingClientRect();
    var w = picker.offsetWidth;
    var h = picker.offsetHeight;
    var left = b.left + b.width / 2 - w / 2;
    left = Math.max(0, Math.min(left, document.documentElement.clientWidth - w));
    var top = b.top - h + 5;
    if (top < 5) {
      top = b.bottom - 5;
    }
    picker.style.left = left - p.left + parent.scrollLeft + "px";
    picker.style.top = top - p.top + parent.scrollTop + "px";
  }

  document.addEventListener("pointerover", function (event) {
    if (event.pointerType !== "mouse") {
      return;
    }
    if (picker && event.target.closest(".discourse-reactions-picker") === picker) {
      cancelCollapse();
      return;
    }
    var button = event.target.closest(".discourse-reactions-reaction-button");
    if (!button) {
      return;
    }
    var actions = button.closest(".discourse-reactions-actions");
    // ReactionsReactionButton#pointerOver: anonymous readers outside
    // archived topics, members who can toggle and undo.
    if (!actions.dataset.picker || !actions.classList.contains("can-toggle-reaction")) {
      return;
    }
    expandPicker(actions);
  });

  document.addEventListener("pointerout", function (event) {
    if (event.pointerType !== "mouse" || !picker) {
      return;
    }
    var from = event.target.closest(".discourse-reactions-reaction-button, .discourse-reactions-picker");
    var to = event.relatedTarget && event.relatedTarget.closest
      ? event.relatedTarget.closest(".discourse-reactions-reaction-button, .discourse-reactions-picker")
      : null;
    if (from && from !== to) {
      scheduleCollapse();
    }
  });

  // The users menu (DMenu, placement bottom), one at a time.
  var menu = null;

  function closeMenu() {
    if (!menu) {
      return;
    }
    menu.trigger.setAttribute("aria-expanded", "false");
    menu.content.remove();
    menu = null;
  }

  function avatarUrl(config, template) {
    return template.replace("{size}", String(config.avatarSize));
  }

  function userRow(config, user) {
    var name = user.name && !config.prioritizeUsername ? user.name : user.username;
    var href = config.basePath + "/u/" + encodeURIComponent(user.username.toLowerCase());
    var reaction = user.reaction
      ? emojiImg(config, user.reaction, "emoji users-popup__reaction")
      : config.likedIcon;
    return (
      '<div class="users-popup__item">' +
      '<a class="trigger-user-card" data-user-card="' + escapeHtml(user.username) + '" href="' +
      escapeHtml(href) + '" aria-hidden="true" tabindex="-1">' +
      '<img loading="lazy" alt="" width="24" height="24" src="' +
      escapeHtml(avatarUrl(config, user.avatar_template)) +
      '" class="avatar"></a>' +
      '<div class="users-popup__user-info">' +
      '<a class="users-popup__name" data-user-card="' + escapeHtml(user.username) + '" href="' +
      escapeHtml(href) + '">' + escapeHtml(name) + "</a>" +
      (config.prioritizeUsername
        ? ""
        : '<a aria-hidden="true" class="users-popup__username" data-user-card="' +
          escapeHtml(user.username) + '" href="' + escapeHtml(href) + '">@' +
          escapeHtml(user.username) + "</a>") +
      "</div>" + reaction + "</div>"
    );
  }

  function emojiImg(config, name, cls) {
    var path = name.replace(/:t(\d)$/, "/$1");
    var src =
      config.basePath + "/images/emoji/" + config.emoji.set + "/" + path + ".png?v=" +
      config.emoji.version;
    return (
      '<img width="20" height="20" src="' + escapeHtml(src) + '" title="' + escapeHtml(name) +
      '" alt="' + escapeHtml(name) + '" class="' + cls + '">'
    );
  }

  function openUsersMenu(trigger) {
    closeMenu();
    var config = JSON.parse(trigger.dataset.usersMenu);
    var portals = document.getElementById("d-menu-portals");
    if (!portals) {
      portals = document.createElement("div");
      portals.id = "d-menu-portals";
      document.body.appendChild(portals);
    }
    var header = "";
    if (config.filters.length) {
      header =
        '<div class="users-popup__header"><button class="users-popup__filter is-active" ' +
        'data-reaction-filter="all" type="button">' + escapeHtml(config.all) + "</button>" +
        config.filters
          .map(function (f) {
            return (
              '<button class="users-popup__filter" data-reaction-filter="' + escapeHtml(f.id) +
              '" type="button">' + f.html + "<span>" + escapeHtml(f.count) + "</span></button>"
            );
          })
          .join("") +
        "</div>";
    }
    var content = document.createElement("div");
    content.className =
      "fk-d-menu discourse-reactions-users-menu-content -animated -expanded";
    content.setAttribute("role", "dialog");
    content.dataset.identifier = "discourse-reactions-users-menu";
    content.dataset.content = "";
    content.dataset.strategy = "absolute";
    content.dataset.placement = "bottom";
    content.innerHTML =
      '<div class="fk-d-menu__inner-content"><div class="users-popup" tabindex="-1">' +
      '<div class="users-popup__sticky-header">' + header + "</div>" +
      '<div class="users-popup__body"></div></div></div>';
    portals.appendChild(content);
    trigger.setAttribute("aria-expanded", "true");
    menu = {
      trigger: trigger,
      content: content,
      config: config,
      filter: null,
      page: 0,
      loading: false,
      more: true,
    };
    place(trigger, content);
    content.querySelector(".users-popup").focus({ preventScroll: true });
    loadMore();
  }

  // Below the trigger, 15px off, centred and kept on the page.
  function place(trigger, content) {
    var t = trigger.getBoundingClientRect();
    var w = content.offsetWidth;
    var left = t.left + t.width / 2 - w / 2;
    left = Math.max(5, Math.min(left, document.documentElement.clientWidth - w - 5));
    content.style.position = "absolute";
    content.style.left = left + window.scrollX + "px";
    content.style.top = t.bottom + window.scrollY + 15 + "px";
  }

  // UsersPopup#loadMore over CustomReaction.fetchReactionsUsersList.
  function loadMore() {
    var m = menu;
    if (!m || m.loading || !m.more) {
      return;
    }
    m.loading = true;
    var params = new URLSearchParams({ page: String(m.page), limit: String(PAGE_SIZE) });
    if (m.filter) {
      params.set("reaction_value", m.filter);
    }
    var filter = m.filter;
    fetch(m.config.url + "?" + params.toString(), {
      credentials: "same-origin",
      headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
    })
      .then(function (r) {
        return r.json();
      })
      .then(function (result) {
        if (menu !== m || m.filter !== filter) {
          return;
        }
        var users = result.users || [];
        var offset = m.page * PAGE_SIZE;
        m.more = result.total_rows
          ? offset + users.length < result.total_rows
          : users.length >= PAGE_SIZE;
        m.page++;
        var body = m.content.querySelector(".users-popup__body");
        body.insertAdjacentHTML(
          "beforeend",
          users
            .map(function (u) {
              return userRow(m.config, u);
            })
            .join("")
        );
      })
      .finally(function () {
        m.loading = false;
      });
  }

  document.addEventListener(
    "scroll",
    function (event) {
      var body = event.target.closest && event.target.closest(".users-popup__body");
      if (body && menu && body.scrollTop + body.clientHeight >= body.scrollHeight - 100) {
        loadMore();
      }
    },
    true
  );

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape" && menu) {
      var trigger = menu.trigger;
      closeMenu();
      trigger.focus();
    }
  });

  document.addEventListener("click", function (event) {
    if (menu) {
      var filter = event.target.closest(".users-popup__filter");
      if (filter && menu.content.contains(filter)) {
        event.preventDefault();
        var id = filter.dataset.reactionFilter === "all" ? null : filter.dataset.reactionFilter;
        if (id !== menu.filter) {
          menu.content.querySelectorAll(".users-popup__filter").forEach(function (b) {
            b.classList.toggle("is-active", b === filter);
          });
          menu.filter = id;
          menu.page = 0;
          menu.more = true;
          menu.loading = false;
          var body = menu.content.querySelector(".users-popup__body");
          body.innerHTML = "";
          body.scrollTop = 0;
          loadMore();
        }
        return;
      }
      if (!menu.content.contains(event.target)) {
        var same = event.target.closest(".discourse-reactions-counter") === menu.trigger;
        closeMenu();
        if (same) {
          return;
        }
      } else {
        return;
      }
    }
    var counter = event.target.closest(".discourse-reactions-counter");
    if (counter) {
      event.preventDefault();
      openUsersMenu(counter);
      return;
    }
    var pickable = event.target.closest(".pickable-reaction");
    if (pickable && picker && picker.contains(pickable)) {
      react(picker.parentElement, pickable.dataset.reaction);
      return;
    }
    var button = event.target.closest(".discourse-reactions-reaction-button");
    if (button) {
      var actions = button.closest(".discourse-reactions-actions");
      react(actions, actions.dataset.currentReaction || actions.dataset.mainReaction);
      return;
    }
    if (picker && !event.target.closest(".discourse-reactions-actions")) {
      collapsePicker();
    }
  });

  window.addEventListener(
    "scroll",
    function () {
      // closeOnScroll
      closeMenu();
    },
    { passive: true }
  );
  // A page swapped out takes its open picker and menu with it (page.js).
  Discourse.onPage(function () {
    return function () {
      collapsePicker();
      closeMenu();
    };
  });
})();
