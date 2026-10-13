// The user card (components/card-contents-base.js and
// user-card-contents.gjs): a click on an avatar or name carrying
// data-user-card, or on an @mention, opens the card by it in a float-kit
// menu (Discourse.float.menu, identifier "usercard"), first as the
// loading placeholder, then with the card the server renders
// (GET /user-card/:username, users#show_card's document). A click outside
// it or Escape closes it, and so does clicking the same target again.
// The layout leaves this script out where Ember's card would not open
// (hide_user_profiles_from_public for an anonymous visitor).
(function () {
  "use strict";

  if (!Discourse.once("user-card")) {
    return;
  }

  // DEFAULT_SELECTORS: where clicks open the card.
  var LISTEN_IN = ["#main-outlet", "#d-menu-portals", ".modal-container"];
  var AVATAR_OVERFLOW_SIZE = 44;

  var card = null; // { target, username, element, menu, loading }
  // showBadges (enable_badges), from the layout's script tag.
  var script = document.currentScript;
  var showBadges = !script || script.dataset.showBadges !== "false";

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

  function placeholder(username) {
    var element = document.createElement("div");
    // The attributes in the order Ember renders them.
    element.setAttribute("aria-label", "User card");
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
    if (closing.menu) {
      closing.menu.destroy();
    }
  }

  // _positionCard(): the desktop menu's options.
  function position(state) {
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
    var url = basePath() + "/user-card/" + encodeURIComponent(state.username);
    var topic = topicId();
    if (topic) {
      url += "?include_post_count_for=" + encodeURIComponent(topic);
    }
    fetch(url, { credentials: "same-origin", headers: { "X-Requested-With": "XMLHttpRequest" } })
      .then(function (response) {
        if (!response.ok) {
          throw new Error("user card " + response.status);
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
  function show(username, target) {
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
    var state = { target: target, username: username, element: placeholder(username), loading: true, menu: null };
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

  // _cardClickHandler: an avatar ([data-user-card]) first, else a mention.
  // In the capture phase, so the card is opened before the boosted link
  // would navigate.
  document.addEventListener(
    "click",
    function (event) {
      if (!(event.target instanceof Element) || !listening(event.target)) {
        return;
      }
      var avatar = event.target.closest("[data-user-card]");
      var mention = avatar ? null : event.target.closest("a.mention");
      var matched = avatar || mention;
      if (!matched || wantsNewWindow(event)) {
        return;
      }
      var username = avatar ? avatar.dataset.userCard : matched.innerText.trim().replace(/^@/, "");
      if (!show(username, matched)) {
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
    var avatar = event.target.closest("[data-user-card]");
    if ((avatar && avatar.getAttribute("data-user-card")) || event.target.closest("#user-card")) {
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

  // dom:clean: a new page closes it.
  Discourse.onPage(function () {
    return close;
  });
})();
