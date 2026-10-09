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
//
// What stays across pages is marked hx-preserve (the composer); it takes
// the new page's values of the attributes it lists in data-page-attrs.
//
// It also brings over what a swap doesn't (the body's classes and
// attributes), swaps in error pages as a full load shows them, and leaves
// anything that isn't a page (an upload, a feed) to the browser.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  var loaded = {};
  var pages = [];
  var swapping = false;

  Discourse.once = function (name) {
    if (loaded[name]) {
      return false;
    }
    loaded[name] = true;
    return true;
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

  function boosted(event) {
    return event.detail.boosted || (event.detail.requestConfig && event.detail.requestConfig.boosted);
  }

  // Before a page is swapped in: the one going away cleans up, and what
  // the body swap leaves behind (the html and body attributes) is brought
  // over from the new page.
  function leaving(html) {
    swapping = true;
    pages.forEach(function (entry) {
      if (entry.cleanup) {
        entry.cleanup();
        entry.cleanup = null;
      }
    });
    var doc = new DOMParser().parseFromString(html, "text/html");
    document.documentElement.className = doc.documentElement.className;
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
    pages.forEach(run);
  }

  // A page is asked for as the browser asks for one: not as XHR, which
  // the server answers with JSON (a member's hx-headers say so for their
  // other requests).
  document.addEventListener("htmx:configRequest", function (event) {
    if (boosted(event)) {
      delete event.detail.headers["X-Requested-With"];
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
      leaving(event.detail.serverResponse);
    }
  });

  // Right after the new body is in, before it paints, so stored state
  // (a collapsed sidebar section) doesn't flicker.
  document.addEventListener("htmx:afterSwap", function (event) {
    if (boosted(event)) {
      arrived();
    }
  });

  // Back and forward: the page is asked for again (historyCacheSize 0)
  // and swapped in the same way; one that fails to load loads in full.
  document.addEventListener("htmx:historyCacheMissLoad", function (event) {
    leaving(event.detail.response);
  });
  document.addEventListener("htmx:historyRestore", arrived);
  document.addEventListener("htmx:historyCacheMissLoadError", function () {
    location.reload();
  });

  // A request that never got an answer leaves the page as it was.
  document.addEventListener("htmx:sendError", function () {
    swapping = false;
  });
})();
