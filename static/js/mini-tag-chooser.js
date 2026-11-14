// select-kit's MiniTagChooser, shared by every form that picks tags: the
// composer, the review queue, the bulk actions, the title editor and the
// breadcrumbs' TagsIntersectionChooser (navigation-bar.js, shipped behind
// this file, so pages without a composer have it too).
(function () {
  "use strict";

  if (!Discourse.once("mini-tag-chooser")) {
    return;
  }

  var base = (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function svg(name, extra) {
    return (
      '<svg class="fa d-icon d-icon-' + name + " svg-icon fa-width-auto" + (extra ? " " + extra : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name + '"></use></svg>'
    );
  }

  // MiniTagChooser (a multi select over /tags/filter/search) on a
  // `.mini-tag-chooser` drawn closed by composer_view (its options in
  // data-options); its body is drawn when it opens, as select-kit draws
  // it. The composer's tags and the review queue's ReviewableFieldTags
  // (static/js/review.js, Discourse.miniTagChooser) both use it. `hooks`:
  //   categoryId()   the categoryId option ("" or null for none)
  //   minimum()      the minimum option (modifyNoSelection's label)
  //   prioritizeRecent()  the prioritizeRecentTags option and the site's
  //                  prioritize_recently_used_tags
  //   changed(added) after the selection changes (`added`: the item just
  //                  selected, the search result as returned)
  //   opened()       after it opens and draws its body
  //   failed(error)  when the search fails
  //   blocked()      the blockedTags ({id, name}), never offered
  // The selection is a list of {id, name, isNew}: a name alone is a tag
  // without an id. With the `chooser: "tag-chooser"` option it is
  // TagChooser where that differs (TagChooserRow rows, no required tag
  // group, no noTags class), and `everyTag` searches without
  // filterForInput (miniTagChooser.copy gives such a chooser its element).
  function miniTagChooser(el, hooks) {
    var selectedTags = [];
    var tagSearchTimer = null;
    var tagSearchSeq = 0;
    var tagRequired = null;
    var tagForbidden = false;
    var tagHighlighted = -1;
    var tagRows = [];
    var tagOpts = {};
    try {
      tagOpts = JSON.parse(el.dataset.options || "{}");
    } catch (_) {}
    var isTagChooser = tagOpts.chooser === "tag-chooser";

    function call(name) {
      var args = Array.prototype.slice.call(arguments, 1);
      return typeof hooks[name] === "function" ? hooks[name].apply(null, args) : undefined;
    }

    function tagLabel(key, vars) {
      var text = (tagOpts.labels || {})[key] || "";
      Object.keys(vars || {}).forEach(function (k) {
        text = text.split("%{" + k + "}").join(vars[k]);
      });
      return text;
    }

    function renderTag(name, count) {
      var html =
        '<a data-tag-name="' + escapeHtml(name.toLowerCase()) + '" class="discourse-tag ' +
        escapeHtml(tagOpts.style || "simple") + '">' + escapeHtml(name) + "</a>";
      if (count) {
        html += ' <span class="discourse-tag-count">x' + count + "</span>";
      }
      return html;
    }

    function tagHeader() {
      var summary = el.querySelector("summary");
      // modifyNoSelection: the minimum, when there is one.
      var minimum = call("minimum") || 0;
      var none =
        minimum > 0
          ? tagLabel(minimum === 1 ? "required_one" : "required_other", { count: minimum })
          : tagLabel("optional");
      var names = selectedTags.map(function (t) {
        return t.name;
      });
      var max = tagOpts.maximum;
      var caret = max && selectedTags.length >= max ? "" : svg("plus", "angle-icon");
      summary.querySelector(".select-kit-header-wrapper").innerHTML =
        '<span class="formatted-selection">' + escapeHtml(names.length ? names.join(", ") : none) + "</span>" + caret;
      summary.dataset.name = names.join(",");
      summary.dataset.value = selectedTags
        .map(function (t) {
          return t.id || t.name;
        })
        .join(",");
      if (!isTagChooser) {
        el.classList.toggle("no-tags", selectedTags.length === 0);
      }
      el.classList.toggle("has-selection", selectedTags.length > 0);
    }

    function tagFilter() {
      var input = el.querySelector(".filter-input");
      return input ? input.value : "";
    }

    // tagUtils.createContentFromInput
    function tagFromInput(input) {
      input = input
        .trim()
        .replace(/\s+/g, "-")
        .replace(/[\/\?#\[\]@!\$&'\(\)\*\+,;=%\\`^\s|\{\}"<>]+/g, "")
        .replace(/-{2,}/g, "-")
        .substring(0, tagOpts.maxLength || 20)
        .replace(/^\.+|\.+$/g, "");
      if (tagOpts.forceLowercase) {
        input = input.toLowerCase();
      }
      return input;
    }

    // tagUtils.validateCreate
    function canCreateTag(filter, results) {
      if (!tagOpts.allowAny || !filter.length) {
        return false;
      }
      if (tagOpts.maximum && selectedTags.length >= tagOpts.maximum) {
        return false;
      }
      var term = filter.replace(new RegExp(tagOpts.filterRegexp || "^$", "g"), "").trim().toLowerCase();
      if (tagForbidden) {
        return false;
      }
      var inCollection = results.some(function (r) {
        return (r.name || "").toLowerCase() === term;
      });
      var inSelection = selectedTags.some(function (t) {
        return t.name.toLowerCase() === term;
      });
      return !(inCollection || inSelection);
    }

    function drawTagBody(results, filter) {
      var body = el.querySelector(".select-kit-body");
      var input = body.querySelector(".filter-input");
      var focused = document.activeElement === input;
      var value = input ? input.value : "";
      var placeholder = tagRequired
        ? tagLabel(tagRequired.min_count === 1 ? "required_group_one" : "required_group_other", {
            count: tagRequired.min_count,
            name: tagRequired.name,
          })
        : tagLabel(tagOpts.allowAny ? "filter_placeholder_with_any" : "filter_placeholder");
      var html =
        '<div class="select-kit-filter multi-select-filter is-expanded"><input class="filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
        escapeHtml(placeholder) + '" spellcheck="false" tabindex="0" type="search"></div>';
      if (selectedTags.length) {
        html += '<div class="selected-content">';
        selectedTags.forEach(function (t, i) {
          html +=
            '<button aria-label="' + escapeHtml(tagLabel("delete_item", { name: t.name })) +
            '" class="btn btn-default selected-choice tag-choice " data-name="' + escapeHtml(t.name) +
            '" data-value="' + escapeHtml(String(t.id || t.name)) + '" data-index="' + i + '" type="button">' +
            svg("xmark") + '<span class="d-button-label">' + escapeHtml(t.name) + "</span></button>";
        });
        html += "</div>";
      }
      tagRows = [];
      var maxReached = tagOpts.maximum && selectedTags.length >= tagOpts.maximum;
      if (maxReached && filter) {
        html +=
          '<ul class="select-kit-errors-collection"><li class="select-kit-error">' +
          escapeHtml(tagLabel(tagOpts.maximum === 1 ? "max_one" : "max_other", { count: tagOpts.maximum })) +
          "</li></ul>";
      }
      if (canCreateTag(filter, results)) {
        var created = tagFromInput(filter);
        if (created) {
          tagRows.push({ create: true, name: created });
        }
      }
      results.forEach(function (r) {
        tagRows.push(r);
      });
      if (tagRows.length) {
        html += '<ul aria-live="polite" class="select-kit-collection" role="menu">';
        tagRows.forEach(function (r, i) {
          var highlighted = i === tagHighlighted ? " is-highlighted" : "";
          if (r.create) {
            html +=
              '<li aria-checked="false" role="menuitemradio" data-index="' + i + '" data-name="' + escapeHtml(r.name) +
              '" data-value="' + escapeHtml(r.name) + '" title="' + escapeHtml(r.name) + '" tabindex="0" class="select-kit-row' +
              highlighted + '"><span class="name">' + escapeHtml(tagLabel("create", { content: r.name })) + "</span></li>";
            return;
          }
          var disabled = r.disabled ? " disabled" : "";
          html +=
            '<li aria-checked="false" role="menuitemradio" data-index="' + i + '" data-name="' + escapeHtml(r.name) +
            '" data-value="' + escapeHtml(String(r.id)) + '" title="' + escapeHtml(r.title || r.name) + '" tabindex="0"' +
            (r.disabled ? ' aria-disabled="true"' : "") + ' class="select-kit-row ' + (isTagChooser ? "tag-chooser-row" : "tag-row") + highlighted + disabled + '">  ' +
            renderTag(r.name, r.count);
          if (r.disabled) {
            html += '<span class="disabled-reason">' + escapeHtml(r.title || "") + "</span>";
          } else if (r.target_tag && !isTagChooser) {
            html += '<span class="synonym-hint">' + escapeHtml(tagLabel("synonym_hint", { tag_name: r.target_tag.name })) + "</span>";
          }
          html += "</li>";
        });
        html += "</ul>";
      }
      if (filter) {
        if (!tagRows.length) {
          html += '<span class="no-content" role="alert">' + escapeHtml(tagLabel("no_content")) + "</span>";
        } else {
          html +=
            '<span class="results-count" role="alert">' +
            escapeHtml(tagLabel(tagRows.length === 1 ? "results_one" : "results_other", { count: tagRows.length })) +
            "</span>";
        }
      }
      body.innerHTML = html;
      var next = body.querySelector(".filter-input");
      next.value = value;
      if (focused || el.open) {
        next.focus();
      }
    }

    // MiniTagChooser#search and _transformJson: the results less the
    // selected tags, sorted by name when the site sorts tags so.
    function searchTags() {
      var filter = tagFilter();
      var seq = ++tagSearchSeq;
      if (tagOpts.maximum === 0) {
        drawTagBody([], filter);
        return;
      }
      var params = new URLSearchParams();
      params.append("q", filter || "");
      params.append("limit", tagOpts.limit);
      var category = call("categoryId");
      if (category) {
        params.append("categoryId", category);
      }
      // TagChooser#search: the blocked tags go with the selected ones.
      var blocked = call("blocked") || [];
      selectedTags
        .concat(blocked)
        .filter(function (t) {
          return t.id;
        })
        .slice(0, 100)
        .forEach(function (t) {
          params.append("selected_tag_ids[]", t.id);
        });
      if (!tagOpts.everyTag) {
        params.append("filterForInput", "true");
      }
      var recent = call("prioritizeRecent") && !filter;
      if (recent) {
        params.append("prioritizeRecentTags", "true");
      }
      fetch(base + "/tags/filter/search?" + params.toString(), {
        credentials: "same-origin",
        headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
      })
        .then(function (r) {
          return r.json();
        })
        .then(function (json) {
          if (seq !== tagSearchSeq || !el.open) {
            return;
          }
          tagForbidden = Boolean(json.forbidden);
          tagRequired = isTagChooser ? null : json.required_tag_group || null;
          var results = json.results || [];
          if (!recent && tagOpts.sortAlphabetically) {
            var byName = function (a, b) {
              return a.name.localeCompare(b.name);
            };
            results = results
              .filter(function (r) {
                return !r.disabled;
              })
              .sort(byName)
              .concat(
                results
                  .filter(function (r) {
                    return r.disabled;
                  })
                  .sort(byName)
              );
          }
          var names = selectedTags.concat(blocked).map(function (t) {
            return t.name;
          });
          results = results.filter(function (r) {
            return names.indexOf(r.name) < 0;
          });
          // select-kit highlights the first row once there is a filter.
          tagHighlighted = filter ? 0 : -1;
          drawTagBody(results, filter);
        })
        .catch(function (error) {
          call("failed", error);
        });
    }

    function changed(added) {
      tagHeader();
      call("changed", added);
      if (el.open) {
        searchTags();
      }
    }

    function chooseTagRow(index) {
      var row = tagRows[index];
      if (!row || row.disabled) {
        return;
      }
      var added = row.create ? { name: row.name, isNew: true } : row;
      if (tagOpts.maximum === 1 && selectedTags.length) {
        // MultiSelect#select: a single choice is replaced.
        selectedTags = [added];
      } else if (tagOpts.maximum && selectedTags.length >= tagOpts.maximum) {
        return;
      } else {
        selectedTags.push(added);
      }
      var input = el.querySelector(".filter-input");
      if (input) {
        input.value = "";
      }
      // select-kit closes once the single choice is made.
      if (tagOpts.maximum === 1) {
        el.open = false;
      }
      changed(added);
    }

    el.addEventListener("toggle", function () {
      el.classList.toggle("is-expanded", el.open);
      if (el.open) {
        tagHighlighted = -1;
        drawTagBody([], "");
        call("opened");
        searchTags();
      } else {
        el.querySelector(".select-kit-body").innerHTML = "";
      }
    });
    el.addEventListener("input", function (e) {
      if (!e.target.classList.contains("filter-input")) {
        return;
      }
      clearTimeout(tagSearchTimer);
      tagSearchTimer = setTimeout(searchTags, 200);
    });
    el.addEventListener("keydown", function (e) {
      if (!el.open || !e.target.classList.contains("filter-input")) {
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        el.open = false;
      } else if (e.key === "Enter") {
        e.preventDefault();
        chooseTagRow(tagHighlighted);
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        if (!tagRows.length) {
          return;
        }
        var from = tagHighlighted < 0 ? (e.key === "ArrowDown" ? -1 : 0) : tagHighlighted;
        tagHighlighted = (from + (e.key === "ArrowDown" ? 1 : tagRows.length - 1)) % tagRows.length;
        el.querySelectorAll(".select-kit-row").forEach(function (row, i) {
          row.classList.toggle("is-highlighted", i === tagHighlighted);
        });
      } else if (e.key === "Backspace" && tagFilter() === "" && selectedTags.length) {
        selectedTags.pop();
        changed(null);
      }
    });
    el.addEventListener("click", function (e) {
      var choice = e.target.closest(".selected-choice");
      if (choice) {
        e.preventDefault();
        selectedTags.splice(parseInt(choice.dataset.index, 10), 1);
        changed(null);
        return;
      }
      var row = e.target.closest(".select-kit-row");
      if (row) {
        e.preventDefault();
        chooseTagRow(parseInt(row.dataset.index, 10));
      }
    });
    el.addEventListener("mouseover", function (e) {
      var row = e.target.closest(".select-kit-row");
      if (row) {
        tagHighlighted = parseInt(row.dataset.index, 10);
        el.querySelectorAll(".select-kit-row").forEach(function (r) {
          r.classList.toggle("is-highlighted", r === row);
        });
      }
    });
    // A click elsewhere closes it, as select-kit's click outside does.
    document.addEventListener("click", function (e) {
      if (el.open && !el.contains(e.target)) {
        el.open = false;
      }
    });

    return {
      options: tagOpts,
      selected: function () {
        return selectedTags;
      },
      set: function (tags) {
        selectedTags = (tags || []).slice(0);
        tagHeader();
      },
      header: tagHeader,
    };
  }

  // A closed, empty copy of the composer's chooser for another form, its
  // options (and labels) overridden; null where the viewer has none.
  miniTagChooser.copy = function (overrides) {
    var source = document.querySelector("#reply-control details.mini-tag-chooser");
    if (!source) {
      return null;
    }
    var details = source.cloneNode(true);
    details.open = false;
    details.classList.remove("is-expanded");
    details.removeAttribute("category-id");
    var options = {};
    try {
      options = JSON.parse(details.dataset.options || "{}");
    } catch (_) {}
    var labels = Object.assign({}, options.labels, (overrides || {}).labels);
    Object.assign(options, overrides);
    options.labels = labels;
    if (options.chooser === "tag-chooser") {
      details.classList.remove("mini-tag-chooser", "no-tags");
      details.classList.add("tag-chooser");
    }
    details.dataset.options = JSON.stringify(options);
    var body = details.querySelector(".select-kit-body");
    body.innerHTML = "";
    body.removeAttribute("style");
    return details;
  };

  // For the review queue's ReviewableFieldTags, MoveToTopic's tags and
  // the bulk actions' ManageTagsForm.
  Discourse.miniTagChooser = miniTagChooser;
})();
