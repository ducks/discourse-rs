// Page navigation without reloads, as Ember's router gives Discourse: the
// body's links (hx-boost) fetch the next page and swap its body in, so
// what lives across pages (the live stream's connection, an open composer
// or chat) can stay, and history works as usual. This script loads once,
// in the head, and gives the page scripts what that needs:
//
//   Discourse.once(name)  true the first time a script runs, so its
//                         document-wide listeners are added once however
//                         often its <script> tag comes back in a page.
//   Discourse.onPage(fn)  runs fn for this page now and for every page
//                         swapped in after; fn may return a cleanup, run
//                         before the page is swapped out.
//   Discourse.swapped     true once a page has been swapped in: the page
//                         came by a navigation inside the app, not a load.
//   Discourse.refresh()   the router's refresh(): this page asked for again
//                         and swapped in where it stands, no history entry.
//   Discourse.showFooter() the page footer a list or topic held back until
//                         it loaded to its end.
//
// What stays across pages is marked hx-preserve (the composer); it takes
// the new page's values of the attributes it lists in data-page-attrs.
//
// It also brings over what a swap doesn't (the body's classes and
// attributes), swaps in error pages as a full load shows them, and leaves
// anything that isn't a page (an upload, a feed) to the browser.
//
// And it keeps the layout in step with the viewport, as Ember re-renders
// when the window crosses 40rem: the page is asked for again in the other
// layout and swapped in where it stands.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  var loaded = {};
  var pages = [];
  var swapping = false;

  // sniff-capabilities: touch when any pointer is coarse, and the iOS
  // classes, on each page's <html> (the server renders no-touch).
  function capabilities() {
    var html = document.documentElement;
    var ua = navigator.userAgent;
    var touch = window.matchMedia("(any-pointer: coarse)").matches;
    if (touch) {
      html.classList.remove("no-touch", "discourse-no-touch");
      html.classList.add("touch", "discourse-touch");
    }
    var ipados = ua.indexOf("Mac OS") !== -1 && !/iPhone|iPod/.test(ua) && touch;
    if (ipados) {
      html.classList.add("ipados-device");
    }
    if (/iPhone|iPod/.test(ua) || ipados) {
      html.classList.add("ios-device");
    }
  }
  capabilities();

  Discourse.once = function (name) {
    if (loaded[name]) {
      return false;
    }
    loaded[name] = true;
    return true;
  };

  // The footer service's showFooter turning on for a page that hid the
  // footer only until it loaded the rest of its list or topic: the
  // footer waits in its templates (Chrome::defer_footer).
  Discourse.showFooter = function () {
    document.querySelectorAll("template.deferred-footer").forEach(function (template) {
      template.replaceWith(template.content);
    });
  };

  function run(entry) {
    var cleanup = entry.fn();
    entry.cleanup = typeof cleanup === "function" ? cleanup : null;
  }

  Discourse.onPage = function (fn) {
    var entry = { fn: fn, cleanup: null };
    pages.push(entry);
    // Scripts run after the markup they set up, so it is there now; mid
    // swap it runs with the others once the new page is in.
    if (!swapping) {
      run(entry);
    }
  };

  function parse(html) {
    return new DOMParser().parseFromString(html, "text/html");
  }

  function boosted(event) {
    return event.detail.boosted || (event.detail.requestConfig && event.detail.requestConfig.boosted);
  }

  // Before a page is swapped in: the one going away cleans up, and what
  // the body swap leaves behind (the html and body attributes) is brought
  // over from the new page.
  function leaving(doc) {
    swapping = true;
    pages.forEach(function (entry) {
      if (entry.cleanup) {
        entry.cleanup();
        entry.cleanup = null;
      }
    });
    // The <html> classes the scripts keep, as Ember's services do across
    // routes: the on-screen keyboard's, and the composer's state while the
    // composer stays (hx-preserve keeps it when the new page has one too).
    var html = document.documentElement;
    var composerStays = Boolean(doc.getElementById("reply-control"));
    var kept = ["keyboard-visible"]
      .concat(composerStays ? ["composer-open", "fullscreen-composer", "composer-has-preview"] : [])
      .filter(function (name) {
        return html.classList.contains(name);
      });
    html.className = doc.documentElement.className;
    kept.forEach(function (name) {
      html.classList.add(name);
    });
    if (!composerStays) {
      html.style.removeProperty("--composer-height");
    }
    capabilities();
    var body = document.body;
    Array.prototype.slice.call(body.attributes).forEach(function (attr) {
      if (!doc.body.hasAttribute(attr.name)) {
        body.removeAttribute(attr.name);
      }
    });
    Array.prototype.slice.call(doc.body.attributes).forEach(function (attr) {
      body.setAttribute(attr.name, attr.value);
    });
    // What stays (hx-preserve) keeps its state, but takes the new page's
    // values of the attributes it names in data-page-attrs (the composer's
    // default category).
    doc.querySelectorAll("[hx-preserve][id][data-page-attrs]").forEach(function (incoming) {
      var kept = document.getElementById(incoming.id);
      if (!kept) {
        return;
      }
      incoming.dataset.pageAttrs.split(" ").forEach(function (name) {
        if (incoming.hasAttribute(name)) {
          kept.setAttribute(name, incoming.getAttribute(name));
        } else {
          kept.removeAttribute(name);
        }
      });
    });
  }

  // After: the page scripts set up the new page.
  function arrived() {
    if (!swapping) {
      return;
    }
    swapping = false;
    anchor = null;
    Discourse.swapped = true;
    pages.forEach(run);
  }

  // A page is asked for as the browser asks for one: not as XHR, which
  // the server answers with JSON (a member's hx-headers say so for their
  // other requests). The page it leaves is kept for the pages that
  // return to it (the login and signup routes' destination_url).
  //
  // The link keeps its history push as an attribute: htmx decides to push
  // from the element's internal `boosted` flag once the response is in,
  // and an out-of-band swap replacing the link meanwhile (the live
  // stream's sidebar or nav redraw) cleans that flag off, so the page
  // arrived with the address bar left on the page before.
  document.addEventListener("htmx:configRequest", function (event) {
    if (boosted(event)) {
      Discourse.previousUrl = location.pathname + location.search;
      delete event.detail.headers["X-Requested-With"];
      var elt = event.detail.elt;
      if (elt && !elt.hasAttribute("hx-push-url")) {
        elt.setAttribute("hx-push-url", "true");
      }
    }
  });

  document.addEventListener("htmx:beforeSwap", function (event) {
    if (!boosted(event)) {
      return;
    }
    var xhr = event.detail.xhr;
    // Not a page: the browser goes there itself.
    var type = xhr.getResponseHeader("Content-Type") || "";
    if (type.indexOf("text/html") !== 0) {
      event.detail.shouldSwap = false;
      location.href = xhr.responseURL || event.detail.pathInfo.requestPath;
      return;
    }
    // A not-found or error page is a page too.
    if (xhr.status >= 400) {
      event.detail.shouldSwap = true;
      event.detail.isError = false;
    }
    if (event.detail.shouldSwap) {
      leaving(parse(event.detail.serverResponse));
    }
  });

  // Right after the new body is in, before it paints, so stored state
  // (a collapsed sidebar section) doesn't flicker. A page asked for
  // before the window crossed 40rem is redrawn in the layout it wants.
  document.addEventListener("htmx:afterSwap", function (event) {
    if (boosted(event)) {
      arrived();
      checkView();
    }
  });

  // Back and forward: the page is asked for again (historyCacheSize 0)
  // and swapped in the same way; one that fails to load loads in full.
  document.addEventListener("htmx:historyCacheMissLoad", function (event) {
    leaving(parse(event.detail.response));
  });
  document.addEventListener("htmx:historyRestore", function () {
    arrived();
    checkView();
  });
  document.addEventListener("htmx:historyCacheMissLoadError", function () {
    location.reload();
  });

  // A request that never got an answer leaves the page as it was.
  document.addEventListener("htmx:sendError", function () {
    swapping = false;
  });

  // Site#mobileView: below 40rem the page is the mobile layout. The server
  // renders the one the mobile_view cookie names (crate::mobile, its guess
  // from the user agent without one), so the cookie follows the viewport,
  // and a page in the other layout (a wrong guess on a first visit, a
  // window crossing the breakpoint) is asked for again and redrawn.
  //
  // NarrowDesktop: the desktop view under 48rem has its own layout (no
  // sidebar, slide-in menus), the narrow_desktop cookie kept the same way.
  // And TopicNavigation's width query: under 925px a desktop topic page
  // shows the progress bar in the timeline's place, which the page picks
  // as it comes in, so a window crossing that width is redrawn too.
  var viewport = window.matchMedia("(min-width: 40rem)");
  var wide = window.matchMedia("(min-width: 48rem)");
  var timelineWidth = window.matchMedia("(min-width: 925px)");
  var generation = 0;
  var hidden = null;

  function wantedMobile() {
    return viewport.matches ? "0" : "1";
  }

  function wantedNarrow() {
    return viewport.matches && !wide.matches ? "1" : "0";
  }

  function wantedView() {
    return wantedMobile() + wantedNarrow();
  }

  function renderedView(doc) {
    return (
      (doc.documentElement.classList.contains("mobile-view") ? "1" : "0") +
      // The body's marker, or before the body is in, the head's.
      ((doc.body
        ? doc.body.hasAttribute("data-narrow-desktop")
        : !!doc.querySelector('meta[name="discourse-narrow-desktop"]'))
        ? "1"
        : "0")
    );
  }

  // A desktop topic page with the navigation its width doesn't want.
  function timelineStale() {
    if (document.documentElement.classList.contains("mobile-view")) {
      return false;
    }
    var nav = document.querySelector(".topic-navigation");
    if (!nav) {
      return false;
    }
    var progress = nav.classList.contains("with-topic-progress") || nav.classList.contains("topic-progress-expanded");
    return progress === timelineWidth.matches;
  }

  function syncView() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    var path = "; path=" + (meta ? meta.content : "") + "/; max-age=31536000; samesite=lax";
    document.cookie = "mobile_view=" + wantedMobile() + path;
    document.cookie = "narrow_desktop=" + wantedNarrow() + path;
    checkView();
  }

  // Only when the cookies took: a browser without cookies keeps the
  // server's guess instead of asking for the page forever. A redraw still
  // on its way is dropped when the window crosses back.
  function checkView() {
    generation += 1;
    var cookies = document.cookie.split("; ");
    var took =
      cookies.indexOf("mobile_view=" + wantedMobile()) !== -1 &&
      cookies.indexOf("narrow_desktop=" + wantedNarrow()) !== -1;
    if ((renderedView(document) !== wantedView() || timelineStale()) && took) {
      redraw(generation);
    } else {
      reveal();
    }
  }

  // A first visit in the wrong layout stays blank until the right one is
  // in, as it did while the page reloaded, rather than showing the other.
  function conceal() {
    hidden = document.createElement("style");
    hidden.textContent = "body { visibility: hidden; }";
    document.head.appendChild(hidden);
    setTimeout(reveal, 5000);
  }

  function reveal() {
    if (hidden) {
      hidden.remove();
      hidden = null;
    }
  }

  // When the page can't be asked for again: a reload, but not again for
  // a page that came back in the wrong layout just now.
  function reload() {
    var last = null;
    try {
      last = JSON.parse(sessionStorage.getItem("mobile-view-reload"));
    } catch (_) {}
    if (last && last.url === location.href && last.wanted === wantedView() && Date.now() - last.at < 5000) {
      reveal();
      return;
    }
    try {
      sessionStorage.setItem(
        "mobile-view-reload",
        JSON.stringify({ url: location.href, wanted: wantedView(), at: Date.now() })
      );
    } catch (_) {}
    location.reload();
  }

  // The page parsed and its deferred scripts (htmx) run.
  function parsed() {
    return new Promise(function (resolve) {
      if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", function () {
          resolve();
        });
      } else {
        resolve();
      }
    });
  }

  // Where the reader is: the first post or topic row below the header,
  // and how far down the window it is.
  function scrollAnchor() {
    if (window.scrollY === 0) {
      return null;
    }
    var header = document.querySelector(".d-header-wrap");
    var top = header ? header.getBoundingClientRect().bottom : 0;
    var rows = document.querySelectorAll("#posts article[data-post-id], tr[data-topic-id]");
    for (var i = 0; i < rows.length; i++) {
      var rect = rows[i].getBoundingClientRect();
      if (rect.bottom > top) {
        var name = rows[i].hasAttribute("data-post-id") ? "data-post-id" : "data-topic-id";
        return {
          selector: rows[i].tagName.toLowerCase() + "[" + name + '="' + rows[i].getAttribute(name) + '"]',
          top: rect.top,
        };
      }
    }
    return null;
  }

  // Kept as the reader scrolls, since by the time the window crossed the
  // breakpoint the old layout has already reflowed to the new width. The
  // first scroll at a new width is that reflow, not the reader's.
  var anchor = null;
  var anchorWidth = window.innerWidth;
  var anchorFrame = null;
  window.addEventListener(
    "scroll",
    function () {
      if (anchorFrame) {
        return;
      }
      anchorFrame = requestAnimationFrame(function () {
        anchorFrame = null;
        if (window.innerWidth === anchorWidth) {
          anchor = scrollAnchor();
        }
        anchorWidth = window.innerWidth;
      });
    },
    { passive: true }
  );

  function redraw(gen) {
    var url = location.href;
    if (document.readyState === "loading") {
      conceal();
    }
    fetch(url, {
      credentials: "same-origin",
      cache: "no-store",
      headers: { "HX-Request": "true", "HX-Boosted": "true", "HX-Current-URL": url },
    })
      .then(function (response) {
        var type = response.headers.get("Content-Type") || "";
        if (!response.ok || type.indexOf("text/html") !== 0) {
          throw new Error("redraw " + response.status);
        }
        return response.text();
      })
      .then(function (html) {
        return parsed().then(function () {
          return html;
        });
      })
      .then(function (html) {
        // Superseded: the window crossed back, or the reader went on to
        // another page, which came in the layout the cookie names.
        if (gen !== generation || location.href !== url) {
          return;
        }
        var doc = parse(html);
        if (renderedView(doc) !== wantedView()) {
          reveal();
          return;
        }
        swapIn(html, doc);
        reveal();
      })
      .catch(function () {
        if (gen === generation) {
          reload();
        }
      });
  }

  // Swapped in as a boosted navigation swaps a page, where the reader
  // is: the focused field keeps its caret and the post being read stays
  // put. What stays (the composer) hears discourse:redraw with its markup
  // in the new layout once the page is in. The document hears
  // discourse:before-redraw and discourse:redrawn around the swap, for
  // what Ember keeps open across a layout switch (the search menu).
  function swapIn(html, doc) {
    var reading = anchor;
    var focused = document.activeElement;
    var selection = null;
    try {
      selection = [focused.selectionStart, focused.selectionEnd];
    } catch (_) {}
    var kept = [];
    doc.querySelectorAll("[hx-preserve][id]").forEach(function (incoming) {
      var element = document.getElementById(incoming.id);
      if (element) {
        kept.push([element, document.importNode(incoming, true)]);
      }
    });
    document.dispatchEvent(new CustomEvent("discourse:before-redraw"));
    leaving(doc);
    htmx.swap(document.body, html, { swapStyle: "innerHTML", swapDelay: 0, settleDelay: 0 });
    arrived();
    document.dispatchEvent(new CustomEvent("discourse:redrawn"));
    kept.forEach(function (pair) {
      pair[0].dispatchEvent(new CustomEvent("discourse:redraw", { detail: pair[1] }));
    });
    if (focused && focused !== document.body && focused.isConnected) {
      focused.focus({ preventScroll: true });
      if (selection && typeof selection[0] === "number") {
        focused.setSelectionRange(selection[0], selection[1]);
      }
    }
    var target = reading && document.querySelector(reading.selector);
    if (target) {
      window.scrollBy(0, target.getBoundingClientRect().top - reading.top);
    }
    // Still the post the reader was on, so crossing back returns to it; the
    // scroll just made isn't theirs.
    anchor = target ? reading : scrollAnchor();
    anchorWidth = -1;
  }

  // A list once its topics are dismissed (topic-dismiss.js).
  Discourse.refresh = function () {
    generation += 1;
    redraw(generation);
  };

  syncView();
  viewport.addEventListener("change", syncView);
  wide.addEventListener("change", syncView);
  timelineWidth.addEventListener("change", syncView);
})();
