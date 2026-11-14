// NavigationBar's mobile dropdown (showDropdown): the trigger in the list
// controls opens the nav items, from the template beside it, as a DMenu
// with modalForMobile, so a bottom sheet (float.js). A click in the list
// closes it, as DDropdownMenu's does.
(function () {
  "use strict";

  if (!Discourse.once("navigation-bar")) {
    return;
  }

  var open = null;

  // Discovery's CategoryDrop and TagDrop navigate using the viewer-filtered
  // API lists. Keep this separate from the composer's editing controls
  // and from TagsIntersectionChooser, a multi select (below).
  var breadcrumbSelector = ".category-breadcrumb details.select-kit.single-select";

  function closeBreadcrumb(drop, focus) {
    drop.open = false;
    drop.classList.remove("is-expanded");
    if (focus) {
      drop.querySelector("summary").focus();
    }
  }

  function selectedCategory(breadcrumb) {
    var selected = null;
    breadcrumb.querySelectorAll(".category-drop > summary").forEach(function (summary) {
      if (summary.dataset.value) {
        selected = summary.dataset.value;
      }
    });
    return selected;
  }

  async function loadBreadcrumb(drop) {
    if (drop.dataset.loading || drop.dataset.loaded) {
      return;
    }
    drop.dataset.loading = "true";
    var body = drop.querySelector(".select-kit-body");
    var base = (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
    var category = drop.classList.contains("category-drop");
    var breadcrumb = drop.closest(".category-breadcrumb");
    body.textContent = "Loading…";
    try {
      var response = await fetch(base + (category ? "/site.json" : "/tags.json"), {
        credentials: "same-origin",
        headers: { Accept: "application/json" },
      });
      if (!response.ok) {
        throw new Error("Could not load " + (category ? "categories" : "tags") + " (" + response.status + ")");
      }
      var data = await response.json();
      var rows = category ? data.categories : data.tags;
      if (!category) {
        (data.extras?.categories || []).forEach(function (group) {
          rows = rows.concat(group.tags);
        });
      }
      var seen = new Set();
      rows = rows.filter(function (row) {
        if (seen.has(row.id)) {
          return false;
        }
        seen.add(row.id);
        return true;
      });
      if (drop.classList.contains("category-breadcrumb__subcategory-selector")) {
        var parent = drop.closest("li").previousElementSibling?.querySelector("summary").dataset.value;
        rows = rows.filter(function (row) { return String(row.parent_category_id) === parent; });
      }
      rows.sort(function (a, b) { return a.name.localeCompare(b.name); });
      body.replaceChildren();
      var filter = document.createElement("input");
      filter.type = "search";
      filter.className = "filter-input";
      filter.placeholder = category ? "Filter categories" : "Filter tags";
      filter.setAttribute("aria-label", filter.placeholder);
      body.appendChild(filter);
      var list = document.createElement("ul");
      list.className = "select-kit-collection";
      body.appendChild(list);
      var categoryId = selectedCategory(breadcrumb);
      var tag = breadcrumb.querySelector(".tag-drop > summary");
      var tagPath = tag?.dataset.value ? encodeURIComponent(tag.dataset.name) + "/" + tag.dataset.value : null;
      function add(label, href, searchable) {
        var item = document.createElement("li");
        var link = document.createElement("a");
        link.className = "select-kit-row";
        link.href = base + href;
        link.textContent = label;
        if (searchable) {
          item.dataset.filter = label.toLocaleLowerCase();
        }
        item.appendChild(link);
        list.appendChild(item);
      }
      add(category ? "All categories" : "All tags", category
        ? (tagPath ? "/tag/" + tagPath : "/latest")
        : (categoryId ? "/c/-/" + categoryId : "/latest"), false);
      rows.forEach(function (row) {
        var path = encodeURIComponent(row.slug || row.name) + "/" + row.id;
        var href = category
          ? (tagPath ? "/tags/c/" + path + "/" + tagPath : "/c/" + path)
          : (categoryId ? "/tags/c/-/" + categoryId + "/" + path : "/tag/" + path);
        add(row.name, href, true);
      });
      var empty = document.createElement("li");
      empty.textContent = "No results";
      empty.hidden = rows.length > 0;
      empty.setAttribute("role", "status");
      list.appendChild(empty);
      filter.addEventListener("input", function () {
        var query = filter.value.trim().toLocaleLowerCase();
        var matches = 0;
        list.querySelectorAll("li[data-filter]").forEach(function (item) {
          item.hidden = !item.dataset.filter.includes(query);
          if (!item.hidden) { matches++; }
        });
        empty.hidden = matches > 0;
      });
      drop.dataset.loaded = "true";
      if (drop.isConnected && drop.open) {
        filter.focus();
      }
    } catch (error) {
      body.textContent = error.message + ". ";
      var retry = document.createElement("button");
      retry.type = "button";
      retry.className = "btn";
      retry.textContent = "Retry";
      retry.addEventListener("click", function (event) {
        // Loading replaces this button before the outside-click handler runs.
        event.stopPropagation();
        loadBreadcrumb(drop);
      });
      body.appendChild(retry);
    } finally {
      delete drop.dataset.loading;
    }
  }

  document.addEventListener("toggle", function (event) {
    var drop = event.target;
    if (!drop.matches?.(breadcrumbSelector)) { return; }
    drop.classList.toggle("is-expanded", drop.open);
    if (drop.open) {
      document.querySelectorAll(breadcrumbSelector + "[open]").forEach(function (other) {
        if (other !== drop) { closeBreadcrumb(other, false); }
      });
      loadBreadcrumb(drop);
    }
  }, true);
  document.addEventListener("click", function (event) {
    document.querySelectorAll(breadcrumbSelector + "[open]").forEach(function (drop) {
      if (!drop.contains(event.target) || event.target.closest("a")) {
        closeBreadcrumb(drop, false);
      }
    });
  });
  document.addEventListener("keydown", function (event) {
    var drop = event.target.closest(breadcrumbSelector);
    if (!drop?.open) { return; }
    if (event.key === "Escape") {
      event.preventDefault();
      closeBreadcrumb(drop, true);
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      var links = Array.from(drop.querySelectorAll("li:not([hidden]) > a"));
      if (!links.length) { return; }
      event.preventDefault();
      var index = links.indexOf(document.activeElement);
      index = event.key === "ArrowDown" ? (index + 1) % links.length : (index <= 0 ? links.length : index) - 1;
      links[index].focus();
    }
  });

  // DiscourseURL.routeTo: the page swapped in as a link's (hx-boost).
  function routeTo(path) {
    var base = (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
    var outlet = document.getElementById("main-outlet");
    if (!window.htmx || !outlet) {
      window.location.assign(base + path);
      return;
    }
    var link = document.createElement("a");
    link.href = base + path;
    link.hidden = true;
    outlet.appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  // BreadCrumbs' TagsIntersectionChooser on a tag intersection: the
  // MiniTagChooser (mini-tag-chooser.js, shipped in front of this file)
  // holding the page's tags, the first its main tag. Each change goes to
  // the page of what is left (onChange): the intersection led by the main
  // tag while it stays, its own list once it is alone, else the others'.
  Discourse.onPage(function () {
    var el = document.querySelector(".category-breadcrumb details.tags-intersection-chooser");
    if (!el) {
      return;
    }
    var tags = [];
    try {
      tags = JSON.parse(el.dataset.tags || "[]");
    } catch (_) {}
    var main = tags[0];
    var chooser = Discourse.miniTagChooser(el, {
      changed: function () {
        var names = chooser.selected().map(function (t) {
          return t.name;
        });
        var path = function (list) {
          return list.map(encodeURIComponent).join("/");
        };
        if (main && names.indexOf(main.name) >= 0) {
          var rest = names.filter(function (name) {
            return name !== main.name;
          });
          if (rest.length) {
            routeTo("/tags/intersection/" + path([main.name].concat(rest)));
          } else if (main.id) {
            routeTo("/tag/" + path([main.slug || main.id + "-tag", main.id]));
          } else {
            routeTo("/tag/" + path([main.name]));
          }
        } else if (names.length >= 2) {
          routeTo("/tags/intersection/" + path(names));
        } else if (names.length === 1) {
          routeTo("/tag/" + path(names));
        } else {
          routeTo("/tags");
        }
      },
    });
    chooser.set(tags);
  });

  // DNavigation's New Category button (createCategory): the new category
  // page.
  document.addEventListener("click", function (event) {
    if (!event.target.closest("#create-category")) {
      return;
    }
    var base = (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
    window.location.assign(base + "/new-category");
  });

  document.addEventListener("click", function (event) {
    var trigger = event.target.closest(".list-control-toggle-link-trigger");
    if (!trigger) {
      return;
    }
    var template = trigger.parentElement.querySelector(
      "template.list-control-toggle-link-menu"
    );
    if (!template) {
      return;
    }
    if (open && open.expanded) {
      open.close();
      return;
    }
    var content = template.content.cloneNode(true).firstElementChild;
    var menu = Discourse.float.menu(trigger, {
      identifier: "list-control-toggle-link",
      modalForMobile: true,
      autofocus: true,
      content: content,
    });
    content.addEventListener("click", function () {
      menu.close();
    });
    open = menu;
  });
})();
