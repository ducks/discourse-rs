// A member's reading of the topic page: services/screen-track.js and the
// screen tracking of modifiers/post-stream-viewport-tracker.js. The posts
// on screen gather time each second while the window has focus and the
// page was scrolled in the last three minutes; the timings go to
// /topics/timings once a minute, or at once when a post never timed and
// not read yet is on screen. Posts the server took are marked read. Not
// ported: an anonymous reader's time (for the signup prompt), the topic
// list's highest-read cache, and the workaround for ignored users' posts.
(function () {
  "use strict";

  if (!Discourse.once("screen-track")) {
    return;
  }
  // The site's base path, from where this script is served.
  var base = new URL(document.currentScript.src).pathname.replace(
    /\/assets\/screen-track\.js$/,
    ""
  );

  // Each topic page tracks its own reading (page.js); leaving it sends
  // what was read, as the route's stop() does.
  Discourse.onPage(function () {
    var topic = document.querySelector("#topic[data-topic-id]");
    var posts = document.getElementById("posts");
    if (!topic || !posts) {
      return;
    }
    var topicId = parseInt(topic.dataset.topicId, 10);

    var PAUSE_UNLESS_SCROLLED = 1000 * 60 * 3;
    var MAX_TRACKING_TIME = 1000 * 60 * 6;
    var AJAX_FAILURE_DELAYS = [5000, 10000, 20000, 40000];
    var ALLOWED_AJAX_FAILURES = [405, 429, 500, 501, 502, 503, 504];

    var ajaxFailures = 0;
    var blockSendingTill = 0;
    var consolidated = [];
    var lastTick = Date.now();
    var lastScrolled = Date.now();
    var lastFlush = 0;
    var timings = new Map();
    var totalTimings = new Map();
    var topicTime = 0;
    var onScreen = new Map();
    var readPosts = new Set();
    var inProgress = false;

    function postNumber(element) {
      return parseInt(element.dataset.postNumber, 10);
    }

    function isRead(element) {
      var state = element.querySelector(".read-state");
      return !state || state.classList.contains("read");
    }

    // trackVisiblePosts: the posts in the viewport below the header.
    var observer = null;
    function observe() {
      if (observer) {
        observer.disconnect();
      }
      var header = document.querySelector(".d-header-wrap");
      var headerOffset = header ? header.getBoundingClientRect().height : 0;
      onScreen.clear();
      observer = new IntersectionObserver(
        function (entries) {
          entries.forEach(function (entry) {
            if (entry.isIntersecting) {
              onScreen.set(postNumber(entry.target), entry.target);
            } else {
              onScreen.delete(postNumber(entry.target));
            }
          });
        },
        { root: document, rootMargin: -headerOffset + "px 0px 0px 0px", threshold: [0, 1] }
      );
      posts.querySelectorAll(":scope > [data-post-number]").forEach(function (el) {
        observer.observe(el);
      });
    }
    observe();
    // Posts arriving on the live stream are watched too.
    var mutations = new MutationObserver(observe);
    mutations.observe(posts, { childList: true });
    window.addEventListener("resize", observe);

    function scrolled() {
      lastScrolled = Date.now();
    }
    window.addEventListener("scroll", scrolled);

    function csrfHeaders() {
      var headers = {};
      try {
        headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
      } catch (e) {
        // No token: the server refuses the request.
      }
      headers["Content-Type"] = "application/x-www-form-urlencoded";
      headers["Discourse-Background"] = "true";
      return headers;
    }

    // consolidateTimings
    function consolidate(newTimings, time) {
      var found = consolidated.find(function (c) {
        return c.topicId === topicId;
      });
      if (found) {
        Object.keys(newTimings).forEach(function (n) {
          found.timings[n] = (found.timings[n] || 0) + newTimings[n];
        });
        found.topicTime += time;
      } else {
        consolidated.push({ timings: newTimings, topicTime: time, topicId: topicId });
      }
    }

    // The posts the server took: readPosts.
    function markRead(numbers) {
      numbers.forEach(function (n) {
        var state = posts.querySelector(
          ':scope > [data-post-number="' + n + '"] .read-state'
        );
        if (state) {
          state.classList.add("read");
        }
      });
    }

    // sendNextConsolidatedTiming
    function sendNext() {
      if (consolidated.length === 0 || inProgress || blockSendingTill > Date.now()) {
        return;
      }
      var next = consolidated.pop();
      var body = Object.keys(next.timings)
        .map(function (n) {
          return "timings%5B" + n + "%5D=" + next.timings[n];
        })
        .concat(["topic_time=" + next.topicTime, "topic_id=" + next.topicId])
        .join("&");
      inProgress = true;
      fetch(base + "/topics/timings", {
        method: "POST",
        headers: csrfHeaders(),
        credentials: "same-origin",
        // The last flush is sent as the page is left.
        keepalive: true,
        body: body,
      })
        .then(function (r) {
          if (r.ok) {
            ajaxFailures = 0;
            markRead(Object.keys(next.timings));
            return;
          }
          if (ALLOWED_AJAX_FAILURES.indexOf(r.status) >= 0) {
            var wait = parseInt(r.headers.get("Retry-After"), 10);
            var delay;
            if (r.status === 429 && wait > 0) {
              delay = wait * 1000;
            } else {
              delay = AJAX_FAILURE_DELAYS[Math.min(ajaxFailures, AJAX_FAILURE_DELAYS.length - 1)];
              ajaxFailures += 1;
            }
            blockSendingTill = Date.now() + delay;
            consolidate(next.timings, next.topicTime);
          }
          console.warn(
            "Failed to update topic times for topic " + next.topicId + " due to " + r.status + " error"
          );
        })
        .catch(function () {
          // Offline: the timings are dropped, as jQuery's ajax error is.
        })
        .finally(function () {
          inProgress = false;
          lastFlush = 0;
        });
    }

    function flush() {
      var newTimings = {};
      var highestSeen = 0;
      timings.forEach(function (time, n) {
        var total = totalTimings.get(n) || 0;
        if (time > 0 && total < MAX_TRACKING_TIME) {
          totalTimings.set(n, total + time);
          newTimings[n] = time;
          highestSeen = Math.max(highestSeen, n);
        }
        timings.set(n, 0);
      });
      if (highestSeen > 0) {
        consolidate(newTimings, topicTime);
        sendNext();
        topicTime = 0;
      }
      lastFlush = 0;
    }

    function tick() {
      var now = Date.now();
      if (now - lastScrolled > PAUSE_UNLESS_SCROLLED) {
        return;
      }
      var diff = now - lastTick;
      lastFlush += diff;
      lastTick = now;

      var rush = false;
      timings.forEach(function (time, n) {
        if (time > 0 && !totalTimings.get(n) && !readPosts.has(n)) {
          rush = true;
        }
      });
      if (!inProgress && (lastFlush > 60 * 1000 || rush)) {
        flush();
      }
      if (!inProgress) {
        sendNext();
      }

      if (document.hasFocus()) {
        topicTime += diff;
        onScreen.forEach(function (element, n) {
          timings.set(n, (timings.get(n) || 0) + diff);
          if (isRead(element)) {
            readPosts.add(n);
          }
        });
      }
    }

    var interval = setInterval(tick, 1000);
    var stopped = false;
    // stop(): what was read goes out when the page is left.
    function leave() {
      if (stopped) {
        return;
      }
      tick();
      flush();
    }
    window.addEventListener("pagehide", leave);
    // reset() then stop(), as Mark unread does before it unreads the topic:
    // nothing more is timed or sent.
    function reset() {
      stopped = true;
      clearInterval(interval);
      timings.clear();
      totalTimings.clear();
      topicTime = 0;
      onScreen.clear();
      readPosts.clear();
      consolidated.length = 0;
    }
    document.addEventListener("screen-track:stop", reset);

    return function () {
      leave();
      stopped = true;
      clearInterval(interval);
      if (observer) {
        observer.disconnect();
      }
      mutations.disconnect();
      window.removeEventListener("resize", observe);
      window.removeEventListener("scroll", scrolled);
      window.removeEventListener("pagehide", leave);
      document.removeEventListener("screen-track:stop", reset);
    };
  });
})();
