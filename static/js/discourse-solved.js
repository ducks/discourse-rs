// discourse-solved on the topic page: the post menu's Solved button accepts
// or unaccepts the answer (SolvedAcceptAnswerButton,
// SolvedUnacceptAnswerButton), the posts coming back redrawn over the live
// stream; the accepted answers' accordion (DPostAccordion) expands,
// collapses and measures whether its quotes overflow; and NoAnswer's
// popup shows after a moment unless dismissed; "Me too" toggles the
// member's shared issue. The confetti on a toggle is not ported.
(function () {
  "use strict";

  if (!Discourse.once("discourse-solved")) {
    return;
  }

  function csrfHeaders() {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    headers["Content-Type"] = "application/x-www-form-urlencoded";
    headers["X-Requested-With"] = "XMLHttpRequest";
    return headers;
  }

  function basePath() {
    var timeline = document.querySelector(".timeline-container[data-topic-url]");
    var url = timeline ? timeline.dataset.topicUrl : "";
    var at = url.indexOf("/t/");
    return at > 0 ? url.slice(0, at) : "";
  }

  // acceptPost / unacceptPost; errors as popupAjaxError shows them.
  function toggleSolution(button, path) {
    var article = button.closest("article[data-post-id]");
    if (!article || button.disabled) {
      return;
    }
    button.disabled = true;
    fetch(basePath() + path, {
      method: "POST",
      credentials: "same-origin",
      headers: csrfHeaders(),
      body: new URLSearchParams({ id: article.dataset.postId }).toString(),
    })
      .then(function (r) {
        if (r.ok) {
          return;
        }
        return r
          .json()
          .then(function (body) {
            window.alert((body.errors || [r.statusText]).join("\n"));
          })
          .catch(function () {
            window.alert(r.statusText);
          });
      })
      .finally(function () {
        button.disabled = false;
      });
  }

  // DPostAccordionItem: expanded or not, its toggle drawn to match.
  function setExpanded(item, expanded) {
    var aside = item.closest(".d-post-accordion");
    if (expanded) {
      item.setAttribute("data-expanded", "");
    } else {
      item.removeAttribute("data-expanded");
    }
    var toggle = item.querySelector(".d-post-accordion-item__toggle");
    if (!toggle) {
      return;
    }
    var label = expanded ? aside.dataset.labelCollapse : aside.dataset.labelExpand;
    toggle.setAttribute("aria-expanded", String(expanded));
    toggle.setAttribute("aria-label", label);
    toggle.setAttribute("title", label);
    var use = toggle.querySelector("use");
    var svg = toggle.querySelector("svg");
    var icon = expanded ? "chevron-up" : "chevron-down";
    if (use && svg) {
      use.setAttribute("href", "#" + icon);
      svg.setAttribute("class", svg.getAttribute("class").replace(/d-icon-chevron-(up|down)/, "d-icon-" + icon));
    }
    measure(item);
  }

  // checkOverflow: the quote overflows its lines while expanded; a
  // collapsed one doesn't.
  function measure(item) {
    var quote = item.querySelector(".d-post-accordion-item__content");
    if (!quote) {
      return;
    }
    var overflowing =
      item.hasAttribute("data-expanded") && quote.scrollHeight > quote.clientHeight + 1;
    item.setAttribute("data-overflowing", String(overflowing));
  }

  function measureAll() {
    document.querySelectorAll(".d-post-accordion-item").forEach(function (item) {
      measure(item);
      if (window.ResizeObserver && !item.dataset.observed) {
        item.dataset.observed = "";
        var quote = item.querySelector(".d-post-accordion-item__content");
        if (quote) {
          new ResizeObserver(function () {
            measure(item);
          }).observe(quote);
        }
      }
    });
  }

  document.addEventListener("click", function (event) {
    var accept = event.target.closest(".post-action-menu__solved-unaccepted");
    if (accept) {
      toggleSolution(accept, "/solution/accept");
      return;
    }
    var unaccept = event.target.closest(".post-action-menu__solved-accepted");
    if (unaccept) {
      toggleSolution(unaccept, "/solution/unaccept");
      return;
    }
    // onClickHeader: links keep their own way; the header toggles a quote
    // with content and jumps to one without.
    var header = event.target.closest(".d-post-accordion-item__header");
    if (header) {
      var item = header.closest(".d-post-accordion-item");
      if (event.target.closest(".d-post-accordion-item__toggle")) {
        setExpanded(item, !item.hasAttribute("data-expanded"));
      } else if (!event.target.closest("a")) {
        if (item.classList.contains("d-post-accordion-item--has-content")) {
          setExpanded(item, !item.hasAttribute("data-expanded"));
        } else {
          var jump = item.querySelector(".d-post-accordion-item__jump");
          if (jump) {
            location.href = jump.href;
          }
        }
      }
      return;
    }
    // SolvedSharedIssueButton#toggle: an anonymous reader logs in first;
    // the button comes back redrawn over the live stream.
    var shared = event.target.closest(".post-action-menu__solved-shared-issue");
    if (shared) {
      if (shared.disabled) {
        return;
      }
      if (!document.body.getAttribute("hx-headers")) {
        location.href = basePath() + "/login";
        return;
      }
      var topic = document.querySelector("#topic[data-topic-id]");
      shared.disabled = true;
      fetch(basePath() + "/solution/shared_issue", {
        method: "POST",
        credentials: "same-origin",
        headers: csrfHeaders(),
        body: new URLSearchParams({ topic_id: topic.dataset.topicId }).toString(),
      })
        .then(function (r) {
          if (!r.ok) {
            return r
              .json()
              .then(function (body) {
                window.alert((body.errors || [r.statusText]).join("\n"));
              })
              .catch(function () {
                window.alert(r.statusText);
              });
          }
        })
        .finally(function () {
          shared.disabled = false;
        });
      return;
    }
    var close = event.target.closest(".topic-navigation-popup .close");
    if (close) {
      var popup = close.closest(".topic-navigation-popup");
      popup.hidden = true;
      var key = "discourse_dismiss_topic_nav_popup_" + popup.dataset.popupId;
      try {
        localStorage.setItem(key, String(Date.now() + Number(popup.dataset.dismissDuration)));
      } catch (e) {
        // Storage refused: it shows again next time.
      }
    }
  });

  // TopicNavigationPopup: unless dismissed and not yet expired, after
  // NoAnswer's delay.
  function showPopups() {
    document.querySelectorAll(".topic-navigation-popup[data-popup-id]").forEach(function (popup) {
      var key = "discourse_dismiss_topic_nav_popup_" + popup.dataset.popupId;
      var value = null;
      try {
        value = localStorage.getItem(key);
      } catch (e) {
        value = null;
      }
      if (value === "true" || Number(value) > Date.now()) {
        return;
      }
      try {
        localStorage.removeItem(key);
      } catch (e) {
        // Nothing stored to forget.
      }
      setTimeout(function () {
        popup.hidden = false;
      }, 2000);
    });
  }

  // Each page's accordions and popup (page.js); posts redrawn over the
  // live stream bring new accordions.
  Discourse.onPage(function () {
    measureAll();
    showPopups();
  });
  document.body.addEventListener("htmx:oobAfterSwap", measureAll);
})();
