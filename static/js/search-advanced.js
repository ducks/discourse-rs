// The full-page search's advanced filters (SearchAdvancedOptions): the
// toggle with its count of filters in use, and the panel it opens: the
// category chooser, the tag chooser and its "all the above tags" box, the
// "only return topics/posts" multi-select, the status combo box, the
// posted-by user chooser, the posted before/after date and the posts and
// views counts. The panel reads its values from the search term
// (didReceiveAttrs) and writes them back into it (_updateSearchTermFor*),
// drawn from #search-advanced-data (crate::search_advanced_view). Also the
// no-results "start a new topic" link (FullPageSearchController#createTopic).
(function () {
  "use strict";

  if (!Discourse.once("search-advanced")) {
    return;
  }

  var data = null;
  var searched = null;
  var submitted = 0;
  var expanded = false;
  // A sort order's search keeps the panel open (it doesn't collapse the
  // filters as Enter and the Search button do).
  var keepOpen = false;
  // The panel's DDateInput state (renderPanel).
  var dateInput = null;

  var REGEXP_BLOCKS = /(([^" \t\n\x0B\f\r]+)?(("[^"]+")?))/g;
  var REGEXP_USERNAME_PREFIX = /^(user:|@)/gi;
  var REGEXP_CATEGORY_PREFIX = /^(category:|#)/gi;
  var REGEXP_TAGS_PREFIX = /^(tags?:|#(?=[a-z0-9\-]+::tag))/gi;
  var REGEXP_IN_PREFIX = /^(in|with):/gi;
  var REGEXP_STATUS_PREFIX = /^status:/gi;
  var REGEXP_MIN_POSTS_PREFIX = /^min_posts:/gi;
  var REGEXP_MAX_POSTS_PREFIX = /^max_posts:/gi;
  var REGEXP_MIN_VIEWS_PREFIX = /^min_views:/gi;
  var REGEXP_MAX_VIEWS_PREFIX = /^max_views:/gi;
  var REGEXP_POST_TIME_PREFIX = /^(before|after):/gi;
  var REGEXP_TAGS_REPLACE = /(^(tags?:|#(?=[a-z0-9\-]+::tag))|::tag\s?$)/gi;
  var REGEXP_CATEGORY_SLUG = /^(\#[a-zA-Z0-9\-:]+)/gi;
  var REGEXP_CATEGORY_ID = /^(category:[0-9]+)/gi;
  var REGEXP_POST_TIME_WHEN = /^(before|after)/gi;
  var IN_OPTIONS_MAPPING = { images: "with" };

  function meta(name) {
    return (document.querySelector('meta[name="' + name + '"]') || {}).content || "";
  }

  function escapeHtml(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  // Handlebars' escapeExpression.
  function escapeExpression(s) {
    return String(s).replace(/[&<>"'`=]/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#x27;", "`": "&#x60;", "=": "&#x3D;" }[c];
    });
  }

  function icon(name, extra) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      " svg-icon fa-width-auto" +
      (extra ? " " + extra : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function str(key) {
    return (data && data.strings[key]) || "";
  }

  function getJson(url) {
    return fetch(meta("discourse-base-uri") + url, {
      credentials: "same-origin",
      headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
    }).then(function (r) {
      if (!r.ok) {
        throw new Error(r.status);
      }
      return r.json();
    });
  }

  function termInput() {
    return document.querySelector("input.search-query");
  }

  function searchTerm() {
    var input = termInput();
    return input ? input.value : "";
  }

  // --- reading the term (didReceiveAttrs) ----------------------------------

  function findSearchTerms() {
    var term = escapeExpression(searchTerm());
    if (!term) {
      return [];
    }
    var blocks = term.match(REGEXP_BLOCKS);
    if (!blocks) {
      return [];
    }
    return blocks.filter(function (b) {
      return b.length !== 0;
    });
  }

  function filterBlocks(regex) {
    return findSearchTerms().filter(function (b) {
      return b.search(regex) !== -1;
    });
  }

  function fresh() {
    return {
      username: null,
      category: null,
      tags: null,
      in: [],
      all_tags: false,
      status: null,
      min_posts: null,
      max_posts: null,
      min_views: null,
      max_views: null,
      time: { when: "before", days: null },
    };
  }

  function setValue(key, replaceRegEx, matchRegEx) {
    var match = filterBlocks(matchRegEx || replaceRegEx);
    var val = searched[key];
    if (match.length !== 0) {
      var userInput = match[0].replace(replaceRegEx, "").toLowerCase();
      if (val !== userInput && userInput.length) {
        searched[key] = userInput;
      }
    } else if (val && val.length !== 0) {
      searched[key] = null;
    }
  }

  function categoryBySlugs(parts) {
    var list = data.categories;
    if (parts.length > 1) {
      return list.find(function (c) {
        return c.parentSlug === parts[0] && c.slug === parts[1];
      });
    }
    return (
      list.find(function (c) {
        return !c.parentSlug && c.slug === parts[0];
      }) ||
      list.find(function (c) {
        return c.slug === parts[0];
      })
    );
  }

  function setCategory() {
    var match = filterBlocks(REGEXP_CATEGORY_PREFIX);
    if (match.length !== 0) {
      var existing = searched.category;
      var found = categoryBySlugs(match[0].replace(REGEXP_CATEGORY_PREFIX, "").split(":"));
      if ((!existing && found) || (existing && found && existing.id !== found.id)) {
        searched.category = found;
      }
    } else {
      searched.category = null;
    }
  }

  function setTags() {
    if (!data.tagging) {
      return;
    }
    var match = filterBlocks(REGEXP_TAGS_PREFIX);
    var tags = searched.tags;
    if (match.length) {
      searched.all_tags = match[0].indexOf("+") !== -1;
    }
    if (match.length !== 0) {
      var joinChar = searched.all_tags ? "+" : ",";
      var existing = Array.isArray(tags) ? tags.join(joinChar) : tags;
      var userInput = match[0].replace(REGEXP_TAGS_REPLACE, "");
      if (existing !== userInput) {
        searched.tags = userInput.split(joinChar);
        searched.showAllTags = searched.tags.length > 1;
      }
    } else if (!tags) {
      searched.tags = null;
    }
  }

  function setIn() {
    var valid = data.inOptions.map(function (o) {
      return o.value;
    });
    var selected = [];
    filterBlocks(REGEXP_IN_PREFIX).forEach(function (block) {
      var value = block.replace(REGEXP_IN_PREFIX, "").toLowerCase();
      if (value === "personal") {
        value = "messages";
      }
      if (valid.indexOf(value) !== -1 && selected.indexOf(value) === -1) {
        selected.push(value);
      }
    });
    searched.in = selected;
  }

  function statusMatch() {
    return new RegExp(
      "status:(" +
        data.statusOptions
          .map(function (o) {
            return o.value;
          })
          .join("|") +
        ")",
      "i"
    );
  }

  function setTime() {
    var match = filterBlocks(REGEXP_POST_TIME_PREFIX);
    if (match.length !== 0) {
      searched.time.when = match[0].match(REGEXP_POST_TIME_WHEN)[0].toLowerCase();
      searched.time.days = match[0].replace(REGEXP_POST_TIME_PREFIX, "");
    } else {
      searched.time.when = "before";
      searched.time.days = null;
    }
  }

  function activeFilterCount() {
    var t = searched;
    return [
      t.username,
      t.category && t.category.id,
      t.tags && t.tags.length,
      t.in && t.in.length,
      t.all_tags,
      t.status,
      t.min_posts,
      t.max_posts,
      t.min_views,
      t.max_views,
      t.time && t.time.days,
    ].filter(Boolean).length;
  }

  function sync() {
    setValue("username", REGEXP_USERNAME_PREFIX);
    setCategory();
    setTags();
    setIn();
    setValue("status", REGEXP_STATUS_PREFIX, statusMatch());
    setTime();
    setValue("min_posts", REGEXP_MIN_POSTS_PREFIX);
    setValue("max_posts", REGEXP_MAX_POSTS_PREFIX);
    setValue("min_views", REGEXP_MIN_VIEWS_PREFIX);
    setValue("max_views", REGEXP_MAX_VIEWS_PREFIX);
    var active = activeFilterCount();
    if (active < submitted) {
      submitted = active;
    }
  }

  // --- writing the term (_updateSearchTermFor*) ----------------------------

  function updateTerm(term) {
    var input = termInput();
    if (!input) {
      return;
    }
    input.value = term.trim();
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }

  function replaceOrAppend(regex, value) {
    var match = filterBlocks(regex);
    var term = searchTerm();
    if (value) {
      term = match.length !== 0 ? term.replace(match[0], value) : term + " " + value;
      updateTerm(term);
    } else if (match.length !== 0) {
      updateTerm(term.replace(match[0], ""));
    }
  }

  function updateForCategory() {
    var match = filterBlocks(REGEXP_CATEGORY_PREFIX);
    var category = searched.category;
    var term = searchTerm();
    var slugMatch = match.length !== 0 ? match[0].match(REGEXP_CATEGORY_SLUG) : null;
    var idMatch = match.length !== 0 ? match[0].match(REGEXP_CATEGORY_ID) : null;
    if (category) {
      var full = category.parentSlug ? "#" + category.parentSlug + ":" + category.slug : "#" + category.slug;
      if (slugMatch) {
        term = term.replace(slugMatch[0], full);
      } else if (idMatch) {
        term = term.replace(idMatch[0], "category:" + category.id);
      } else if (category.slug) {
        term += " " + full;
      } else {
        term += " category:" + category.id;
      }
    } else {
      if (slugMatch) {
        term = term.replace(slugMatch[0], "");
      }
      if (idMatch) {
        term = term.replace(idMatch[0], "");
      }
    }
    updateTerm(term);
  }

  function updateForTags() {
    var match = filterBlocks(REGEXP_TAGS_PREFIX);
    var tags = searched.tags;
    var term = searchTerm();
    if (tags && tags.length !== 0) {
      var joined = tags.join(searched.all_tags ? "+" : ",");
      term = match.length !== 0 ? term.replace(match[0], "tags:" + joined) : term + " tags:" + joined;
      if (tags.length > 1) {
        searched.showAllTags = true;
      }
      updateTerm(term);
    } else if (match.length !== 0) {
      updateTerm(term.replace(match[0], ""));
    }
  }

  function updateForIn() {
    var term = searchTerm();
    var values = data.inOptions.map(function (o) {
      return o.value;
    });
    values.push("personal");
    term = term.replace(new RegExp("\\s*(in|with):(" + values.join("|") + ")", "gi"), "");
    searched.in.forEach(function (filter) {
      term += " " + (IN_OPTIONS_MAPPING[filter] || "in") + ":" + filter;
    });
    updateTerm(term);
  }

  function updateForPostTime() {
    var days = searched.time.days;
    replaceOrAppend(REGEXP_POST_TIME_PREFIX, days ? searched.time.when + ":" + days : null);
  }

  // --- select-kit ---------------------------------------------------------

  function float(chooser) {
    var body = chooser.querySelector(".select-kit-body");
    var header = chooser.querySelector("summary");
    var ui = window.FloatingUIDOM;
    if (!ui || !body) {
      return;
    }
    body.style.position = "fixed";
    body.style.top = "0";
    body.style.left = "0";
    body.style.minWidth = Math.round(header.getBoundingClientRect().width) + "px";
    ui.computePosition(header, body, {
      placement: "bottom-start",
      strategy: "fixed",
      middleware: [ui.flip({ padding: 3 }), ui.shift({ limiter: ui.limitShift() }), ui.offset(3)],
    }).then(function (result) {
      body.style.transform = "translate(" + Math.round(result.x) + "px," + Math.round(result.y) + "px)";
      body.style.visibility = "visible";
    });
  }

  function comboHeader(id, name, value, label, clearable, open) {
    var filterBy = escapeHtml(str("filter_by").replace("%{name}", name));
    var valueAttr = value === undefined ? "" : ' data-value="' + escapeHtml(value) + '"';
    return (
      '<summary aria-label="' +
      filterBy +
      '" name="' +
      filterBy +
      '" data-name="' +
      escapeHtml(name) +
      '"' +
      valueAttr +
      ' tabindex="0" id="' +
      id +
      '-header" class="select-kit-header single-select-header combo-box-header"><div class="select-kit-header-wrapper"><div class="select-kit-selected-name selected-name choice" data-name="' +
      escapeHtml(name) +
      '"' +
      (value ? ' data-value="' + escapeHtml(value) + '"' : "") +
      ' title="' +
      escapeHtml(name) +
      '"><span class="name">' +
      label +
      "</span></div>" +
      (clearable
        ? '<button aria-label="' +
          escapeHtml(str("clear_input")) +
          '" class="btn no-text btn-icon btn-clear btn-transparent" type="button">' +
          icon("xmark") +
          '<span aria-hidden="true">​</span></button>'
        : "") +
      icon(open ? "angle-up" : "angle-down", "angle-icon") +
      "</div></summary>"
    );
  }

  function multiHeader(id, names, values, showPlus, userChooser) {
    var text = names.length ? names.join(", ") : str("default_header");
    var valueAttr = userChooser && !values.length ? "" : ' data-value="' + escapeHtml(values.join(",")) + '"';
    return (
      '<summary data-name="' +
      escapeHtml(names.join(",")) +
      '"' +
      valueAttr +
      ' tabindex="0" id="' +
      id +
      '-header" class="select-kit-header multi-select-header"><div class="select-kit-header-wrapper"><span class="formatted-selection">' +
      escapeHtml(text) +
      "</span>" +
      (showPlus ? icon("plus", "angle-icon") : "") +
      "</div></summary>"
    );
  }

  function details(id, classes, hasSelection, header) {
    return (
      '<details id="' +
      id +
      '" class="select-kit ' +
      classes +
      (hasSelection ? " has-selection" : "") +
      '">' +
      header +
      '<div id="' +
      id +
      '-body" class="select-kit-body"></div></details>'
    );
  }

  function controlGroup(cls, label, controls) {
    return (
      '<div class="control-group ' +
      cls +
      '"><label class="control-label">' +
      escapeHtml(label) +
      '</label><div class="controls">' +
      controls +
      "</div></div>"
    );
  }

  function optionName(list, value) {
    var found = list.find(function (o) {
      return o.value === value;
    });
    return found ? found.name : value;
  }

  function categoryControl() {
    var c = searched.category;
    var id = "search-in-category";
    var header = c
      ? comboHeader(id, c.name, String(c.id), c.label, true, false)
      : comboHeader(id, str("all_categories"), "", escapeHtml(str("all_categories")), false, false);
    return details(
      id,
      "single-select combobox combo-box category-chooser search-advanced-category-chooser",
      !!c,
      header
    );
  }

  function tagsControl() {
    var tags = (searched.tags || []).filter(function (t) {
      return t !== "";
    });
    var html = details(
      "search-with-tags",
      "multi-select tag-chooser",
      tags.length > 0,
      multiHeader("search-with-tags", tags, tags, true, false)
    );
    if (searched.showAllTags) {
      html +=
        '<section class="field"><label><input class="ember-checkbox all-tags" type="checkbox">' +
        escapeHtml(str("all_tags")) +
        "</label></section>";
    }
    return html;
  }

  function inControl() {
    var names = searched.in.map(function (v) {
      return optionName(data.inOptions, v);
    });
    return details(
      "search-in-options",
      "multi-select",
      searched.in.length > 0,
      multiHeader("search-in-options", names, searched.in, true, false)
    );
  }

  function statusControl() {
    var id = "search-status-options";
    var status = searched.status;
    var header;
    if (status) {
      var name = optionName(data.statusOptions, status);
      header = comboHeader(id, name, status, escapeHtml(name), true, false);
    } else {
      header = comboHeader(id, str("any"), undefined, escapeHtml(str("any")), false, false);
    }
    return details(id, "single-select combobox combo-box", !!status, header);
  }

  function postedByControl() {
    var u = searched.username;
    var names = u ? [u] : [];
    return details(
      "search-posted-by",
      "multi-select user-chooser",
      !!u,
      multiHeader("search-posted-by", names, names, !u, true)
    );
  }

  function postTimeControl() {
    var when = searched.time.when;
    var name = optionName(data.timeOptions, when);
    return (
      details("postTime", "single-select combobox combo-box", true, comboHeader("postTime", name, when, escapeHtml(name), false, false)) +
      '<div class="d-date-input" aria-label="' +
      escapeHtml(str("post_time_aria")) +
      '"><input id="search-post-date" class="ember-text-field date-picker" placeholder="' +
      escapeHtml(str("date_placeholder")) +
      '" aria-label="' +
      escapeHtml(str("post_time_aria")) +
      '" type="date"><div class="picker-container"></div></div>'
    );
  }

  function numberInput(id, aria, placeholder) {
    return (
      '<input id="' +
      id +
      '" class="ember-text-field input-small" aria-label="' +
      escapeHtml(str(aria)) +
      '" placeholder="' +
      escapeHtml(str(placeholder)) +
      '" type="number">'
    );
  }

  function panelHtml() {
    var html = '<div class="search-advanced-filters"><div class="search-advanced-options">';
    html += controlGroup("advanced-search-category", str("in_category"), categoryControl());
    if (data.tagging) {
      html += controlGroup("advanced-search-tags", str("with_tags"), tagsControl());
    }
    html += controlGroup("advanced-search-topics-posts", str("filters"), inControl());
    html += controlGroup("advanced-search-topic-status", str("statuses"), statusControl());
    html += controlGroup("advanced-search-posted-by", str("posted_by"), postedByControl());
    html +=
      '<div class="control-group advanced-search-posted-date"><label class="control-label">' +
      escapeHtml(str("post_time")) +
      '</label><div class="controls inline-form">' +
      postTimeControl() +
      "</div></div>";
    html +=
      '<div class="count-group control-group"><label class="control-label">' +
      escapeHtml(str("post_count")) +
      '</label><div class="controls">' +
      numberInput("search-min-post-count", "post_min_aria", "post_min") +
      icon("left-right") +
      numberInput("search-max-post-count", "post_max_aria", "post_max") +
      "</div></div>";
    html +=
      '<div class="count-group control-group"><label class="control-label">' +
      escapeHtml(str("views")) +
      '</label><div class="controls">' +
      numberInput("search-min-views", "min_views_aria", "min_views") +
      icon("left-right") +
      numberInput("search-max-views", "max_views_aria", "max_views") +
      "</div></div>";
    html += "</div>";
    if (data.mobile) {
      var label = escapeHtml(str("search_button"));
      html +=
        '<div class="second-search-button"><button aria-label="' +
        label +
        '" class="btn btn-icon-text btn-primary search-cta" type="button">' +
        icon("magnifying-glass") +
        '<span class="d-button-label">' +
        label +
        "</span></button></div>";
    }
    return html + "</div>";
  }

  // The values the inputs hold (their properties, as Ember binds them).
  function fillInputs(panel) {
    var set = function (id, value) {
      var el = panel.querySelector("#" + id);
      if (el) {
        el.value = value || "";
      }
    };
    set("search-min-post-count", searched.min_posts);
    set("search-max-post-count", searched.max_posts);
    set("search-min-views", searched.min_views);
    set("search-max-views", searched.max_views);
    set("search-post-date", searched.time.days);
    var all = panel.querySelector("input.all-tags");
    if (all) {
      all.checked = !!searched.all_tags;
    }
  }

  function renderToggle() {
    var wrapper = document.querySelector(".advanced-filters");
    if (!wrapper) {
      return;
    }
    var toggle = wrapper.querySelector(".advanced-filters__toggle");
    wrapper.classList.toggle("--is-expanded", expanded);
    toggle.setAttribute("aria-expanded", expanded ? "true" : "false");
    var badge = toggle.querySelector(".badge-notification");
    if (submitted) {
      if (!badge) {
        toggle.insertAdjacentHTML("afterbegin", '<span class="badge-notification"></span>');
        badge = toggle.querySelector(".badge-notification");
      }
      badge.textContent = String(submitted);
    } else if (badge) {
      badge.remove();
    }
    var chevron = toggle.querySelector(".d-icon");
    if (chevron) {
      chevron.outerHTML = icon(expanded ? "chevron-up" : "chevron-down");
    }
  }

  function renderPanel() {
    var wrapper = document.querySelector(".advanced-filters");
    if (!wrapper) {
      return;
    }
    var old = wrapper.querySelector(".search-advanced-filters");
    var open = old ? old.querySelector("details[open]") : null;
    var openId = open ? open.id : null;
    var filter = open ? open.querySelector(".filter-input") : null;
    var filterValue = filter ? filter.value : "";
    if (old) {
      old.remove();
    }
    if (!expanded) {
      dateInput = null;
      return;
    }
    // DDateInput lives as long as the panel: a date changed after its
    // first render sets the native picker's min (didUpdateAttrs), and
    // its --has-value follows what was typed into it.
    if (!old || !dateInput) {
      dateInput = { days: searched.time.days, updated: false, hasValue: false };
    } else if (dateInput.days !== searched.time.days) {
      dateInput.days = searched.time.days;
      dateInput.updated = true;
    }
    wrapper.insertAdjacentHTML("beforeend", panelHtml());
    var panel = wrapper.querySelector(".search-advanced-filters");
    fillInputs(panel);
    var date = panel.querySelector("#search-post-date");
    if (date) {
      date.classList.toggle("--has-value", dateInput.hasValue);
      if (dateInput.updated) {
        date.setAttribute("min", "");
      }
    }
    if (openId) {
      var again = panel.querySelector("#" + openId);
      if (again) {
        openChooser(again, filterValue);
      }
    }
  }

  function rerender() {
    renderToggle();
    renderPanel();
  }

  // --- the choosers' bodies -------------------------------------------------

  function rowLi(index, name, value, extraClass, selected, inner) {
    return (
      '<li aria-checked="' +
      (selected ? "true" : "false") +
      '" role="menuitemradio" data-index="' +
      index +
      '" data-name="' +
      escapeHtml(name) +
      '"' +
      (value === undefined ? "" : ' data-value="' + escapeHtml(value) + '"') +
      ' title="' +
      escapeHtml(name) +
      '" tabindex="0" class="select-kit-row' +
      (extraClass ? " " + extraClass : "") +
      (selected ? " is-highlighted is-selected" : "") +
      '">' +
      (inner || '<span class="name">' + escapeHtml(name) + "</span>") +
      "</li>"
    );
  }

  function filterDiv(id, multi, expandedFilter, placeholder, searchIcon) {
    if (!expandedFilter) {
      return '<div id="' + id + '-filter" class="select-kit-filter"></div>';
    }
    return (
      '<div id="' +
      id +
      '-filter" class="select-kit-filter' +
      (multi ? " multi-select-filter" : "") +
      ' is-expanded"><input class="ember-text-field filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
      escapeHtml(placeholder) +
      '" spellcheck="false" tabindex="0" type="search">' +
      (searchIcon ? icon("magnifying-glass", "filter-icon") : "") +
      "</div>"
    );
  }

  function selectedContent(items) {
    if (!items.length) {
      return "";
    }
    return (
      '<div class="selected-content">' +
      items
        .map(function (i) {
          return (
            '<button aria-label="' +
            escapeHtml(str("delete_item").replace("%{name}", i.name)) +
            '" class="btn btn-default selected-choice tag-choice " data-name="' +
            escapeHtml(i.name) +
            '" data-value="' +
            escapeHtml(i.value) +
            '" type="button">' +
            icon("xmark") +
            '<span class="d-button-label">' +
            escapeHtml(i.name) +
            "</span></button>"
          );
        })
        .join("") +
      "</div>"
    );
  }

  function collection(rows) {
    return '<ul aria-live="polite" class="select-kit-collection" role="menu">' + rows + "</ul>";
  }

  function normalize(s) {
    return String(s || "")
      .normalize("NFD")
      .replace(/[̀-ͯ]/g, "")
      .toLowerCase()
      .trim();
  }

  function setBody(chooser, html, filterValue) {
    var body = chooser.querySelector(".select-kit-body");
    body.innerHTML = html;
    var input = body.querySelector(".filter-input");
    if (input) {
      input.value = filterValue || "";
      input.focus();
    }
    float(chooser);
  }

  function categoryBody(chooser, filter) {
    var needle = normalize(filter);
    var selected = searched.category ? searched.category.id : null;
    var rows = data.categories
      .filter(function (c) {
        if (!needle) {
          return true;
        }
        var parent = c.parentSlug
          ? data.categories.find(function (p) {
              return p.slug === c.parentSlug && !p.parentSlug;
            })
          : null;
        return normalize(c.name).indexOf(needle) !== -1 || (parent && normalize(parent.name).indexOf(needle) !== -1);
      })
      .map(function (c, i) {
        var row = c.row.replace('data-name="', 'data-index="' + i + '" data-name="');
        if (c.id === selected) {
          row = row.replace('class="category-row select-kit-row"', 'aria-checked="" class="category-row select-kit-row is-selected is-highlighted"');
        }
        return row;
      })
      .join("");
    setBody(chooser, filterDiv("search-in-category", false, true, str("filter_placeholder"), true) + collection(rows), filter);
  }

  function optionsBody(chooser, id, options, selectedValue, withNone) {
    var rows = "";
    var index = 0;
    if (withNone && selectedValue) {
      rows += rowLi(index++, str("any"), undefined, "is-none none", false);
    }
    options.forEach(function (o) {
      rows += rowLi(index++, o.name, o.value, "", o.value === selectedValue);
    });
    setBody(chooser, filterDiv(id, false, false) + collection(rows));
  }

  function inBody(chooser, filter) {
    var needle = normalize(filter);
    var selected = searched.in.map(function (v) {
      return { name: optionName(data.inOptions, v), value: v };
    });
    var rows = data.inOptions
      .filter(function (o) {
        return searched.in.indexOf(o.value) === -1 && (!needle || normalize(o.name).indexOf(needle) !== -1);
      })
      .map(function (o, i) {
        return rowLi(i, o.name, o.value, "", false);
      })
      .join("");
    setBody(
      chooser,
      filterDiv("search-in-options", true, true, str("filter_placeholder")) + selectedContent(selected) + collection(rows),
      filter
    );
  }

  var tagRequest = 0;
  function tagsBody(chooser, filter) {
    var tags = (searched.tags || []).filter(function (t) {
      return t !== "";
    });
    var selected = tags.map(function (t) {
      return { name: t, value: t };
    });
    var head = filterDiv("search-with-tags", true, true, str("tags_placeholder")) + selectedContent(selected);
    setBody(chooser, head, filter);
    var params = new URLSearchParams({ q: filter || "", limit: String(data.maxTagSearchResults || 5) });
    tags.forEach(function (t) {
      params.append("selected_tags[]", t);
    });
    var mine = ++tagRequest;
    getJson("/tags/filter/search.json?" + params.toString())
      .then(function (json) {
        if (mine !== tagRequest || !chooser.open) {
          return;
        }
        var lower = tags.map(function (t) {
          return t.toLowerCase();
        });
        var results = (json.results || []).filter(function (r) {
          return lower.indexOf(String(r.name).toLowerCase()) === -1;
        });
        if (data.tagsSortAlphabetically) {
          results.sort(function (a, b) {
            return a.name.localeCompare(b.name);
          });
        }
        var rows = results
          .map(function (r, i) {
            return rowLi(
              i,
              r.name,
              r.id !== undefined ? r.id : r.name,
              "tag-chooser-row",
              false,
              '<a data-tag-name="' +
                escapeHtml(String(r.name).toLowerCase()) +
                '" class="discourse-tag ' +
                escapeHtml(data.tagStyle || "simple") +
                '">' +
                escapeHtml(r.text || r.name) +
                "</a>" +
                (r.count ? '<span class="discourse-tag-count">x' + r.count + "</span>" : "")
            );
          })
          .join("");
        var input = chooser.querySelector(".select-kit-body .filter-input");
        var value = input ? input.value : filter;
        setBody(chooser, head + (rows ? collection(rows) : ""), value);
      })
      .catch(function () {});
  }

  var USER_IGNORE = /([\u2000-\u206F\u2E00-\u2E7F\\'!"#$%&()*,\/:;<=>?\[\]^`{|}~])|\s\s|^\s$|^[^+]*\+[^@]*$/;
  var userTimer = null;
  function userBody(chooser, filter) {
    var u = searched.username;
    var selected = u ? [{ name: u, value: u }] : [];
    var head = filterDiv("search-posted-by", true, true, str("filter_placeholder")) + selectedContent(selected);
    setBody(chooser, head, filter);
    var term = (filter || "").replace(/^@/, "");
    clearTimeout(userTimer);
    if (u || term === "" || term.indexOf("@") !== -1 || USER_IGNORE.test(term)) {
      return;
    }
    userTimer = setTimeout(function () {
      var params = new URLSearchParams({ term: term, include_groups: "false", limit: "6" });
      getJson("/u/search/users.json?" + params.toString())
        .then(function (json) {
          if (!chooser.open) {
            return;
          }
          var rows = (json.users || [])
            .slice(0, 6)
            .map(function (user, i) {
              var name = user.name || user.username;
              return rowLi(
                i,
                name,
                user.username,
                "user-row" + (i === 0 ? " is-highlighted" : ""),
                false,
                '<img alt="" width="24" height="24" src="' +
                  escapeHtml(String(user.avatar_template || "").replace("{size}", "24")) +
                  '" class="avatar" title="' +
                  escapeHtml(user.username) +
                  '"><span class="username">' +
                  escapeHtml(user.username) +
                  '</span><span class="name">' +
                  escapeHtml(user.name || "") +
                  "</span>"
              );
            })
            .join("");
          var input = chooser.querySelector(".select-kit-body .filter-input");
          setBody(chooser, head + collection(rows), input ? input.value : filter);
        })
        .catch(function () {});
    }, 300);
  }

  function renderBody(chooser, filter) {
    switch (chooser.id) {
      case "search-in-category":
        categoryBody(chooser, filter);
        break;
      case "search-with-tags":
        tagsBody(chooser, filter);
        break;
      case "search-in-options":
        inBody(chooser, filter);
        break;
      case "search-status-options":
        optionsBody(chooser, "search-status-options", data.statusOptions, searched.status, true);
        break;
      case "postTime":
        optionsBody(chooser, "postTime", data.timeOptions, searched.time.when, false);
        break;
      case "search-posted-by":
        userBody(chooser, filter);
        break;
    }
  }

  function openChooser(chooser, filter) {
    chooser.open = true;
    // select-kit's classes: is-expanded before has-selection.
    var selected = chooser.classList.contains("has-selection");
    chooser.classList.remove("has-selection");
    chooser.classList.add("is-expanded");
    if (selected) {
      chooser.classList.add("has-selection");
    }
    var caret = chooser.querySelector("summary .angle-icon");
    if (caret && caret.classList.contains("d-icon-angle-down")) {
      caret.outerHTML = icon("angle-up", "angle-icon");
    }
    renderBody(chooser, filter || "");
  }

  function closeChooser(chooser) {
    chooser.open = false;
    chooser.classList.remove("is-expanded");
    var body = chooser.querySelector(".select-kit-body");
    if (body) {
      body.innerHTML = "";
      body.removeAttribute("style");
    }
    var caret = chooser.querySelector("summary .angle-icon");
    if (caret && caret.classList.contains("d-icon-angle-up")) {
      caret.outerHTML = icon("angle-down", "angle-icon");
    }
  }

  // A choice: the searched value set, then the term written from it.
  function chooseValue(chooser, row) {
    var value = row.dataset.value;
    switch (chooser.id) {
      case "search-in-category":
        searched.category =
          data.categories.find(function (c) {
            return String(c.id) === value;
          }) || null;
        closeChooser(chooser);
        updateForCategory();
        break;
      case "search-with-tags":
        searched.tags = (searched.tags || []).filter(function (t) {
          return t !== "";
        });
        searched.tags.push(row.dataset.name);
        updateForTags();
        break;
      case "search-in-options":
        searched.in = searched.in.concat([value]);
        updateForIn();
        break;
      case "search-status-options":
        searched.status = value === undefined ? null : value;
        closeChooser(chooser);
        replaceOrAppend(statusMatch(), searched.status ? "status:" + searched.status : null);
        break;
      case "postTime":
        closeChooser(chooser);
        if (value) {
          searched.time.when = value;
          updateForPostTime();
        }
        break;
      case "search-posted-by":
        searched.username = value;
        closeChooser(chooser);
        replaceOrAppend(REGEXP_USERNAME_PREFIX, value ? "@" + value : null);
        break;
    }
  }

  function unchooseValue(chooser, value) {
    switch (chooser.id) {
      case "search-with-tags":
        searched.tags = (searched.tags || []).filter(function (t) {
          return t !== value;
        });
        if (!searched.tags.length) {
          searched.tags = null;
        }
        updateForTags();
        break;
      case "search-in-options":
        searched.in = searched.in.filter(function (v) {
          return v !== value;
        });
        updateForIn();
        break;
      case "search-posted-by":
        searched.username = null;
        replaceOrAppend(REGEXP_USERNAME_PREFIX, null);
        break;
    }
  }

  function clearComboValue(chooser) {
    if (chooser.id === "search-in-category") {
      searched.category = null;
      updateForCategory();
    } else if (chooser.id === "search-status-options") {
      searched.status = null;
      replaceOrAppend(statusMatch(), null);
    }
  }

  // A change that leaves the term as it was (a time without a date) still
  // redraws the controls from the searched values.
  function choose(chooser, row) {
    chooseValue(chooser, row);
    rerender();
  }

  function unchoose(chooser, value) {
    unchooseValue(chooser, value);
    rerender();
  }

  function clearCombo(chooser) {
    clearComboValue(chooser);
    rerender();
  }

  // --- the page -------------------------------------------------------------

  Discourse.onPage(function () {
    var el = document.getElementById("search-advanced-data");
    data = null;
    if (!el) {
      return;
    }
    try {
      data = JSON.parse(el.textContent);
    } catch (e) {
      return;
    }
    searched = fresh();
    expanded = !!data.expanded || keepOpen;
    keepOpen = false;
    submitted = data.count || 0;
    sync();
    // A new model: the badge counts what this search used.
    submitted = activeFilterCount();
    rerender();
  });

  Discourse.searchAdvancedKeepOpen = function () {
    keepOpen = expanded;
  };

  document.addEventListener("input", function (event) {
    if (!data || !event.target.matches) {
      return;
    }
    if (event.target.matches("input.search-query")) {
      sync();
      rerender();
      return;
    }
    var input = event.target;
    if (input.id === "search-post-date") {
      // DDateInput#onChangeDate
      if (dateInput) {
        dateInput.hasValue = !!input.value;
      }
      input.classList.toggle("--has-value", !!input.value);
      if (input.value) {
        searched.time.days = input.value;
        updateForPostTime();
      }
      return;
    }
    if (input.closest && input.closest(".search-advanced-filters details .filter-input")) {
      renderBody(input.closest("details"), input.value);
      return;
    }
    var counts = {
      "search-min-post-count": ["min_posts", REGEXP_MIN_POSTS_PREFIX],
      "search-max-post-count": ["max_posts", REGEXP_MAX_POSTS_PREFIX],
      "search-min-views": ["min_views", REGEXP_MIN_VIEWS_PREFIX],
      "search-max-views": ["max_views", REGEXP_MAX_VIEWS_PREFIX],
    };
    var count = counts[input.id];
    if (count) {
      searched[count[0]] = input.value.length ? input.value : null;
      replaceOrAppend(count[1], searched[count[0]] ? count[0] + ":" + searched[count[0]] : null);
    }
  });

  document.addEventListener("change", function (event) {
    if (!data || !event.target.matches) {
      return;
    }
    if (event.target.matches("#search-post-date")) {
      // The native picker's onchange: onChangeWhenDate only with a date.
      if (event.target.value) {
        searched.time.days = event.target.value;
        updateForPostTime();
      }
    } else if (event.target.matches(".search-advanced-filters input.all-tags")) {
      searched.all_tags = event.target.checked;
      updateForTags();
    }
  });

  document.addEventListener("keydown", function (event) {
    var input = event.target.closest && event.target.closest(".search-advanced-filters details .filter-input");
    if (!input) {
      return;
    }
    var chooser = input.closest("details");
    if (event.key === "Enter") {
      event.preventDefault();
      var row =
        chooser.querySelector(".select-kit-collection .select-kit-row.is-highlighted") ||
        chooser.querySelector(".select-kit-collection .select-kit-row");
      if (row) {
        choose(chooser, row);
      }
    } else if (event.key === "Escape") {
      closeChooser(chooser);
    }
  });

  document.addEventListener("click", function (event) {
    var target = event.target;
    if (!target.closest || !data) {
      return;
    }
    var newTopic = target.closest(".no-results-suggestion > a");
    if (newTopic) {
      event.preventDefault();
      if (Discourse.composerOpen) {
        Discourse.composerOpen({ mode: "create-topic", draftKey: "new_topic" });
      }
      return;
    }
    if (target.closest(".advanced-filters__toggle")) {
      expanded = !expanded;
      rerender();
      return;
    }
    var panel = target.closest(".search-advanced-filters");
    // A click elsewhere closes the open chooser.
    document.querySelectorAll(".search-advanced-filters details[open]").forEach(function (chooser) {
      if (!chooser.contains(target)) {
        closeChooser(chooser);
      }
    });
    if (!panel) {
      return;
    }
    var chooser = target.closest("details.select-kit");
    if (!chooser) {
      return;
    }
    if (target.closest(".btn-clear")) {
      event.preventDefault();
      closeChooser(chooser);
      clearCombo(chooser);
      return;
    }
    var choice = target.closest(".selected-choice");
    if (choice) {
      event.preventDefault();
      unchoose(chooser, choice.dataset.value);
      return;
    }
    var row = target.closest(".select-kit-collection .select-kit-row");
    if (row) {
      event.preventDefault();
      choose(chooser, row);
      return;
    }
    if (target.closest("summary")) {
      event.preventDefault();
      if (chooser.open) {
        closeChooser(chooser);
      } else {
        openChooser(chooser, "");
      }
    }
  });
})();
