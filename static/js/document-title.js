// Ember's document-title service for a member: the tab title's count
// prefix, "(N) title", or with the dynamic_favicon option the count drawn
// on the favicon instead (lib/update-tab-count.js, based on Favcount by
// Chris Hunt, Copyright 2013 Chris Hunt, MIT License).
//
// The count is one of two, by the member's title_count_mode option:
// - notifications: what d-document's _updateNotifications sums as the
//   notification state changes while the tab is hidden
//   (all_unread_notifications_count and unseen_reviewable_count, read off
//   the header avatar the live stream redraws, crate::header_badge),
//   back to 0 as the tab is shown again;
// - contextual: the latest list's "N new or updated topics"
//   (DiscoveryTopicsList's incomingCount), or the posts others made in
//   the topic being viewed while the tab was hidden (the topic
//   controller's incrementBackgroundContextCount, a `post` event ending
//   in the stream's created-by-other marker), which showing the tab
//   clears. Each page starts at 0 (clean-dom-on-route-change).
// No count shows in the title during do not disturb.
//
// The page's own title is whatever anything else writes to <title> (the
// server's with each page, the title editor): the prefix is put on again
// after each write. Scripts wanting the page's title ask for it:
//
//   Discourse.documentTitle.getTitle()            without the count
//   Discourse.documentTitle.updateContextCount(n)
(function () {
  "use strict";

  if (!Discourse.once("document-title")) {
    return;
  }

  var script = document.currentScript;
  var contextual = script.dataset.titleCountMode === "contextual";
  var dynamicFavicon = script.hasAttribute("data-dynamic-favicon");
  var CREATED_BY_OTHER = "<template data-created-by-other></template>";

  var title = document.title;
  // document.title as last written here.
  var rendered = title;
  var hasFocus = true;
  var contextCount = 0;
  var notificationCount = 0;
  var backgroundNotify = false;

  // The current user's fields, as the header avatar carries them.
  function currentUser() {
    var avatar = document.querySelector("#toggle-current-user > img.avatar");
    var data = avatar ? avatar.dataset : {};
    return {
      allUnread: parseInt(data.allUnread, 10) || 0,
      unseenReviewables: parseInt(data.unseenReviewables, 10) || 0,
      doNotDisturb: "doNotDisturb" in data,
    };
  }

  function displayCount() {
    return contextual ? contextCount : notificationCount;
  }

  // A title written by anything else is the page's new one (setTitle).
  function getTitle() {
    if (document.title !== rendered) {
      title = document.title;
    }
    return title;
  }

  function renderTitle() {
    var text = getTitle();
    var count = displayCount();
    if (count > 0 && !dynamicFavicon && !currentUser().doNotDisturb) {
      text = "(" + count + ") " + text;
    }
    if (text !== rendered) {
      document.title = text;
    }
    rendered = document.title;
  }

  // update-tab-count's renderIcon.
  function renderIcon(canvas, img, count) {
    count = Math.round(count);
    if (isNaN(count) || count < 1) {
      count = "";
    } else if (count < 10) {
      count = " " + count;
    } else if (count > 99) {
      count = "99";
    }

    // Scale canvas elements based on favicon size
    var multiplier = img.width / 16;
    var fontSize = multiplier * 11;
    var xOffset = multiplier;
    var shadow = multiplier * 2;

    canvas.height = canvas.width = img.width;
    var ctx = canvas.getContext("2d");
    ctx.font = "bold " + fontSize + "px Arial, sans-serif";

    if (count) {
      ctx.globalAlpha = 0.4;
    }
    ctx.drawImage(img, 0, 0);
    ctx.globalAlpha = 1.0;

    // Draw white drop shadow
    ctx.shadowColor = "#FFF";
    ctx.shadowBlur = shadow;
    ctx.shadowOffsetX = 0;
    ctx.shadowOffsetY = 0;

    // Draw white border
    ctx.fillStyle = "#FFF";
    ctx.fillText(count, xOffset, fontSize);
    ctx.fillText(count, xOffset + multiplier, fontSize);
    ctx.fillText(count, xOffset, fontSize + multiplier);
    ctx.fillText(count, xOffset + multiplier, fontSize + multiplier);

    // Draw black count
    ctx.fillStyle = "#000";
    ctx.fillText(count, xOffset + multiplier / 2.0, fontSize + multiplier / 2.0);

    // Replace favicon with new favicon
    var newFavicon = document.createElement("link");
    newFavicon.rel = "icon";
    newFavicon.href = canvas.toDataURL("image/png");
    var favicon = document.querySelector("link[rel=icon]");
    if (favicon) {
      favicon.remove();
    }
    document.head.appendChild(newFavicon);
  }

  // site_favicon_url, as the page links it. Ember asks for a remote one
  // through /favicon/proxied; here it is asked for where it is.
  var faviconLink = document.querySelector("link[rel=icon]");
  var faviconUrl = faviconLink && faviconLink.href;

  function renderFavicon() {
    if (!dynamicFavicon || !faviconUrl) {
      return;
    }
    var count = displayCount();
    var canvas = document.createElement("canvas");
    var img = document.createElement("img");
    img.crossOrigin = "anonymous";
    img.onload = function () {
      renderIcon(canvas, img, count);
    };
    img.src = faviconUrl;
  }

  function updateContextCount(count) {
    contextCount = count;
    renderTitle();
  }

  function updateNotificationCount(count) {
    if (!hasFocus) {
      notificationCount = count;
      renderFavicon();
      renderTitle();
    }
  }

  function incrementBackgroundContextCount() {
    if (!hasFocus) {
      backgroundNotify = true;
      contextCount += 1;
      renderFavicon();
      renderTitle();
    }
  }

  function setFocus(focus) {
    hasFocus = focus;
    if (hasFocus && backgroundNotify) {
      updateContextCount(0);
    }
    backgroundNotify = false;
    if (hasFocus) {
      notificationCount = 0;
    }
    renderFavicon();
    renderTitle();
  }

  // d-document's _focusChanged.
  function focusChanged() {
    if (document.visibilityState === "hidden") {
      if (hasFocus) {
        setFocus(false);
      }
    } else if (!hasFocus) {
      setFocus(true);
    }
  }
  document.addEventListener("visibilitychange", focusChanged);
  document.addEventListener("resume", focusChanged);
  document.addEventListener("freeze", focusChanged);

  new MutationObserver(renderTitle).observe(document.querySelector("title"), {
    childList: true,
    characterData: true,
    subtree: true,
  });

  // The stream's redraws. onNotification's notifications:changed, when
  // all_unread_notifications_count is another; the title drawn again
  // either way, for do not disturb starting or ending. The latest list's
  // banner: incomingCount's observer, when it is another.
  var allUnread = currentUser().allUnread;
  var incoming = 0;
  document.addEventListener("htmx:oobAfterSwap", function () {
    var user = currentUser();
    if (user.allUnread !== allUnread) {
      allUnread = user.allUnread;
      updateNotificationCount(user.allUnread + user.unseenReviewables);
    }
    var banner = document.querySelector("#list-updates > a[data-count]");
    var count = banner ? parseInt(banner.dataset.count, 10) || 0 : 0;
    if (count !== incoming) {
      incoming = count;
      updateContextCount(count);
    }
    renderTitle();
  });

  document.addEventListener("htmx:sseBeforeMessage", function (event) {
    var message = event.detail;
    if (
      message &&
      message.type === "post" &&
      typeof message.data === "string" &&
      message.data.slice(-CREATED_BY_OTHER.length) === CREATED_BY_OTHER
    ) {
      incrementBackgroundContextCount();
    }
  });

  // clean-dom-on-route-change, and the new list's tracking state.
  Discourse.onPage(function () {
    incoming = 0;
    updateContextCount(0);
  });

  Discourse.documentTitle = {
    getTitle: getTitle,
    updateContextCount: updateContextCount,
  };
})();
