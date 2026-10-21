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

  // The post at the eyeline, which slides from the top of the posts to the
  // bottom of the window as the reader nears the end of the page: its
  // index in the stream, and how far through the topic that is
  // (TopicController#currentPostScrolled's percent). Null between posts.
  function position(stream) {
    var total = Math.max(stream.length, 1);
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
        return { index: index, percent: clamp((index + within - 1) / total, 0, 1) };
      }
    }
    return null;
  }

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
    // Mark unread (deferTopic): stop timing the page, unread its last
    // post, then go home (or to the message's inbox).
    var defer = event.target.closest("[data-defer-url]");
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
          var number = nav.dataset.lastReadNumber;
          var post = document.querySelector('#posts article[data-post-number="' + number + '"]');
          if (post) {
            window.scrollTo({ top: post.getBoundingClientRect().top + window.scrollY - headerHeight() });
          } else {
            var timeline = document.querySelector("template.topic-timeline-fullscreen");
            var url = timeline ? timeline.content.firstElementChild.dataset.topicUrl : location.pathname;
            location.href = url + "/" + number;
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
    var chunk = parseInt(container.dataset.chunkSize, 10) || 20;
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

    // A click on the scroll area: to that post, on this page or its own.
    if (area) {
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
