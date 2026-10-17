// The category management lists in the browser (src/admin_category_
// management_view.rs draws them): AdminCategoryManagementList's filter
// (its text, after the input delay, and its visibility), reset, and
// DLoadMore's next pages. The page draws each page of rows (`fragment`)
// from ListAdminCategories, as the component asks the categories
// endpoint.
(function () {
  "use strict";

  if (!Discourse.once("admin-category-management")) {
    return;
  }

  function ui() {
    return Discourse.adminUi;
  }

  function list() {
    return document.querySelector(".admin-contents .admin-category-management-list[data-type]");
  }

  var page = 0;
  var loading = false;
  var timer = null;
  var observer = null;

  function query(el, nextPage) {
    var input = el.querySelector(".d-filter-controls__input");
    var select = el.querySelector(".d-filter-controls__dropdown");
    var params = new URLSearchParams();
    params.set("fragment", "1");
    params.set("page", String(nextPage));
    if (input.value) {
      params.set("filter", input.value);
    }
    if (select.value !== "all") {
      params.set("visibility", select.value);
    }
    return location.pathname + "?" + params.toString();
  }

  function fetchPage(el, nextPage) {
    return fetch(query(el, nextPage), {
      credentials: "same-origin",
      headers: { "X-Requested-With": "fragment" },
    }).then(function (response) {
      if (!response.ok) {
        throw new Error(response.statusText);
      }
      el.dataset.hasMore = response.headers.get("x-has-more") || "false";
      return response.text();
    });
  }

  // The list drawn again for the filters, from its first page.
  function reload() {
    var el = list();
    if (!el) {
      return;
    }
    var input = el.querySelector(".d-filter-controls__input");
    var select = el.querySelector(".d-filter-controls__dropdown");
    el.querySelector(".d-filter-controls > .d-filter-controls__reset").disabled =
      !input.value && select.value === "all";
    Array.prototype.forEach.call(select.options, function (o) {
      o.classList.toggle("--selected", o.selected);
    });
    page = 0;
    loading = true;
    fetchPage(el, 0)
      .then(function (html) {
        el.querySelectorAll(
          ":scope > div:not(.d-filter-controls):not(.d-page-subheader):not(.loading-container), :scope > .admin-category-management-list__empty, :scope > .d-filter-controls__no-results"
        ).forEach(function (old) {
          old.remove();
        });
        var filtering = input.value || select.value !== "all";
        if (html) {
          el.insertAdjacentHTML("beforeend", html);
        } else if (filtering) {
          // DFilterControls' no results.
          var reset = ui().escapeHtml(ui().t("filter_controls.reset"));
          el.insertAdjacentHTML(
            "beforeend",
            '<div class="d-filter-controls__no-results"><p>' +
              ui().escapeHtml(el.dataset.noResults) +
              '</p><button aria-label="' +
              reset +
              '" class="btn btn-icon-text btn-default d-filter-controls__reset" title="' +
              reset +
              '" type="button">' +
              ui().icon("arrow-rotate-left") +
              '<span class="d-button-label">' +
              reset +
              "</span></button></div>"
          );
        } else {
          el.insertAdjacentHTML(
            "beforeend",
            '<p class="admin-category-management-list__empty">' + ui().escapeHtml(el.dataset.noResults) + "</p>"
          );
        }
        watch();
      }, ui().failed)
      .then(function () {
        loading = false;
      });
  }

  // DLoadMore: the next page as the table's end comes into view.
  function watch() {
    if (observer) {
      observer.disconnect();
      observer = null;
    }
    var el = list();
    var body = el && el.querySelector(".admin-category-management-list__table tbody");
    if (!body || !window.IntersectionObserver) {
      return;
    }
    observer = new IntersectionObserver(
      function (entries) {
        if (!entries[0].isIntersecting || loading || el.dataset.hasMore !== "true") {
          return;
        }
        loading = true;
        fetchPage(el, page + 1)
          .then(function (rows) {
            page += 1;
            body.insertAdjacentHTML("beforeend", rows);
            watch();
          }, ui().failed)
          .then(function () {
            loading = false;
          });
      },
      { rootMargin: "0px 0px 250px 0px" }
    );
    observer.observe(body.lastElementChild || body);
  }

  document.addEventListener("input", function (event) {
    if (!list() || !event.target.closest(".admin-category-management-list .d-filter-controls__input")) {
      return;
    }
    clearTimeout(timer);
    timer = setTimeout(reload, 400);
  });

  document.addEventListener("change", function (event) {
    if (list() && event.target.closest(".admin-category-management-list .d-filter-controls__dropdown")) {
      reload();
    }
  });

  document.addEventListener("click", function (event) {
    var el = list();
    if (!el || !event.target.closest(".admin-category-management-list .d-filter-controls__reset")) {
      return;
    }
    el.querySelector(".d-filter-controls__input").value = "";
    el.querySelector(".d-filter-controls__dropdown").value = "all";
    reload();
  });

  Discourse.onPage(function () {
    page = 0;
    loading = false;
    if (list()) {
      ui().strings("admin-category-management-strings");
      watch();
    }
  });
})();
