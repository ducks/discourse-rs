// DiscourseBanner (components/discourse-banner.gjs) and the banner
// initializer (instance-initializers/banner.js): the site's banner topic
// drawn beside each template.site-banner-slot (macros.html site_banner),
// from its data-config: the preloaded banner, what the component reads of
// the current user and the site, and its strings.
//
// It is drawn while it has a key the viewer hasn't dismissed: the
// member's dismissed_banner_key, else the key the keyValueStore keeps
// (localStorage discourse_dismissed_banner_key). Dismissing it PUTs the
// member's dismissed_banner_key (User#dismissBanner); a visitor's is
// stored, and the banner hidden until the page changes. Staff get the
// edit link to the banner topic. A slot marked data-wrap draws it in a
// container of its own; data-first-post (the topic) only while the
// stream holds the first post (shouldHideScrollableContentAbove).
//
// `/site/banner` (a `banner` event on the page's stream) brings a new
// banner, or null once it is removed, and the banner is drawn again.
(function () {
  "use strict";

  var STORE_KEY = "discourse_dismissed_banner_key";

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' + name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name + '"></use></svg>'
    );
  }

  function storedKey() {
    try {
      return window.localStorage.getItem(STORE_KEY);
    } catch (e) {
      return null;
    }
  }

  function store(key) {
    try {
      window.localStorage.setItem(STORE_KEY, key);
    } catch (e) {
      // A browser that keeps nothing shows the banner on the next page.
    }
  }

  function base() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  // The member's CSRF token and XHR header, from the body's hx-headers.
  function memberHeaders() {
    try {
      return JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      return {};
    }
  }

  function stateOf(slot) {
    if (!slot._siteBanner) {
      var config = {};
      try {
        config = JSON.parse(slot.dataset.config || "{}");
      } catch (e) {
        window.console && console.error("site-banner config", e);
      }
      slot._siteBanner = {
        banner: config.banner || null,
        currentUser: config.currentUser || null,
        desktopView: !!config.desktopView,
        strings: config.strings || {},
        hide: false,
        drawn: null,
      };
    }
    return slot._siteBanner;
  }

  // DiscourseBanner#visible
  function visible(state) {
    var dismissed = (state.currentUser && state.currentUser.dismissed_banner_key) || storedKey();
    var key = state.banner && state.banner.key;
    if (key) {
      key = parseInt(key, 10);
    }
    if (dismissed) {
      dismissed = parseInt(dismissed, 10);
    }
    return !state.hide && key && dismissed !== key;
  }

  // DiscourseBanner#content: the cooked without its headings' ids.
  function content(html) {
    var div = document.createElement("div");
    div.innerHTML = html || "";
    div.querySelectorAll("[id^='heading--']").forEach(function (el) {
      el.removeAttribute("id");
    });
    return div.innerHTML;
  }

  function decorate(el) {
    function run() {
      if (window.Discourse && Discourse.decorateCooked && el.isConnected) {
        Discourse.decorateCooked.apply(el);
      }
    }
    // The decorators register once the page is parsed.
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", run, { once: true });
    } else {
      run();
    }
  }

  function dismiss(slot) {
    var state = stateOf(slot);
    var key = state.banner && state.banner.key;
    if (state.currentUser) {
      state.currentUser.dismissed_banner_key = key;
      var headers = memberHeaders();
      headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      fetch(base() + "/u/" + encodeURIComponent(state.currentUser.username) + ".json", {
        method: "PUT",
        credentials: "same-origin",
        headers: headers,
        body: new URLSearchParams({ dismissed_banner_key: key }).toString(),
      }).catch(function (e) {
        window.console && console.error(e);
      });
    } else {
      state.hide = true;
      store(key);
    }
    draw(slot);
  }

  function draw(slot) {
    var state = stateOf(slot);
    if (state.drawn) {
      state.drawn.remove();
      state.drawn = null;
    }
    if (!visible(state)) {
      return;
    }
    if (slot.hasAttribute("data-first-post") && !document.getElementById("post_1")) {
      return;
    }
    var banner = state.banner;
    var s = state.strings;
    var buttons = "";
    if (state.currentUser && state.currentUser.staff) {
      buttons +=
        '<a class="btn btn-transparent edit-banner" href="' + escapeHtml(banner.url || "") + '"> ' +
        icon("pencil") + " " + (state.desktopView ? s.edit || "" : "") + " </a>";
    }
    var close = escapeHtml(s.close || "");
    buttons +=
      '<button aria-label="' + close + '" class="btn no-text btn-icon btn-transparent close" title="' +
      close + '" type="button">' + icon("xmark") + ' <span aria-hidden="true"> &#8203; </span></button>';
    var html =
      '<div class="container"><div class="row"><div id="banner"><div class="floated-buttons">' +
      buttons + '</div><div id="banner-content">' + content(banner.html) + "</div></div></div></div>";
    if (slot.hasAttribute("data-wrap")) {
      html = '<div class="container">' + html + "</div>";
    }
    var holder = document.createElement("div");
    holder.innerHTML = html;
    var el = holder.firstElementChild;
    el.querySelector("#banner .close").addEventListener("click", function () {
      dismiss(slot);
    });
    slot.after(el);
    state.drawn = el;
    decorate(el.querySelector("#banner-content"));
  }

  function slots() {
    return document.querySelectorAll("template.site-banner-slot");
  }

  // Each slot is drawn once, as its page arrives; the topic's again as its
  // stream loads or drops the first post.
  function watchFirstPost(slot) {
    var posts = document.getElementById("posts");
    if (!posts) {
      return;
    }
    var hadFirst = !!document.getElementById("post_1");
    new MutationObserver(function () {
      var hasFirst = !!document.getElementById("post_1");
      if (slot.isConnected && hasFirst !== hadFirst) {
        hadFirst = hasFirst;
        draw(slot);
      }
    }).observe(posts, { childList: true });
  }

  slots().forEach(function (slot) {
    if (slot._siteBannerInit) {
      return;
    }
    slot._siteBannerInit = true;
    if (!slot.hasAttribute("data-first-post")) {
      draw(slot);
      return;
    }
    // The topic's posts come after the slot.
    var init = function () {
      draw(slot);
      watchFirstPost(slot);
    };
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", init, { once: true });
    } else {
      init();
    }
  });

  if (!Discourse.once("site-banner")) {
    return;
  }

  // BannerInit#onMessage
  document.addEventListener("htmx:sseBeforeMessage", function (event) {
    var message = event.detail;
    if (!message || message.type !== "banner") {
      return;
    }
    event.preventDefault();
    var data = null;
    try {
      data = JSON.parse(message.data);
    } catch (e) {
      return;
    }
    slots().forEach(function (slot) {
      stateOf(slot).banner = data || null;
      draw(slot);
    });
  });
})();
