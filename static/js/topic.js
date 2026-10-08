// The topic page's client side: the buttons that act in the browser (copy
// link, share, show more, an anonymous reader's reply), live updates
// arriving twice, and the timeline, which follows
// the post being read as Ember's does (components/topic-timeline,
// modifiers/post-stream-viewport-tracker).
(function () {
  "use strict";

  // The notification level menu (DMenu): one for the page, moved to the
  // body and floated under the trigger that opened it, bottom-start, 10px
  // below as float-kit's offset puts it.
  var trackingMenu = document.querySelector(".notifications-tracking-content");
  var trackingTrigger = null;

  function closeTrackingMenu() {
    if (!trackingTrigger) {
      return;
    }
    trackingMenu.hidden = true;
    trackingMenu.classList.remove("-expanded");
    trackingMenu.classList.remove("topic-timeline-notifications-tracking-content");
    trackingTrigger.setAttribute("aria-expanded", "false");
    trackingTrigger = null;
  }

  function openTrackingMenu(trigger) {
    if (trackingMenu.parentElement !== document.body) {
      document.body.appendChild(trackingMenu);
    }
    var rect = trigger.getBoundingClientRect();
    trackingMenu.style.position = "absolute";
    trackingMenu.style.maxWidth = "min(400px, -20px + 100dvw)";
    trackingMenu.style.left = rect.left + window.scrollX + "px";
    trackingMenu.style.top = rect.bottom + window.scrollY + 10 + "px";
    trackingMenu.classList.toggle(
      "topic-timeline-notifications-tracking-content",
      !!trigger.closest(".timeline-footer-controls")
    );
    trackingMenu.hidden = false;
    trackingMenu.classList.add("-expanded");
    trigger.setAttribute("aria-expanded", "true");
    trackingTrigger = trigger;
  }

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      closeTrackingMenu();
    }
  });

  // A level saved: TopicDetails#updateNotifications sets it and clears
  // the reason, so every trigger and the footer's reason follow.
  document.addEventListener("htmx:afterRequest", function (event) {
    var option = event.target.closest(".notifications-tracking-btn");
    if (!option || !event.detail.successful) {
      return;
    }
    var icon = option.querySelector(".notifications-tracking-btn__icons svg");
    document
      .querySelectorAll(".notifications-tracking-trigger")
      .forEach(function (trigger) {
        trigger.dataset.levelId = option.dataset.levelId;
        trigger.dataset.levelName = option.dataset.levelName;
        trigger.title = option.dataset.tooltip;
        trigger.querySelector("svg").replaceWith(icon.cloneNode(true));
        var label = trigger.querySelector(".d-button-label");
        if (label) {
          label.textContent = option.dataset.title + " ";
        }
      });
    document
      .querySelectorAll(".notifications-button-footer .text")
      .forEach(function (text) {
        text.innerHTML = option.dataset.reason;
      });
    trackingMenu
      .querySelectorAll(".notifications-tracking-btn")
      .forEach(function (b) {
        b.classList.toggle("-selected", b === option);
      });
    closeTrackingMenu();
  });

  document.addEventListener("click", function (event) {
    if (trackingMenu) {
      var trigger = event.target.closest(".notifications-tracking-trigger");
      if (trigger) {
        var reopen = trigger !== trackingTrigger;
        closeTrackingMenu();
        if (reopen) {
          openTrackingMenu(trigger);
        }
        return;
      }
      if (!event.target.closest(".notifications-tracking-content")) {
        closeTrackingMenu();
      }
    }
    // A post's copy link, and the footer's share: the URL, absolute.
    var copy = event.target.closest(
      ".post-action-menu__copy-link, .share-and-invite"
    );
    if (copy) {
      navigator.clipboard.writeText(
        new URL(copy.dataset.shareUrl, location.href).href
      );
      return;
    }
    // Show more: the post menu's collapsed buttons.
    var more = event.target.closest(".post-action-menu__show-more");
    if (more) {
      var nav = more.closest("nav.post-controls");
      nav.querySelectorAll(".actions > [hidden]").forEach(function (b) {
        b.hidden = false;
      });
      nav.classList.replace("collapsed", "expanded");
      more.remove();
      return;
    }
    // An anonymous reader's reply asks them to log in.
    var login = event.target.closest("[data-login-url]");
    if (login) {
      location.href = login.dataset.loginUrl;
    }
  });

  // A post created while the page was read can arrive once more: keep the
  // first copy of each.
  document.addEventListener("htmx:sseMessage", function () {
    var seen = {};
    document
      .querySelectorAll("#posts > [data-post-number]")
      .forEach(function (el) {
        var n = el.getAttribute("data-post-number");
        if (seen[n]) {
          el.remove();
        } else {
          seen[n] = true;
        }
      });
  });

  // The timeline.
  var container = document.querySelector(".timeline-container");
  if (!container) {
    return;
  }
  var stream = JSON.parse(container.dataset.stream);
  var lookup = JSON.parse(container.dataset.lookup);
  var total = Math.max(stream.length, 1);
  var chunk = parseInt(container.dataset.chunkSize, 10) || 20;
  var area = container.querySelector(".timeline-scrollarea");
  var paddings = area.querySelectorAll(".timeline-padding");
  var replies = container.querySelector(".timeline-replies");
  var scrollerContent = container.querySelector(".timeline-scroller-content");
  var SCROLLER = 50;
  var height = 300;
  var current = 1;

  function clamp(v, lo, hi) {
    return Math.max(lo, Math.min(hi, v));
  }

  function headerHeight() {
    var header = document.querySelector(".d-header-wrap");
    return header ? header.getBoundingClientRect().height : 0;
  }

  // The lookup's date label for an index: the exact entry or the nearest
  // one before it.
  function agoFor(index) {
    var label = null;
    for (var i = 0; i < lookup.length && lookup[i][0] <= index; i++) {
      label = lookup[i][1];
    }
    return label;
  }

  function render(percent) {
    var before = (height - SCROLLER) * percent;
    paddings[0].style.height = before + "px";
    paddings[1].style.height = height - before - SCROLLER + "px";
    current = clamp(clamp(Math.floor(total * percent), 0, total) + 1, 1, total);
    replies.textContent = container.dataset.repliesFormat
      .replace("%{current}", current)
      .replace("%{total}", total);
    var label = agoFor(current);
    var ago = scrollerContent.querySelector(".timeline-ago");
    if (label === null) {
      if (ago) {
        ago.remove();
      }
    } else {
      if (!ago) {
        ago = document.createElement("div");
        ago.className = "timeline-ago";
        scrollerContent.appendChild(ago);
      }
      ago.textContent = label;
    }
  }

  // The post at the eyeline, which slides from the top of the posts to the
  // bottom of the window as the reader nears the end of the page.
  function percentNow() {
    var wrapper = document.querySelector(".posts-wrapper");
    var boundary = document.querySelector(".post-stream__bottom-boundary");
    var top = Math.max(headerHeight(), wrapper.getBoundingClientRect().top) + 1;
    var bottom = boundary
      ? boundary.getBoundingClientRect().top
      : window.innerHeight;
    var docH = document.documentElement.scrollHeight;
    var vh = window.innerHeight;
    var span = Math.min(vh, docH - (bottom + window.scrollY), docH - vh);
    var progress =
      span > 0 ? 1 - clamp((docH - vh - window.scrollY) / span, 0, 1) : 1;
    var eyeline = top + progress * (bottom - top);
    var posts = document.querySelectorAll("#posts article[data-post-id]");
    for (var i = 0; i < posts.length; i++) {
      var rect = posts[i].getBoundingClientRect();
      // The last post holds the eyeline at its bottom edge too: the end of
      // the page.
      var last = i === posts.length - 1;
      if (eyeline >= rect.top && (eyeline < rect.bottom || (last && eyeline <= rect.bottom))) {
        var index = stream.indexOf(parseInt(posts[i].dataset.postId, 10)) + 1;
        if (index < 1) {
          return null;
        }
        var within = (eyeline - rect.top) / rect.height;
        return clamp((index + within - 1) / total, 0, 1);
      }
    }
    return null;
  }

  // timeline-docked above the posts, timeline-docked-bottom past them.
  function dock() {
    var posts = document.querySelector(".container.posts");
    var bottom = document.getElementById("topic-bottom");
    var pos = headerHeight() + window.scrollY;
    var topicTop = posts.getBoundingClientRect().top + window.scrollY;
    var topicBottom = bottom.getBoundingClientRect().top + window.scrollY;
    var above = pos < topicTop;
    var past =
      !above && pos + container.getBoundingClientRect().height > topicBottom;
    container.classList.toggle("timeline-docked", above || past);
    container.classList.toggle("timeline-docked-bottom", past);
  }

  var pending = false;
  function onScroll() {
    if (pending) {
      return;
    }
    pending = true;
    window.requestAnimationFrame(function () {
      pending = false;
      var percent = percentNow();
      if (percent !== null) {
        render(percent);
      }
      dock();
    });
  }

  function resize() {
    height = clamp((window.innerHeight - headerHeight()) / 2, 170, 300);
    area.style.height = height + "px";
    onScroll();
  }

  // A click on the scroll area: to that post, on this page or its own.
  area.addEventListener("click", function (event) {
    if (!event.target.closest(".timeline-padding")) {
      return;
    }
    var areaTop = area.getBoundingClientRect().top;
    var percent = clamp(
      (event.clientY - (areaTop + SCROLLER / 2)) / (height - SCROLLER),
      0,
      1
    );
    var index = clamp(Math.floor(total * percent) + 1, 1, total);
    var post = document.querySelector(
      '#posts article[data-post-id="' + stream[index - 1] + '"]'
    );
    if (post) {
      window.scrollTo({
        top: post.getBoundingClientRect().top + window.scrollY - headerHeight(),
      });
    } else {
      var page = Math.floor((index - 1) / chunk) + 1;
      location.href =
        container.dataset.topicUrl + (page > 1 ? "?page=" + page : "");
    }
  });

  window.addEventListener("scroll", onScroll, { passive: true });
  window.addEventListener("resize", resize);
  resize();
})();
