// The topic page's client side: the buttons that act in the browser (copy
// link, share, show more, an anonymous reader's reply), live updates
// arriving twice, and the timeline, or in the mobile view the progress
// bar, which follow the post being read as Ember's do
// (components/topic-timeline, topic-progress,
// modifiers/post-stream-viewport-tracker).
(function () {
  "use strict";

  if (!Discourse.once("topic")) {
    return;
  }

  function clamp(v, lo, hi) {
    return Math.max(lo, Math.min(hi, v));
  }

  function headerHeight() {
    var header = document.querySelector(".d-header-wrap");
    return header ? header.getBoundingClientRect().height : 0;
  }

  // lib/offset-calculator.js headerOffset(): the --header-offset the
  // layout keeps on <html>.
  function headerOffset() {
    return parseInt(document.documentElement.style.getPropertyValue("--header-offset"), 10) || 0;
  }

  // offsetCalculator(): the header, but on the desktop view at the bottom
  // of a page whose footer is shorter than the window, the share of the
  // window the footer's visible part scales up to.
  function offsetCalculator() {
    var min = headerOffset();
    var bottom = document.getElementById("topic-bottom");
    if (document.documentElement.classList.contains("mobile-view") || !bottom) {
      return min;
    }
    var windowHeight = window.innerHeight;
    var documentHeight = document.body.clientHeight;
    var topicBottomOffsetTop = bottom.offsetTop;
    if (documentHeight - windowHeight > topicBottomOffsetTop) {
      return min;
    }
    var visibleBottomHeight = window.scrollY + windowHeight - topicBottomOffsetTop;
    if (visibleBottomHeight > 0) {
      var bottomHeight = documentHeight - topicBottomOffsetTop;
      return Math.max(min, ((windowHeight - bottomHeight) * visibleBottomHeight) / bottomHeight);
    }
    return min;
  }

  // The topic's URL (Topic#url, the page's canonical /t/:slug/:id) and
  // the post number the address asks for (the route's nearPost).
  var TOPIC_PATH = /^(.*\/t\/[^/]+\/\d+)(?:\/(\d+))?\/?$/;

  function topicUrl() {
    var m = TOPIC_PATH.exec(location.pathname);
    return m ? m[1] : null;
  }

  function nearPost() {
    var m = TOPIC_PATH.exec(location.pathname);
    return m && m[2] ? parseInt(m[2], 10) : null;
  }

  // PostStream#closestPostNumberFor: the post on the page nearest the
  // number, the first of a tie.
  function closestPostNumber(postNumber) {
    var closest = null;
    document.querySelectorAll("#posts > [data-post-number]").forEach(function (el) {
      var n = parseInt(el.dataset.postNumber, 10);
      if (closest === null || Math.abs(postNumber - n) < Math.abs(closest - postNumber)) {
        closest = n;
      }
    });
    return closest;
  }

  // lib/lock-on.js: keep the element at the top, below the header, for a
  // second while the page settles (images, fonts, the rest of the page
  // still coming in), unless the reader scrolls.
  var LOCK_DURATION_MS = 1000;
  var LOCK_TIMEOUT_MS = 5000;
  var SCROLL_EVENTS = ["scroll", "touchmove", "mousedown", "wheel", "keyup"];
  var SCROLL_TYPES = ["mousedown", "mousewheel", "touchmove", "wheel"];
  var lock = null;

  function within(threshold, x, y) {
    return Math.abs(x - y) < threshold;
  }

  function query(selector) {
    try {
      return document.querySelector(selector);
    } catch (e) {
      // An anchor that is no selector: Ember's querySelector throws too.
      console.error(e);
      return null;
    }
  }

  function LockOn(selector) {
    var self = this;
    var startedAt;
    var previousTop;
    var requestId;

    this.elementTop = function () {
      var element = query(selector);
      if (!element) {
        return;
      }
      return element.getBoundingClientRect().top + window.scrollY - headerOffset();
    };

    function listener(event) {
      if (event.which > 0 || SCROLL_TYPES.indexOf(event.type) !== -1) {
        self.clearLock();
      }
    }

    function removeListener() {
      SCROLL_EVENTS.forEach(function (name) {
        document.body.removeEventListener(name, listener);
      });
    }

    this.clearLock = function () {
      removeListener();
      window.cancelAnimationFrame(requestId);
      if (lock === self) {
        lock = null;
      }
    };

    function performLocking() {
      var elementTop = self.elementTop();
      if (!previousTop && !elementTop) {
        if (Date.now() - startedAt > LOCK_TIMEOUT_MS) {
          self.clearLock();
          return;
        }
        requestId = window.requestAnimationFrame(performLocking);
        return;
      }
      var top = Math.max(0, elementTop);
      if (isNaN(top)) {
        self.clearLock();
        return;
      }
      if (!within(4, top, previousTop) || !within(4, window.scrollY, top)) {
        window.scrollTo(window.pageXOffset, top);
        previousTop = top;
      }
      if (Date.now() - startedAt > LOCK_DURATION_MS) {
        self.clearLock();
        return;
      }
      requestId = window.requestAnimationFrame(performLocking);
    }

    this.lock = function () {
      startedAt = Date.now();
      previousTop = self.elementTop();
      if (previousTop) {
        window.scrollTo(window.pageXOffset, previousTop);
      }
      requestId = window.requestAnimationFrame(performLocking);
      removeListener();
      SCROLL_EVENTS.forEach(function (name) {
        document.body.addEventListener(name, listener, { passive: true });
      });
    };
  }

  // DiscourseURL.jumpToPost: the top for the first post (without an
  // anchor), else the anchor or the post locked on below the header;
  // with skipIfOnScreen, not a post already wholly in the window.
  function jumpToPost(postNumber, anchor, skipIfOnScreen) {
    var holderId = "#post_" + postNumber;
    if (postNumber === 1 && !anchor) {
      window.scrollTo(window.pageXOffset, 0);
      return;
    }
    var selector;
    var holder;
    if (anchor) {
      selector = "#main #" + anchor + ", a[name=" + anchor + "]";
      holder = query(selector);
      if (!holder) {
        // Not drawn: LockOn waits for it.
        var postHolder = document.querySelector(holderId);
        if (postHolder) {
          postHolder.scrollIntoView(true);
        }
      }
    }
    if (!holder && !anchor) {
      selector = holderId;
      holder = document.querySelector(holderId);
    }
    if (lock) {
      lock.clearLock();
    }
    lock = new LockOn(selector);
    if (holder && skipIfOnScreen) {
      var elementTop = lock.elementTop();
      var scrollTop = window.scrollY;
      var windowHeight = window.innerHeight - offsetCalculator();
      if (elementTop > scrollTop && elementTop + holder.offsetHeight < scrollTop + windowHeight) {
        lock = null;
        return;
      }
    }
    lock.lock();
    if (lock && lock.elementTop() < 1) {
      lock.clearLock();
    }
  }

  // lib/utilities.js highlightPost, which post:highlight runs: a later
  // post's body takes the focus, for screen readers.
  function highlightPost(postNumber) {
    var container = document.querySelector("#post_" + postNumber);
    var element = container && container.querySelector(".topic-body, .small-action-desc");
    if (!element) {
      return;
    }
    if (postNumber > 1) {
      element.setAttribute("tabindex", "0");
      element.focus();
    }
  }

  // DiscourseURL.replaceState: the address, when it changes.
  function replaceState(path) {
    if (location.pathname + location.search + location.hash !== path) {
      history.replaceState(history.state, "", path);
    }
  }

  // Whether Ember has the post drawn: its viewport tracker cloaks the
  // posts more than a window's height outside the window (the cloaking
  // observer's root margin, SLACK_FACTOR 1), uncloaking one that overlaps
  // that band by 5px, or by 1px and 5% of its height.
  function uncloaked(article) {
    var rect = article.getBoundingClientRect();
    var margin = Math.ceil(window.innerHeight);
    var overlap = Math.min(rect.bottom, window.innerHeight + margin) - Math.max(rect.top, -margin);
    return overlap >= 5 || (overlap >= 1 && overlap / rect.height >= 0.05);
  }

  // DiscourseURL.routeTo a post of this topic that the page has
  // (navigatedToPost): the address, then post:highlight and jumpToPost.
  // post:highlight runs before the jump, so a post Ember has cloaked there
  // has no body to focus.
  function goToPost(postNumber) {
    replaceState(topicUrl() + "/" + postNumber + location.search);
    if (uncloaked(document.getElementById("post_" + postNumber))) {
      highlightPost(postNumber);
    }
    jumpToPost(postNumber, null);
  }

  // The page for a path, swapped in as a link's (hx-boost); replace takes
  // the address's place in the history, as DiscourseURL.replaceState.
  function visit(path, replace) {
    if (!window.htmx) {
      if (replace) {
        location.replace(path);
      } else {
        location.assign(path);
      }
      return;
    }
    var link = document.createElement("a");
    link.href = path;
    link.hidden = true;
    if (replace) {
      link.setAttribute("hx-replace-url", "true");
    }
    document.getElementById("main-outlet").appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  function topicIdOf(path) {
    var m = TOPIC_PATH.exec(path);
    return m ? /(\d+)$/.exec(m[1])[1] : null;
  }

  // The address of a page of this topic swapped in for routeToPost, whose
  // jump leaves a post wholly in the window where it is.
  var pendingRoute = null;

  // DiscourseURL.routeTo(path, { keepFilter: true, skipIfOnScreen: true })
  // for a post's URL, as the composer's save calls it. Within this topic
  // it is navigatedToPost: the address replaced, then post:highlight and
  // jumpToPost, which leaves a post wholly in the window where it is. A
  // post the page doesn't draw is the stream's refresh around it: the
  // page for it, taking the address's place. Another topic's post is
  // that topic's page.
  //
  // Ember's post:highlight focuses only a post its stream already drew:
  // none after a refresh, and a post just created only when the composer
  // staged it (opts.highlight false otherwise), its body not drawn yet
  // when it runs (measured against rs-ref-c: an edit saved away from its
  // post, replies at the bottom and from the middle, staged or not).
  // opts.arriving: a post the page's live stream is bringing (a reply on
  // the last page), waited for rather than loaded.
  function routeToPost(path, opts) {
    opts = opts || {};
    var pathname = path.split(/[?#]/)[0];
    var target = TOPIC_PATH.exec(pathname);
    if (!target || !document.getElementById("topic") || topicIdOf(location.pathname) !== topicIdOf(pathname)) {
      visit(path, false);
      return;
    }
    var number = target[2] ? parseInt(target[2], 10) : 1;
    var go = function () {
      var holder = document.getElementById("post_" + number);
      if (!holder) {
        pendingRoute = pathname;
        visit(path, true);
        return;
      }
      replaceState(path);
      if (opts.highlight !== false && uncloaked(holder)) {
        highlightPost(number);
      }
      jumpToPost(number, null, true);
    };
    if (opts.arriving && !document.getElementById("post_" + number)) {
      whenDrawn(number, go);
    } else {
      go();
    }
  }

  // Runs fn once the post is on the page (a live update bringing it), or
  // after five seconds without it.
  var DRAWN_TIMEOUT_MS = 5000;
  function whenDrawn(number, fn) {
    var posts = document.getElementById("posts");
    if (!posts || document.getElementById("post_" + number)) {
      fn();
      return;
    }
    var done = false;
    var observer = new MutationObserver(function () {
      if (document.getElementById("post_" + number)) {
        finish();
      }
    });
    var timer = setTimeout(finish, DRAWN_TIMEOUT_MS);
    function finish() {
      if (done) {
        return;
      }
      done = true;
      observer.disconnect();
      clearTimeout(timer);
      fn();
    }
    observer.observe(posts, { childList: true, subtree: true });
  }

  // Whether new posts land on this page: the last page of the topic,
  // whose live stream brings them (Ember's loadedAllPosts).
  function atTopicEnd() {
    return !!document.querySelector('[sse-connect*="tail=1"]');
  }

  Discourse.topicRoute = {
    routeToPost: routeToPost,
    atTopicEnd: atTopicEnd,
    topicId: function () {
      return document.getElementById("topic") ? topicIdOf(location.pathname) : null;
    },
  };

  // Ember draws a post's changes in place, so the body post:highlight
  // focused keeps its tabindex and the focus; a live update here swaps the
  // post's markup, and the new body takes them over.
  var redrawnHighlight = null;
  document.addEventListener("htmx:oobBeforeSwap", function (event) {
    var target = event.detail && event.detail.target;
    if (!target || !target.matches || !target.matches("#posts > [data-post-number]")) {
      return;
    }
    var body = target.querySelector('.topic-body[tabindex="0"], .small-action-desc[tabindex="0"]');
    if (body) {
      redrawnHighlight = {
        number: target.dataset.postNumber,
        focused: document.activeElement === body,
      };
    }
  });
  document.addEventListener("htmx:oobAfterSwap", function () {
    var kept = redrawnHighlight;
    redrawnHighlight = null;
    if (!kept) {
      return;
    }
    var wrapper = document.querySelector('#posts > [data-post-number="' + kept.number + '"]');
    var body = wrapper && wrapper.querySelector(".topic-body, .small-action-desc");
    if (!body) {
      return;
    }
    body.setAttribute("tabindex", "0");
    if (kept.focused) {
      body.focus({ preventScroll: true });
    }
  });

  // A layout switch draws the page again where the reader is (page.js
  // swapIn): not an entry.
  var redrawing = false;
  document.addEventListener("discourse:before-redraw", function () {
    redrawing = true;
  });
  document.addEventListener("discourse:redrawn", function () {
    redrawing = false;
  });

  // Entering the topic (TopicFromParams#setupController): the post nearest
  // the one asked for is highlighted and jumped to, or the anchor in the
  // address is. Only a page load takes the focus there: come from another
  // page of the app, Ember's focuses nothing (measured against rs-ref-b),
  // and a link to another post of the same topic is navigatedToPost's
  // jump, where a cloaked post has no body to focus (goToPost).
  Discourse.onPage(function () {
    var url = topicUrl();
    if (redrawing || !document.getElementById("topic") || !url) {
      return;
    }
    var closest = closestPostNumber(nearPost() || 1);
    if (closest === null) {
      return;
    }
    var previous = TOPIC_PATH.exec((Discourse.previousUrl || "").split("?")[0]);
    // The composer's routeTo of a post the page didn't have.
    var route = pendingRoute === location.pathname;
    pendingRoute = null;
    if (!Discourse.swapped) {
      highlightPost(closest);
    } else if (
      previous &&
      previous[1] === url &&
      !route &&
      uncloaked(document.getElementById("post_" + closest))
    ) {
      highlightPost(closest);
    }
    jumpToPost(closest, location.hash ? location.hash.slice(1) : null, route);
    return function () {
      if (lock) {
        lock.clearLock();
      }
    };
  });

  // The post at the eyeline, which slides from the top of the posts to the
  // bottom of the window as the reader nears the end of the page
  // (post-stream-viewport-tracker.js findPostMatchingEyeline): its
  // article and how far down it the eyeline is. Null between posts.
  function eyelinePost() {
    var wrapper = document.querySelector(".posts-wrapper");
    if (!wrapper) {
      return null;
    }
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
        return { article: posts[i], within: (eyeline - rect.top) / rect.height };
      }
    }
    return null;
  }

  // The eyeline's post as an index in the stream, and how far through the
  // topic that is (TopicController#currentPostScrolled's percent).
  function position(stream) {
    var at = eyelinePost();
    if (!at) {
      return null;
    }
    var index = stream.indexOf(parseInt(at.article.dataset.postId, 10)) + 1;
    if (index < 1) {
      return null;
    }
    var total = Math.max(stream.length, 1);
    return { index: index, percent: clamp((index + at.within - 1) / total, 0, 1) };
  }

  // The address follows the post being read (currentPostChanged and the
  // topic route's postChangedRoute): when the eyeline's post changes, the
  // topic's URL with its number (the bare URL for the first post) replaces
  // the address once the page has stopped scrolling for half a second
  // (_replaceUnlessScrolling). The eyeline is looked for 10ms after a
  // scroll (SCROLL_BATCH_INTERVAL_MS).
  var SCROLL_DELAY = 500;
  var SCROLL_BATCH_INTERVAL_MS = 10;
  Discourse.onPage(function () {
    var url = topicUrl();
    if (!url || !document.getElementById("topic")) {
      return;
    }
    var current = null;
    var batch = null;
    var scheduled = null;
    var lastScrollPos = 0;

    // The topic's URL as the address has it when the time comes: an edit
    // that retitled the topic has put its new slug there (routeToPost).
    function replaceUnlessScrolling(number) {
      var pos = document.scrollingElement.scrollTop;
      if (pos === lastScrollPos) {
        var at = topicUrl() || url;
        replaceState((number > 1 ? at + "/" + number : at) + location.search + location.hash);
        return;
      }
      lastScrollPos = pos;
      scheduled = setTimeout(replaceUnlessScrolling, SCROLL_DELAY, number);
    }

    function check() {
      batch = null;
      var at = eyelinePost();
      if (!at) {
        return;
      }
      var number = parseInt(at.article.parentElement.dataset.postNumber, 10);
      if (number === current) {
        return;
      }
      current = number;
      clearTimeout(scheduled);
      lastScrollPos = document.scrollingElement.scrollTop;
      scheduled = setTimeout(replaceUnlessScrolling, SCROLL_DELAY, number);
    }

    function onScroll() {
      clearTimeout(batch);
      batch = setTimeout(check, SCROLL_BATCH_INTERVAL_MS);
    }

    window.addEventListener("scroll", onScroll, { passive: true });
    // The viewport tracker sees a post the live stream adds too: a reply
    // landing under the eyeline at the end of the page.
    document.addEventListener("htmx:sseMessage", onScroll);
    onScroll();
    return function () {
      window.removeEventListener("scroll", onScroll);
      document.removeEventListener("htmx:sseMessage", onScroll);
      clearTimeout(batch);
      clearTimeout(scheduled);
    };
  });

  // The progress bar (TopicProgress): the post's index and the bar's fill.
  Discourse.onPage(function () {
    var nav = document.getElementById("topic-progress");
    if (!nav) {
      return;
    }
    var stream = JSON.parse(nav.dataset.stream);
    var current = nav.querySelector(".nums span");
    var pending = false;
    function onScroll() {
      if (pending) {
        return;
      }
      pending = true;
      window.requestAnimationFrame(function () {
        pending = false;
        var at = position(stream);
        if (at) {
          current.textContent = at.index;
          nav.style.setProperty("--progress-bg-width", (at.percent * 100).toFixed(2) + "%");
        }
      });
    }
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll);
    onScroll();
    return function () {
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onScroll);
    };
  });

  // The mobile view's footer "..." (the topic-footer-mobile-dropdown
  // DMenu): its buttons, from the template beside it, in a bottom sheet
  // that a click on one closes (DDropdownMenu).
  var footerSheet = null;
  document.addEventListener("click", function (event) {
    var trigger = event.target.closest(".topic-footer-mobile-dropdown-trigger");
    if (!trigger || !window.Discourse || !Discourse.float) {
      return;
    }
    if (footerSheet && footerSheet.expanded) {
      footerSheet.close();
      return;
    }
    var template = trigger.parentElement.querySelector("template.topic-footer-mobile-dropdown-menu");
    if (!template) {
      return;
    }
    var content = template.content.cloneNode(true).firstElementChild;
    footerSheet = Discourse.float.menu(trigger, {
      identifier: "topic-footer-mobile-dropdown",
      modalForMobile: true,
      content: content,
    });
    var sheet = footerSheet;
    content.addEventListener("click", function () {
      sheet.close();
    });
  });

  // The member's CSRF token and XHR header, from the body's hx-headers.
  function memberHeaders() {
    try {
      return JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request.
      return {};
    }
  }

  // popupAjaxError: the error the server gave.
  function popupError(r) {
    return r
      .json()
      .then(function (body) {
        window.alert((body.errors || [r.statusText]).join("\n"));
      })
      .catch(function () {
        window.alert(r.statusText);
      });
  }

  // A message's participant off it (TopicController#removeAllowedUser and
  // #removeAllowedGroup): PUT remove-allowed-user or remove-allowed-group,
  // then their entry leaves the map. Removing oneself asks first, then
  // goes to one's messages.
  function removeParticipant(button) {
    var self = button.hasAttribute("data-remove-self");
    var user = button.dataset.removeUser;
    var form = new URLSearchParams();
    var path;
    if (user !== undefined) {
      form.set("username", user);
      path = "/remove-allowed-user";
    } else {
      form.set("name", button.dataset.removeGroup);
      path = "/remove-allowed-group";
    }
    var map = button.closest(".topic-map__private-message-map");
    var url = map.dataset.topicUrl + path;
    function send() {
      var headers = memberHeaders();
      headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      fetch(url, {
        method: "PUT",
        headers: headers,
        body: form.toString(),
        credentials: "same-origin",
      }).then(function (r) {
        if (!r.ok) {
          return popupError(r);
        }
        if (self) {
          location.href = map.dataset.messagesUrl;
          return;
        }
        // Every copy of the map (the first post's and the bottom one).
        var kind = user !== undefined ? "data-remove-user" : "data-remove-group";
        var name = user !== undefined ? user : button.dataset.removeGroup;
        document
          .querySelectorAll(".topic-map__private-message-map [" + kind + "]")
          .forEach(function (b) {
            if (b.getAttribute(kind) === name) {
              b.closest(".user").remove();
            }
          });
      });
    }
    if (self) {
      Discourse.dialog.yesNoConfirm({
        message: map.dataset.removeSelfConfirm,
        didConfirm: send,
      });
    } else {
      send();
    }
  }

  // The more topics tabs (services/more-topics-tabs): the list shown is
  // the picked tab's, remembered per context in localStorage as Ember's
  // keyValueStore keeps it; the others wait in templates.
  function moreTopicsKey() {
    var pm = document.body.classList.contains("archetype-private_message");
    return "discourse_more-topics-preference-" + (pm ? "pm" : "topic");
  }

  function showMoreTopicsTab(id) {
    var lists = document.querySelector(".more-topics__lists");
    var template = lists && lists.querySelector('template.more-topics__tab[data-tab-id="' + id + '"]');
    if (!template) {
      return;
    }
    var current = lists.querySelector(".more-topics__list");
    var active = current.querySelector(".topic-list-header-tab.active");
    var stash = document.createElement("template");
    stash.className = "more-topics__tab";
    stash.dataset.tabId = active.dataset.tabId;
    stash.content.appendChild(current);
    template.replaceWith(template.content);
    lists.appendChild(stash);
  }

  Discourse.onPage(function () {
    var saved = null;
    try {
      saved =
        window.localStorage.getItem(moreTopicsKey()) ||
        window.localStorage.getItem("discourse_more-topics-list-preference");
    } catch (e) {
      // No storage: the first tab stays.
    }
    if (saved) {
      showMoreTopicsTab(saved);
    }
  });

  document.addEventListener("click", function (event) {
    var tab = event.target.closest(".topic-list-header-tab");
    if (tab) {
      if (!tab.classList.contains("active")) {
        try {
          window.localStorage.setItem(moreTopicsKey(), tab.dataset.tabId);
        } catch (e) {
          // No storage: the pick lasts this page.
        }
        showMoreTopicsTab(tab.dataset.tabId);
      }
      return;
    }
    var remove = event.target.closest(
      ".topic-map__private-message-map .remove-invited"
    );
    if (remove) {
      removeParticipant(remove);
      return;
    }
    // Archive and Move to Inbox (toggleArchiveMessage): then the inbox the
    // message left or went back to, its group's when it is a group's.
    var archive = event.target.closest("[data-archive-url]");
    if (archive) {
      if (archive.disabled) {
        return;
      }
      archive.disabled = true;
      fetch(archive.dataset.archiveUrl, {
        method: "PUT",
        headers: memberHeaders(),
        credentials: "same-origin",
      })
        .then(function (r) {
          if (!r.ok) {
            // Ember leaves the failure unhandled: the button comes back.
            throw new Error(archive.dataset.archiveUrl + ": " + r.status);
          }
          return r.json();
        })
        .then(function (body) {
          var url = archive.dataset.inboxUrl;
          if (body && body.group_name) {
            url += "/group/" + encodeURIComponent(body.group_name);
          }
          location.href = url;
        })
        .catch(function (e) {
          archive.disabled = false;
          console.error(e);
        });
      return;
    }
    // A footer button whose menu is not ported (the topic's bookmarks
    // menu, TopicBookmarksMenu's grouped dropdown) says so.
    var unportedButton = event.target.closest("#topic-footer-buttons [data-unported]");
    if (unportedButton) {
      var notice = document.getElementById("page-error");
      if (notice) {
        notice.textContent = "Not ported yet: " + unportedButton.dataset.unported + ".";
        notice.hidden = false;
      }
      return;
    }
    // Mark unread (deferTopic): stop timing the page, unread its last
    // post, then go home (or to the message's inbox).
    var defer =event.target.closest("[data-defer-url]");
    if (defer) {
      document.dispatchEvent(new Event("screen-track:stop"));
      fetch(defer.dataset.deferUrl, {
        method: "DELETE",
        headers: memberHeaders(),
        credentials: "same-origin",
      }).then(function (r) {
        if (r.ok) {
          location.href = defer.dataset.deferTo;
          return;
        }
        return popupError(r);
      });
      return;
    }
    // A small action's delete and recover (the topic controller's
    // deletePost and recoverPost): DELETE /posts/:id with the page as its
    // context (Post#destroy), PUT /posts/:id/recover, then the post again
    // as the server draws it. Staff deleting one with replies get the
    // reply-ids dialog in Ember, not ported.
    var smallAction = event.target.closest(
      ".small-action-delete, .small-action-recover"
    );
    if (smallAction) {
      var article = smallAction.closest("article.small-action");
      if (!article || smallAction.disabled) {
        return;
      }
      if (smallAction.hasAttribute("data-has-replies")) {
        var error = document.getElementById("page-error");
        if (error) {
          error.textContent =
            "Not ported yet: deleting a post with replies (the delete replies dialog).";
          error.hidden = false;
        }
        return;
      }
      var root =
        (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
      var postId = article.dataset.postId;
      var recover = smallAction.classList.contains("small-action-recover");
      var headers = memberHeaders();
      var init = { method: recover ? "PUT" : "DELETE", headers: headers, credentials: "same-origin" };
      if (!recover) {
        headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
        init.body = "context=" + encodeURIComponent(location.pathname);
      }
      smallAction.disabled = true;
      fetch(root + "/posts/" + postId + (recover ? "/recover" : ""), init).then(function (r) {
        if (!r.ok) {
          smallAction.disabled = false;
          return popupError(r);
        }
        htmx.ajax("GET", root + "/live/post/" + postId, { swap: "none" });
      });
      return;
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

  // The timeline, on each topic page (page.js); its window listeners go
  // with the page.
  Discourse.onPage(function () {
    var container = document.querySelector(".timeline-container");
    if (!container) {
      return;
    }
    return setupTimeline(container, false);
  });

  // The mobile view's fullscreen timeline (TopicNavigation's
  // topicProgressExpanded): a tap on the progress bar swaps it in, its
  // scroller showing the excerpt of the post it is on (PostStream#excerpt,
  // /t/:id/excerpts); a tap outside it, or on a post in it, swaps the
  // progress bar back after its slide out.
  var expanded = null;
  var excerpts = {};
  function collapse(delay) {
    if (!expanded) {
      return;
    }
    var state = expanded;
    expanded = null;
    state.container.classList.remove("show");
    setTimeout(function () {
      state.cleanup();
      state.container.replaceWith(state.wrapper);
      state.nav.classList.remove("topic-progress-expanded", "with-timeline");
      state.nav.classList.add("with-topic-progress");
    }, delay == null ? 500 : delay);
  }

  function showExcerpt(container, stream, index) {
    var postId = stream[index - 1];
    var title = container.querySelector(".title");
    if (!title || !postId) {
      return;
    }
    function fill(info) {
      if (!info || !container.isConnected) {
        return;
      }
      var html = "";
      if (info.username) {
        html = "<span class='username'>" + info.username + ":</span> ";
      }
      if (!info.excerpt) {
        return;
      }
      var div = title.querySelector(".post-excerpt");
      if (!div) {
        div = document.createElement("div");
        div.className = "post-excerpt";
        title.appendChild(div);
      }
      div.innerHTML = html + info.excerpt;
    }
    if (excerpts[postId]) {
      fill(excerpts[postId]);
      return;
    }
    var ids = stream.slice(Math.max(index - 1 - 20, 0), index - 1 + 20).filter(function (id) {
      return !excerpts[id];
    });
    var topicId = container.dataset.topicUrl.split("/").pop();
    var base = (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
    var query = ids
      .map(function (id) {
        return "post_ids%5B%5D=" + id;
      })
      .join("&");
    fetch(base + "/t/" + topicId + "/excerpts.json?" + query, {
      credentials: "same-origin",
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
    })
      .then(function (r) {
        return r.ok ? r.json() : [];
      })
      .then(function (list) {
        list.forEach(function (obj) {
          excerpts[obj.post_id] = obj;
        });
        fill(excerpts[postId]);
      })
      .catch(function () {});
  }

  document.addEventListener("click", function (event) {
    var progress = event.target.closest("#topic-progress");
    if (progress && !expanded) {
      var nav = progress.closest(".topic-navigation");
      var template = nav && nav.querySelector("template.topic-timeline-fullscreen");
      if (!template) {
        return;
      }
      var wrapper = document.getElementById("topic-progress-wrapper");
      var container = template.content.firstElementChild.cloneNode(true);
      nav.classList.remove("with-topic-progress");
      nav.classList.add("topic-progress-expanded", "with-timeline");
      wrapper.replaceWith(container);
      expanded = {
        nav: nav,
        wrapper: wrapper,
        container: container,
        cleanup: setupTimeline(container, true),
      };
      window.requestAnimationFrame(function () {
        container.classList.add("show");
      });
      return;
    }
    if (expanded) {
      if (!event.target.closest(".timeline-fullscreen")) {
        collapse();
      } else if (event.target.closest(".timeline-scrollarea, .fancy-title, .start-date, .now-date")) {
        // topic:jump-to-post
        collapse();
      }
    }
  });

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      collapse();
    }
  });

  // TopicProgress#showBackButton and goBack: while the reader is above
  // their last read post, a button back down to it.
  Discourse.onPage(function () {
    var nav = document.getElementById("topic-progress");
    if (!nav || !nav.dataset.lastReadId) {
      return;
    }
    var stream = JSON.parse(nav.dataset.stream);
    var readPos = stream.indexOf(parseInt(nav.dataset.lastReadId, 10));
    function update() {
      var wrapper = document.getElementById("topic-progress-wrapper");
      if (!wrapper) {
        return;
      }
      var current = parseInt(nav.querySelector(".nums span").textContent, 10) || 1;
      var show = readPos >= 0 && readPos < stream.length - 1 && readPos > current;
      var existing = wrapper.querySelector(".progress-back-container");
      if (show && !existing) {
        var holder = document.createElement("div");
        holder.className = "progress-back-container";
        holder.innerHTML =
          '<button class="btn btn-icon-text btn-primary btn-small progress-back" type="button"><svg class="fa d-icon d-icon-arrow-down svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#arrow-down"></use></svg><span class="d-button-label"></span></button>';
        holder.querySelector(".d-button-label").textContent = nav.dataset.backLabel;
        holder.querySelector("button").addEventListener("click", function () {
          // jumpToPost(last_read_post_number): this page's post, else the
          // topic loaded at it.
          var number = parseInt(nav.dataset.lastReadNumber, 10);
          if (document.getElementById("post_" + number)) {
            goToPost(number);
          } else {
            location.href = topicUrl() + "/" + number;
          }
        });
        wrapper.insertBefore(holder, nav);
      } else if (!show && existing) {
        existing.remove();
      }
    }
    var pending = false;
    function onScroll() {
      if (pending) {
        return;
      }
      pending = true;
      window.requestAnimationFrame(function () {
        pending = false;
        update();
      });
    }
    window.addEventListener("scroll", onScroll, { passive: true });
    update();
    return function () {
      window.removeEventListener("scroll", onScroll);
    };
  });

  function setupTimeline(container, fullscreen) {
    var stream = JSON.parse(container.dataset.stream);
    var lookup = JSON.parse(container.dataset.lookup);
    var total = Math.max(stream.length, 1);
    // displayTimeLineScrollArea: a lone post under 1000px tall gets no
    // scroll area, and no footer controls with it; the container still
    // docks. Ember first asks as the timeline renders, before the posts
    // are there, so the area shows; it asks again on each
    // topic:current-post-scrolled (onScroll, below).
    var bare = false;
    function checkBare() {
      var postsWrapper = document.querySelector(".posts-wrapper");
      if (bare || stream.length !== 1 || !postsWrapper || postsWrapper.offsetHeight >= 1000) {
        return;
      }
      bare = true;
      container
        .querySelectorAll(".timeline-scrollarea-wrapper, .timeline-footer-controls")
        .forEach(function (el) {
          el.remove();
        });
    }
    var area = container.querySelector(".timeline-scrollarea");
    var paddings = area ? area.querySelectorAll(".timeline-padding") : [];
    var replies = container.querySelector(".timeline-replies");
    var scrollerContent = container.querySelector(".timeline-scroller-content");
    var SCROLLER = 50;
    var height = 300;
    var current = 1;

    // The date label for an index (closestDaysAgoFor): the days ago of the
    // exact lookup entry or the nearest one before it, as timelineDate
    // draws today less them in the viewer's zone (static/js/dates.js).
    function agoFor(index) {
      var days = null;
      for (var i = 0; i < lookup.length && lookup[i][0] <= index; i++) {
        days = lookup[i][1];
      }
      return days === null ? null : Discourse.dates.fill("%{" + days + ":@timeline-days-ago}");
    }

    function render(percent) {
      var before = (height - SCROLLER) * percent;
      paddings[0].style.height = before + "px";
      paddings[1].style.height = height - before - SCROLLER + "px";
      current = clamp(clamp(Math.floor(total * percent), 0, total) + 1, 1, total);
      if (fullscreen) {
        showExcerpt(container, stream, current);
      }
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

    function percentNow() {
      var at = position(stream);
      return at ? at.percent : null;
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

    // The container acts on topic:current-post-scrolled, which the
    // viewport tracker sends only while a post is at the eyeline: then it
    // asks displayTimeLineScrollArea again, moves the scroller and checks
    // the dock (dockCheck).
    var pending = false;
    function onScroll() {
      if (pending) {
        return;
      }
      pending = true;
      window.requestAnimationFrame(function () {
        pending = false;
        var percent = percentNow();
        if (percent === null) {
          return;
        }
        checkBare();
        if (!bare) {
          render(percent);
        }
        if (!fullscreen) {
          dock();
        }
      });
    }

    function resize() {
      if (area) {
        height = clamp((window.innerHeight - headerHeight()) / 2, 170, 300);
        area.style.height = height + "px";
      }
      onScroll();
    }

    // updatePercentage: a click on the track or a date, or the scroller
    // dragged, puts the scroller's centre at the pointer (less where on
    // the scroller the drag took hold), then commits: the scroller and
    // its labels follow, and unless a drag goes on the topic goes there
    // (jumpToIndex), or to its last post past the end (jumpEnd).
    var dragging = false;
    var dragOffset = null;
    var percentage = null;
    var scroller = area ? area.querySelector(".timeline-scroller") : null;

    function updatePercentage(event) {
      event.preventDefault();
      var desired = event.pageY - (dragOffset || 0);
      var areaTop = area.getBoundingClientRect().top + window.scrollY;
      var scrollerHeight = scroller.offsetHeight;
      var scrollableTop = areaTop + scrollerHeight / 2;
      var scrollableHeight = area.offsetHeight - scrollerHeight;
      percentage = clamp((desired - scrollableTop) / scrollableHeight, 0, 1);
      commit();
    }

    function commit() {
      render(percentage);
      if (dragging) {
        return;
      }
      var scrollPosition = clamp(Math.floor(total * percentage), 0, total) + 1;
      if (current === scrollPosition) {
        jumpToIndex(current);
      } else {
        jumpEnd();
      }
    }

    // TopicController#_jumpToIndex and _jumpToPostId: a post the page
    // draws is gone to; another is loaded by id first (findPostsByIds, GET
    // /t/:id/posts.json) for its number, and the topic is routed to it,
    // which loads the posts around it.
    function jumpToIndex(index) {
      var postId = stream[clamp(index, 1, stream.length) - 1];
      if (!postId) {
        return;
      }
      var post = document.querySelector('#posts article[data-post-id="' + postId + '"]');
      if (post) {
        goToPost(parseInt(post.parentElement.dataset.postNumber, 10));
        return;
      }
      var url = container.dataset.topicUrl;
      var topicId = url.split("/").pop();
      var base = (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
      fetch(base + "/t/" + topicId + "/posts.json?post_ids%5B%5D=" + postId, {
        credentials: "same-origin",
        headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
      })
        .then(function (r) {
          if (!r.ok) {
            throw new Error("/t/" + topicId + "/posts.json: " + r.status);
          }
          return r.json();
        })
        .then(function (json) {
          var found = json.post_stream && json.post_stream.posts && json.post_stream.posts[0];
          if (found) {
            routeToPost(url + "/" + found.post_number);
          }
        })
        .catch(function (e) {
          console.error(e);
        });
    }

    // jumpEnd: the topic's last post (lastPostUrl, the now date's link).
    function jumpEnd() {
      var now = container.querySelector(".now-date");
      var m = now && /\/(\d+)$/.exec(now.getAttribute("href"));
      if (!m) {
        return;
      }
      var number = parseInt(m[1], 10);
      if (document.getElementById("post_" + number)) {
        goToPost(number);
      } else {
        routeToPost(container.dataset.topicUrl + "/" + number);
      }
    }

    if (area && scroller) {
      area.addEventListener("click", function (event) {
        if (event.target.closest(".timeline-padding")) {
          updatePercentage(event);
        }
      });
      container.querySelectorAll(".start-date, .now-date").forEach(function (link) {
        link.addEventListener("click", updatePercentage);
      });
      // d-pointer-drag on the scroller: the drag holds the pointer, marks
      // the area --dragging and the body dragging, and commits on release
      // (or on a cancel, cancelCommits).
      var pointerId = null;
      var endDrag = function () {
        if (pointerId === null) {
          return;
        }
        try {
          scroller.releasePointerCapture(pointerId);
        } catch (e) {
          // Already released.
        }
        pointerId = null;
        dragging = false;
        dragOffset = null;
        area.classList.remove("--dragging");
        document.body.classList.remove("dragging");
        commit();
      };
      scroller.addEventListener("pointerdown", function (event) {
        if (event.button !== 0 || pointerId !== null) {
          return;
        }
        try {
          scroller.setPointerCapture(event.pointerId);
        } catch (e) {
          return;
        }
        pointerId = event.pointerId;
        var rect = scroller.getBoundingClientRect();
        dragOffset = event.pageY - (rect.top + window.scrollY + scroller.offsetHeight / 2);
        dragging = true;
        area.classList.add("--dragging");
        document.body.classList.add("dragging");
      });
      scroller.addEventListener("pointermove", function (event) {
        if (event.pointerId !== pointerId) {
          return;
        }
        event.stopPropagation();
        updatePercentage(event);
      });
      scroller.addEventListener("pointerup", endDrag);
      scroller.addEventListener("pointercancel", endDrag);
      scroller.addEventListener("lostpointercapture", endDrag);
    }

    // Where the scroller starts (calculatePosition from the constructor):
    // _percentFor the entered post's progress index (enteredIndex), the
    // post nearest the URL's post number, else the first.
    function enteredPercent() {
      var m = location.pathname.match(/\/t\/[^/]+\/\d+\/(\d+)/);
      var near = m ? parseInt(m[1], 10) : 1;
      var closest = null;
      var distance = Infinity;
      document.querySelectorAll("#posts > [data-post-number]").forEach(function (el) {
        var d = Math.abs(parseInt(el.dataset.postNumber, 10) - near);
        var article = el.querySelector("article[data-post-id]");
        if (article && d < distance) {
          distance = d;
          closest = article;
        }
      });
      if (!closest) {
        return null;
      }
      var index = stream.indexOf(parseInt(closest.dataset.postId, 10)) + 1;
      // The switch _percentFor has, on the one-based index it is given.
      if (index === 0) {
        return 0;
      }
      if (index === total - 1) {
        return 1;
      }
      return clamp(index / total, 0, 1);
    }

    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", resize);
    resize();
    var entered = enteredPercent();
    if (entered !== null) {
      render(entered);
    }
    return function () {
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", resize);
    };
  }
})();
