// The user card (components/card-contents-base.js and
// user-card-contents.gjs): a click on an avatar or name carrying
// data-user-card, or on an @mention, opens the card by it in a float-kit
// menu (Discourse.float.menu, identifier "usercard"), first as the
// loading placeholder, then with the card the server renders
// (GET /user-card/:username, users#show_card's document). A click outside
// it or Escape closes it, and so does clicking the same target again.
// The layout leaves this script out where Ember's card would not open
// (hide_user_profiles_from_public for an anonymous visitor).
//
// The group card (group-card-contents.gjs, the same base): a click on a
// group's name carrying data-group-card or on a group mention
// (a.mention-group) opens it the same way, with the card the server
// renders (GET /group-card/:name, what Ember builds from
// /groups/:name.json and its first ten members). Its Join and Leave are
// GroupMembershipButton's and its Message the composer's openNewMessage.
(function () {
  "use strict";

  if (!Discourse.once("user-card")) {
    return;
  }

  // DEFAULT_SELECTORS: where clicks open the card.
  var LISTEN_IN = ["#main-outlet", "#d-menu-portals", ".modal-container"];
  var AVATAR_OVERFLOW_SIZE = 44;

  var card = null; // { kind: "user" | "group", target, username, element, menu, loading }
  // showBadges (enable_badges), from the layout's script tag.
  var script = document.currentScript;
  var showBadges = !script || script.dataset.showBadges !== "false";
  // The card's label (`user.card`, site texts included).
  var cardLabel = (script && script.dataset.label) || "User card";

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  // lib/intercept-click.js wantsNewWindow()
  function wantsNewWindow(event) {
    return (
      event.defaultPrevented ||
      event.shiftKey ||
      event.metaKey ||
      event.ctrlKey ||
      (event.button && event.button !== 0) ||
      (event.currentTarget && event.currentTarget.target === "_blank")
    );
  }

  // viewingTopic: the topic page's id, for include_post_count_for.
  function topicId() {
    var topic = document.getElementById("topic");
    return topic ? topic.dataset.topicId : null;
  }

  function placeholder(kind, username) {
    var element = document.createElement("div");
    if (kind === "group") {
      // GroupCardContents while loading: no group, so no group class.
      element.id = "group-card";
      element.className = "no-bg group-card show" + (showBadges ? " show-badges" : "");
      element.innerHTML =
        '<div class="card-content"><div class="card-row first-row"><div class="group-card-avatar">' +
        '<div class="card-avatar-placeholder animated-placeholder placeholder-animation"></div>' +
        '</div></div><div class="card-row second-row"><div class="animated-placeholder placeholder-animation"></div></div></div>';
      return element;
    }
    // The attributes in the order Ember renders them.
    element.setAttribute("aria-label", cardLabel);
    element.id = "user-card";
    element.className =
      "user-card show" +
      (showBadges ? " show-badges" : "") +
      " no-bg user-card-" + username + " group-undefined";
    element.setAttribute("role", "dialog");
    var row = '<div class="card-row"><div class="animated-placeholder placeholder-animation"></div></div>';
    element.innerHTML =
      '<div class="card-content"><div class="card-row first-row"><div class="user-card-avatar">' +
      '<div class="card-avatar-placeholder animated-placeholder placeholder-animation"></div>' +
      '</div></div><div class="card-row second-row"><div class="animated-placeholder placeholder-animation"></div></div>' +
      row + row + row + "</div>";
    return element;
  }

  // _close(): the card's state goes, then the menu (_hide).
  function close() {
    if (!card) {
      return;
    }
    var closing = card;
    card = null;
    if (closing.onScroll) {
      window.removeEventListener("scroll", closing.onScroll);
    }
    var veil = document.querySelector(".card-cloak");
    if (veil) {
      veil.classList.remove("--visible");
    }
    if (closing.menu) {
      closing.menu.destroy();
    }
  }

  function mobileView() {
    return document.documentElement.classList.contains("mobile-view");
  }

  // CardContainer's cloak in the mobile view, behind the card.
  function cloak() {
    var el = document.querySelector(".card-cloak");
    if (!el && mobileView()) {
      var outlet = document.getElementById("main-outlet");
      if (outlet) {
        el = document.createElement("div");
        el.className = "card-cloak";
        outlet.appendChild(el);
      }
    }
    return el;
  }

  // _positionCard(): the desktop menu's options; in the mobile view the
  // card is fixed across the top of the screen over the cloak, and the
  // page's scroll closes it (_bindMobileScroll).
  function position(state) {
    if (mobileView()) {
      var veil = cloak();
      if (veil) {
        veil.classList.add("--visible");
      }
      state.menu = Discourse.float.menu(state.target, {
        content: state.element,
        strategy: "fixed",
        identifier: "usercard",
        computePosition: function (content) {
          content.style.left = "10px";
          content.style.right = "10px";
          content.style.top = 10 + AVATAR_OVERFLOW_SIZE + "px";
        },
      });
      var lastScroll = 0;
      state.onScroll = function () {
        // throttle(this, this._close, 1000)
        var now = Date.now();
        if (now - lastScroll >= 1000) {
          lastScroll = now;
          setTimeout(close, 0);
        }
      };
      window.addEventListener("scroll", state.onScroll, { passive: true });
      setTimeout(function () {
        if (card === state) {
          state.element.setAttribute("tabindex", "-1");
          state.element.focus({ preventScroll: true });
        }
      }, 350);
      return;
    }
    var headerOffset = Discourse.float.headerOffset();
    state.menu = Discourse.float.menu(state.target, {
      content: state.element,
      autoUpdate: { ancestorScroll: false, layoutShift: false },
      identifier: "usercard",
      padding: {
        top: 10 + AVATAR_OVERFLOW_SIZE + headerOffset,
        right: 10,
        bottom: 10,
        left: 10,
      },
      maxWidth: "unset",
      fallbackPlacements: ["right-start", "left-start", "top-start", "bottom-start"],
      crossAxisShift: false,
      shiftBeforeVisibilityOptimizer: true,
    });
    // Focus the card once it is placed.
    setTimeout(function () {
      if (card === state) {
        state.element.setAttribute("tabindex", "-1");
        state.element.focus();
      }
    }, 350);
  }

  function load(state) {
    var url = basePath() + "/" + state.kind + "-card/" + encodeURIComponent(state.username);
    var topic = state.kind === "user" && topicId();
    if (topic) {
      url += "?include_post_count_for=" + encodeURIComponent(topic);
    }
    fetch(url, { credentials: "same-origin", headers: { "X-Requested-With": "XMLHttpRequest" } })
      .then(function (response) {
        if (!response.ok) {
          throw new Error(state.kind + " card " + response.status);
        }
        return response.text();
      })
      .then(function (html) {
        if (card !== state) {
          return;
        }
        var holder = document.createElement("div");
        holder.innerHTML = html;
        var loaded = holder.firstElementChild;
        // The element stays (it is the menu's content and may have
        // focus); it takes the loaded card's attributes and contents.
        // The group card is loaded again after a join or a leave, with
        // other attributes.
        ["data-group-name", "data-confirm-leave"].forEach(function (name) {
          state.element.removeAttribute(name);
        });
        Array.prototype.slice.call(loaded.attributes).forEach(function (attr) {
          state.element.setAttribute(attr.name, attr.value);
        });
        state.element.innerHTML = loaded.innerHTML;
        state.loading = false;
      })
      .catch(function () {
        if (card === state) {
          close();
        }
      });
  }

  // _show(): false when the click is the card's to handle.
  function show(kind, username, target) {
    // Don't show if nested: a link in the card navigates.
    if (target.closest(".card-content")) {
      close();
      return true;
    }
    if (card) {
      var same = card.target === target;
      close();
      if (same) {
        return false;
      }
    }
    var state = { kind: kind, target: target, username: username, element: placeholder(kind, username), loading: true, menu: null };
    card = state;
    // Ember positions in next(): after the click is done.
    setTimeout(function () {
      if (card !== state) {
        return;
      }
      position(state);
      load(state);
    });
    return false;
  }

  function listening(target) {
    return LISTEN_IN.some(function (selector) {
      var region = target.closest(selector);
      return region !== null;
    });
  }

  // _cardClickHandler: an avatar ([data-user-card]) first, else a mention;
  // then the group card's, a name ([data-group-card]) first, else a group
  // mention. In the capture phase, so the card is opened before the
  // boosted link would navigate.
  document.addEventListener(
    "click",
    function (event) {
      if (!(event.target instanceof Element) || !listening(event.target)) {
        return;
      }
      var avatar = event.target.closest("[data-user-card]");
      var mention = avatar ? null : event.target.closest("a.mention");
      var matched = avatar || mention;
      var kind = "user";
      if (!matched) {
        avatar = event.target.closest("[data-group-card]");
        matched = avatar || event.target.closest("a.mention-group");
        kind = "group";
      }
      if (!matched || wantsNewWindow(event)) {
        return;
      }
      var username = avatar
        ? avatar.getAttribute("data-" + kind + "-card")
        : matched.innerText.trim().replace(/^@/, "");
      if (!show(kind, username, matched)) {
        event.preventDefault();
        event.stopPropagation();
      }
    },
    true
  );

  // _clickOutsideHandler
  document.addEventListener("pointerdown", function (event) {
    if (!card || !(event.target instanceof Element)) {
      return;
    }
    var attr = "data-" + card.kind + "-card";
    var avatar = event.target.closest("[" + attr + "]");
    if ((avatar && avatar.getAttribute(attr)) || event.target.closest("#" + card.kind + "-card")) {
      return;
    }
    close();
  });

  // _escListener
  document.addEventListener("keyup", function (event) {
    if (card && event.key === "Escape") {
      var target = card.target;
      target.focus();
      close();
    }
  });

  // composePM(user, post): the post the card was opened on, when it was
  // opened on one, as its url and topic title.
  function cardPost(target) {
    var article = target && target.closest && target.closest("article[data-post-id]");
    var number = article && article.parentElement && article.parentElement.dataset.postNumber;
    var topic = location.pathname.slice(basePath().length).match(/^\/t\/([^/]+)\/(\d+)/);
    if (!number || !topic) {
      return null;
    }
    var title = document.querySelector("#topic-title .fancy-title");
    return {
      url: basePath() + "/t/" + topic[1] + "/" + topic[2] + (parseInt(number, 10) > 1 ? "/" + number : ""),
      topicTitle: title ? title.textContent.trim() : "",
    };
  }

  // The card's Message button: the message composer to its user (the
  // application route's composePrivateMessage), the card closed.
  document.addEventListener("click", function (event) {
    if (!card || !(event.target instanceof Element)) {
      return;
    }
    var button = event.target.closest("#user-card .compose-pm button");
    if (!button || !window.composer || !window.composer.composePrivateMessage) {
      return;
    }
    var username = card.username;
    var post = cardPost(card.target);
    close();
    window.composer.composePrivateMessage(username, post);
  });

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

  // popupAjaxError
  function ajaxError(response) {
    return response
      .json()
      .then(function (body) {
        return (body.errors || [response.statusText]).join("\n");
      })
      .catch(function () {
        return response.statusText;
      })
      .then(function (text) {
        if (Discourse.dialog) {
          Discourse.dialog.popupAjaxError(text);
        } else {
          window.alert(text);
        }
      });
  }

  // group.join() and group.leave(), then the card with the group as it
  // is now (is_group_user and reloadMembers).
  function membership(state, button, method, path) {
    button.disabled = true;
    fetch(basePath() + "/groups/" + encodeURIComponent(button.dataset.groupId) + path, {
      method: method,
      credentials: "same-origin",
      headers: csrfHeaders(),
    })
      .then(function (response) {
        if (!response.ok) {
          return ajaxError(response);
        }
        if (card === state) {
          load(state);
        }
      })
      .finally(function () {
        button.disabled = false;
      });
  }

  // The group card's buttons and links. In the capture phase and stopped
  // there: the groups pages' own handlers (groups.js) take the same
  // classes and draw the page again.
  document.addEventListener(
    "click",
    function (event) {
      if (!card || card.kind !== "group" || !(event.target instanceof Element)) {
        return;
      }
      var el = event.target.closest("#group-card button, #group-card a.card-tiny-avatar");
      if (!el) {
        return;
      }
      var state = card;
      var name = state.element.dataset.groupName;
      event.preventDefault();
      event.stopPropagation();
      if (el.matches("a.card-tiny-avatar")) {
        // close(): the click is prevented, the card goes.
        close();
      } else if (el.matches(".group-index-join, .group-index-request") && !document.body.getAttribute("hx-headers")) {
        // showLogin
        window.location.assign(basePath() + "/login");
      } else if (el.matches(".group-index-join")) {
        membership(state, el, "PUT", "/join.json");
      } else if (el.matches(".group-index-leave")) {
        // leaveGroup: asked first unless anyone may join again.
        var confirmLeave = state.element.dataset.confirmLeave;
        if (!confirmLeave) {
          membership(state, el, "DELETE", "/leave.json");
        } else if (Discourse.dialog) {
          Discourse.dialog.yesNoConfirm({
            message: confirmLeave,
            didConfirm: function () {
              membership(state, el, "DELETE", "/leave.json");
            },
          });
        } else if (window.confirm(confirmLeave)) {
          membership(state, el, "DELETE", "/leave.json");
        }
      } else if (el.matches(".group-index-request")) {
        // Not ported: RequestGroupMembershipForm. The group's page.
        window.location.assign(basePath() + "/g/" + encodeURIComponent(name));
      } else if (el.matches(".group-message-button") && window.composer) {
        // messageGroup: composer.openNewMessage, the card left open.
        window.composer.open({ mode: "private-message", recipients: [name], hasGroups: true });
      }
    },
    true
  );

  // topic-header:trigger-user-card and topic-header:trigger-group-card:
  // the docked header's participants open the card by their avatar or
  // name (static/js/header-topic-info.js).
  Discourse.showUserCard = function (username, target) {
    show("user", username, target);
  };
  Discourse.showGroupCard = function (name, target) {
    show("group", name, target);
  };
  // card:close, from chat-new-message.js's Chat button.
  Discourse.closeUserCard = close;

  // dom:clean: a new page closes it.
  Discourse.onPage(function () {
    return close;
  });
})();
