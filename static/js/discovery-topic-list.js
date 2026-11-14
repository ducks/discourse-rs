// The discovery lists loading their next page as the reader nears the end
// (components/discovery/topics.gjs loadMore, models/topic-list.js
// loadMore, DLoadMore with LOAD_MORE_ROOT_MARGIN). The list's sentinel
// carries more_topics_url; when it comes within 600px of the window that
// page is asked for, its rows not already in the list are appended (the
// first of them highlighted), and the sentinel takes the page's
// more_topics_url. While it loads the footer's spinner shows. Once there
// is no more to load the footer is the last page's (its dismiss buttons)
// and the page footer shows. With room left in the window it loads on.
(function () {
  "use strict";

  if (!Discourse.once("discovery-topic-list")) {
    return;
  }

  var ROOT_MARGIN = "0px 0px 600px 0px";
  var DEBOUNCE_MS = 100;

  function spinner(container, on) {
    if (!container) {
      return;
    }
    container.classList.toggle("visible", on);
    if (on) {
      container.setAttribute("data-loading", "true");
      container.innerHTML = '<div class="spinner "></div>';
    } else {
      container.removeAttribute("data-loading");
      container.innerHTML = "";
    }
  }

  // TopicListItem#highlightRow
  function highlight(row) {
    row.addEventListener(
      "animationend",
      function () {
        row.classList.remove("highlighted");
      },
      { once: true }
    );
    row.classList.add("highlighted");
  }

  // The discovery controller's query params, in the order Ember writes them.
  var QUERY_PARAMS = [
    "order", "ascending", "status", "state", "search", "max_posts", "min_posts", "q", "before",
    "bumped_before", "f", "subset", "period", "topic_ids", "group_name", "tags", "match_all_tags",
    "no_subcategories", "no_tags", "exclude_tag",
  ];

  // NewListHeaderControlsWrapper#click: changeNewListSubset sets the
  // `subset` query param (replace: true), which reloads the list from
  // its first page; the other query params stay.
  document.addEventListener("click", function (event) {
    var button = event.target.closest(".topic-replies-toggle-wrapper button.topics-replies-toggle");
    if (!button) {
      return;
    }
    event.preventDefault();
    var subset = button.classList.contains("--topics") ? "topics" : button.classList.contains("--replies") ? "replies" : null;
    var current = new URLSearchParams(location.search);
    if ((current.get("subset") || null) === subset) {
      return;
    }
    var params = new URLSearchParams();
    QUERY_PARAMS.forEach(function (name) {
      if (name === "subset" ? subset : current.has(name)) {
        params.set(name, name === "subset" ? subset : current.get(name));
      }
    });
    var query = params.toString();
    var url = location.pathname + (query ? "?" + query : "");
    if (!window.htmx) {
      window.location.replace(url);
      return;
    }
    var link = document.createElement("a");
    link.href = url;
    link.hidden = true;
    link.setAttribute("hx-replace-url", "true");
    document.getElementById("main-outlet").appendChild(link);
    window.htmx.process(link);
    link.click();
  });

  Discourse.onPage(function () {
    var sentinel =document.querySelector("#list-area .load-more-sentinel[data-more-url]");
    var body = document.querySelector("#list-area tbody.topic-list-body");
    if (!sentinel || !body) {
      return;
    }
    var loading = false;
    var gone = false;
    var observer = null;
    var timer = null;

    function process(node) {
      if (window.htmx) {
        window.htmx.process(node);
      }
    }

    function loadMore() {
      var url = sentinel.dataset.moreUrl;
      if (loading || gone || !url) {
        return;
      }
      // Discovery::Topics#loadMore
      if (Discourse.documentTitle) {
        Discourse.documentTitle.updateContextCount(0);
      }
      loading = true;
      // DLoadMore's isLoading: no observer while the page loads.
      if (observer) {
        observer.disconnect();
      }
      var footer = document.querySelector("#list-area footer.topic-list-bottom");
      spinner(footer && footer.querySelector(":scope > .loading-container"), true);
      fetch(url, {
        credentials: "same-origin",
        headers: { Accept: "text/html", "X-Requested-With": "XMLHttpRequest" },
      })
        .then(function (r) {
          if (!r.ok) {
            throw new Error(url + ": " + r.status);
          }
          return r.text();
        })
        .then(function (html) {
          if (gone) {
            return;
          }
          var doc = new DOMParser().parseFromString(html, "text/html");
          var have = {};
          body.querySelectorAll(":scope > tr[data-topic-id]").forEach(function (row) {
            have[row.dataset.topicId] = true;
          });
          var added = 0;
          doc.querySelectorAll("#list-area tbody.topic-list-body > tr[data-topic-id]").forEach(function (row) {
            if (have[row.dataset.topicId]) {
              return;
            }
            var node = document.importNode(row, true);
            body.appendChild(node);
            process(node);
            if (added++ === 0) {
              highlight(node);
            }
          });
          var next = doc.querySelector("#list-area .load-more-sentinel[data-more-url]");
          if (next) {
            sentinel.dataset.moreUrl = next.dataset.moreUrl;
          } else {
            delete sentinel.dataset.moreUrl;
            // allLoaded: the footer as the last page draws it.
            var nextFooter = doc.querySelector("#list-area footer.topic-list-bottom");
            if (footer && nextFooter) {
              var node = document.importNode(nextFooter, true);
              // showEmptyFilterEducationInFooter: only for a list with no
              // topics at all, not an empty last page.
              if (body.querySelector(":scope > tr[data-topic-id]")) {
                node.querySelectorAll(".--empty-topic-filter").forEach(function (el) {
                  el.remove();
                });
              }
              footer.replaceWith(node);
              process(node);
              footer = null;
            }
            if (Discourse.showFooter) {
              Discourse.showFooter();
            }
          }
        })
        .catch(function (e) {
          console.error(e);
        })
        .then(function () {
          spinner(footer && footer.querySelector(":scope > .loading-container"), false);
          loading = false;
          if (gone || !sentinel.dataset.moreUrl) {
            return;
          }
          // Room left in the window for more topics: on to the next page.
          if (window.innerHeight >= document.documentElement.scrollHeight) {
            loadMore();
          } else {
            watch();
          }
        });
    }

    function watch() {
      if (observer) {
        observer.disconnect();
      }
      observer = new IntersectionObserver(
        function (entries) {
          entries.forEach(function (entry) {
            if (entry.isIntersecting && sentinel.dataset.moreUrl) {
              clearTimeout(timer);
              timer = setTimeout(loadMore, DEBOUNCE_MS);
            }
          });
        },
        { threshold: 0, rootMargin: ROOT_MARGIN }
      );
      observer.observe(sentinel);
    }

    watch();
    return function () {
      gone = true;
      clearTimeout(timer);
      if (observer) {
        observer.disconnect();
      }
    };
  });
})();
