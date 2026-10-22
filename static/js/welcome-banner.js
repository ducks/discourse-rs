// WelcomeBanner's checkViewport: the body's welcome-banner--visible while
// the banner's search is usable, any part of it below the header
// (lib/offset-calculator's headerOffset). An IntersectionObserver only
// says when to look again, as the banner crosses the header. The server
// draws the class on a desktop page that can search; this keeps it
// current as the page scrolls, narrows (the banner's search is hidden
// below 40rem) or goes without search.
(function () {
  "use strict";

  // Swapped-in pages run this again; the page hook stays from the first.
  if (!Discourse.once("welcome-banner")) {
    return;
  }

  function headerOffset() {
    return parseInt(document.documentElement.style.getPropertyValue("--header-offset"), 10) || 0;
  }

  Discourse.onPage(function () {
    var banner = document.querySelector(".welcome-banner");
    if (!banner || !document.body.classList.contains("welcome-banner--enabled")) {
      return;
    }
    // Without site.can_search the banner has no search, and the class stays off.
    var searchMenu = banner.querySelector(".welcome-banner__search-menu");
    function check() {
      var usable = !!searchMenu && searchMenu.getBoundingClientRect().bottom > headerOffset();
      document.body.classList.toggle("welcome-banner--visible", usable);
    }
    var threshold = [];
    for (var i = 0; i <= 100; i++) {
      threshold.push(i / 100);
    }
    var observer = new IntersectionObserver(check, { threshold: threshold });
    observer.observe(banner);
    return function () {
      observer.disconnect();
      document.body.classList.remove("welcome-banner--visible");
    };
  });
})();
