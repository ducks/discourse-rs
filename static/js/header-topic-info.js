// The docked header over a topic: once the topic's title scrolls out of
// view, the header shows the topic (Header::Topic::Info) after the
// minimized logo, which the topic page carries in
// <template id="header-topic-info">. When, as Ember decides it:
//
//   services/header.js        topicInfoVisible, with its latch while
//                             scrolling down and, in the mobile view, the
//                             header back to normal while scrolling up
//   services/scroll-direction lastScrollDirection, paused for a second
//                             after the page comes in
//   components/topic-title    the title's IntersectionObserver (threshold
//                             1: any of it out of view, above the window)
//   routes/topic/from-params  enterTopic: the title counts as visible
//                             when the first post is loaded and no later
//                             post was asked for
//
// While docked, the logo is the minimized one, AuthButtons leaves out
// sign up, and the page's title is aria-hidden (titleIsVisibleOnHeader).
// The title and the more participants link go back to the first post;
// a participant's avatar opens their user card. Each page sets this up
// again: a boosted navigation brings a new header and a new topic.
(function () {
  "use strict";

  if (!Discourse.once("header-topic-info")) {
    return;
  }

  // scroll-direction.js
  var MOBILE_SCROLL_TOLERANCE = 5;
  var PAUSE_AFTER_TRANSITION_MS = 1000;
  var UNSCROLLED = "unscrolled";
  var SCROLLED_DOWN = "scroll-down";
  var SCROLLED_UP = "scroll-up";

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

  Discourse.onPage(function () {
    var template = document.getElementById("header-topic-info");
    var topicTitle = document.getElementById("topic-title");
    var outlet = document.querySelector(".d-header .home-logo-wrapper-outlet");
    if (!template || !topicTitle || !outlet || !("IntersectionObserver" in window)) {
      return;
    }
    var mobile = document.documentElement.classList.contains("mobile-view");

    // ScrollDirection, from routeDidChange.
    var direction = UNSCROLLED;
    var lastScroll = null;
    var bottomHit = 0;
    var paused = true;
    var pauseTimer = setTimeout(function () {
      paused = false;
    }, PAUSE_AFTER_TRANSITION_MS);
    var throttleTimer = null;
    var bottomTimer = null;

    function calculateDirection(offset) {
      var delta = Math.floor(offset - lastScroll);
      if (delta <= MOBILE_SCROLL_TOLERANCE && delta >= -MOBILE_SCROLL_TOLERANCE) {
        return;
      }
      if (offset === 0) {
        setDirection(UNSCROLLED);
        lastScroll = 0;
        return;
      }
      var current = delta > 0 ? SCROLLED_DOWN : SCROLLED_UP;
      var distanceToBottom = Math.floor(document.body.clientHeight - offset - window.innerHeight);
      if (offset < 0) {
        setDirection(UNSCROLLED);
      } else if (current !== direction && distanceToBottom > 0) {
        setDirection(current);
      }
      lastScroll = Math.floor(offset);
      if (distanceToBottom > 0) {
        bottomHit = 0;
      } else {
        // The very bottom resets the direction on a second scroll down.
        clearTimeout(bottomTimer);
        bottomTimer = setTimeout(function () {
          bottomHit = 1;
        }, 1000);
        if (bottomHit === 1) {
          setDirection(UNSCROLLED);
        }
      }
    }

    function onScroll() {
      if (paused) {
        lastScroll = window.scrollY;
        return;
      }
      // throttle(handleScroll, 100, false): once, at the end of the window.
      if (!throttleTimer) {
        throttleTimer = setTimeout(function () {
          throttleTimer = null;
          calculateDirection(window.scrollY);
        }, 100);
      }
    }

    // Header: the title's visibility and the latch.
    var mainTopicTitleVisible = false;
    var latched = false;

    function setDirection(value) {
      direction = value;
      render();
    }

    function setTitleVisible(value) {
      mainTopicTitleVisible = value;
      if (!value) {
        latched = true;
      } else if (direction === SCROLLED_UP) {
        latched = false;
      }
      render();
    }

    function topicInfoVisible() {
      var isLatched = latched && direction === SCROLLED_DOWN;
      if (mainTopicTitleVisible && !isLatched) {
        return false;
      }
      if (mobile && direction === SCROLLED_UP) {
        return false;
      }
      return true;
    }

    // What docking swaps in and out.
    var logo = outlet.querySelector(".title");
    var minimized = template.content.querySelector(".title").cloneNode(true);
    var info = template.content.querySelector(".extra-info-wrapper").cloneNode(true);
    var signUp = document.querySelector(".d-header .auth-buttons .sign-up-button");
    var signUpNext = signUp ? signUp.nextSibling : null;
    var signUpParent = signUp ? signUp.parentNode : null;
    var docked = false;
    var processed = false;

    function render() {
      // titleIsVisibleOnHeader
      var heading = topicTitle.querySelector("h1[data-topic-id]");
      if (heading) {
        if (mainTopicTitleVisible) {
          heading.removeAttribute("aria-hidden");
        } else {
          heading.setAttribute("aria-hidden", "true");
        }
      }
      var visible = topicInfoVisible();
      if (visible === docked) {
        return;
      }
      docked = visible;
      if (visible) {
        if (logo && logo.parentNode) {
          logo.replaceWith(minimized);
        }
        outlet.after(info);
        // Boosted once in the page, where they inherit hx-boost.
        if (!processed && window.htmx) {
          processed = true;
          window.htmx.process(minimized);
          window.htmx.process(info);
        }
        if (signUp && signUp.parentNode) {
          signUp.remove();
        }
      } else {
        if (logo && minimized.parentNode) {
          minimized.replaceWith(logo);
        }
        info.remove();
        if (signUp && signUpParent && !signUp.parentNode) {
          signUpParent.insertBefore(signUp, signUpNext && signUpNext.parentNode === signUpParent ? signUpNext : null);
        }
      }
    }

    // jumpToTopPost: the first post, scrolled to the top when the page has
    // it (DiscourseURL.routeTo the topic's first post, which jumpToPost
    // scrolls to 0); otherwise the boosted link loads the topic's start.
    function jumpToTopPost(event) {
      if (wantsNewWindow(event)) {
        return;
      }
      if (!document.getElementById("post_1")) {
        return;
      }
      event.preventDefault();
      event.stopImmediatePropagation();
      var url = event.currentTarget.getAttribute("href") + "/1";
      history.replaceState(history.state, "", url);
      window.scrollTo(window.scrollX, 0);
    }
    info.querySelectorAll(".topic-link, .more-participants").forEach(function (link) {
      link.addEventListener("click", jumpToTopPost, true);
    });

    // Participant: topic-header:trigger-user-card and
    // topic-header:trigger-group-card. The group card is not ported.
    info.querySelectorAll(".trigger-user-card a, .trigger-group-card a").forEach(function (link) {
      link.addEventListener(
        "click",
        function (event) {
          event.preventDefault();
          event.stopImmediatePropagation();
          if (link.parentNode.classList.contains("trigger-group-card")) {
            var error = document.getElementById("page-error");
            if (error) {
              error.textContent = "Not ported yet: the group card.";
              error.hidden = false;
            }
          } else if (Discourse.showUserCard) {
            Discourse.showUserCard(link.getAttribute("title"), link);
          }
        },
        true
      );
    });

    // enterTopic(topic, isLoadingFirstPost)
    var nearPost = location.pathname.match(/\/t\/[^/]+\/\d+\/(\d+)/);
    var firstPostPresent = !!document.getElementById("post_1");
    latched = false;
    setTitleVisible(firstPostPresent && !(nearPost && parseInt(nearPost[1], 10) > 1));

    var observer = new IntersectionObserver(
      function (entries) {
        entries.forEach(function (entry) {
          // In the window or below it (a small window, a tall header).
          setTitleVisible(entry.isIntersecting || entry.boundingClientRect.top > 0);
        });
      },
      { threshold: 1 }
    );
    observer.observe(topicTitle);
    window.addEventListener("scroll", onScroll, { passive: true });

    return function () {
      observer.disconnect();
      window.removeEventListener("scroll", onScroll);
      clearTimeout(pauseTimer);
      clearTimeout(throttleTimer);
      clearTimeout(bottomTimer);
    };
  });
})();
