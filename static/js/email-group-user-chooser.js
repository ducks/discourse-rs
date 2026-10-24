// EmailGroupUserChooser (select-kit/components/email-group-user-chooser
// over UserChooser and MultiSelect): users, the groups the viewer may
// message and, when allowed, an email, found through /u/search/users and
// ranked as lib/user-search's organizeResults ranks them. The helpers are
// shared with the AddPmParticipants modal (add-pm-participants.js); attach()
// runs a multi-select chooser in place, as the message composer's
// recipients (composer-user-selector.gjs) use it.
(function () {
  "use strict";

  if (!window.Discourse || Discourse.emailGroupUserChooser) {
    return;
  }

  function base() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  function escapeHtml(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
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

  // lib/utilities' emailValid
  var EMAIL =
    /^[a-zA-Z0-9!#$%&'*+\/=?\^_`{|}~\-]+(?:\.[a-zA-Z0-9!#$%&'\*+\/=?\^_`{|}~\-]+)*@(?:[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?\.)+[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?$/;
  function emailValid(s) {
    return EMAIL.test(s);
  }

  // lib/user-search's skipSearch
  var IGNORE = /([\u2000-\u206F\u2E00-\u2E7F\\'!"#$%&()*,\/:;<=>?\[\]^`{|}~])|\s\s|^\s$|^[^+]*\+[^@]*$/;
  function skipSearch(term, allowEmails) {
    if (term.indexOf("@") !== -1 && !allowEmails) {
      return true;
    }
    return IGNORE.test(term);
  }

  function lowerCaseIncludes(string, term) {
    return !!(string && term && string.toLowerCase().indexOf(term.toLowerCase()) !== -1);
  }

  // lib/user-search's organizeResults, then EmailGroupUserChooser#search's
  // items: { id, name, isUser | isGroup | isEmail, avatar_template }.
  function organize(r, term, exclude, allowEmails, limit) {
    var first = ((term || "").trim().split(/\s/, 1) || [])[0];
    var users = (r.users || []).filter(function (u) {
      return exclude.indexOf(u.username) === -1;
    });
    users.forEach(function (u) {
      u.isMetadataMatch = !!first && !lowerCaseIncludes(u.username, first) && !lowerCaseIncludes(u.name, first);
    });
    var groups = (r.groups || []).filter(function (g) {
      return exclude.indexOf(g.name) === -1;
    });
    var emails = allowEmails && emailValid(term) ? [{ username: term, isEmail: true }] : [];
    var lower = function (v) {
      return v == null ? v : String(v).toLowerCase();
    };
    var seenUsers = {};
    var seenGroups = {};
    var pickUsers = function (test) {
      return users.filter(function (u) {
        if (seenUsers[u.username] || !test(u)) {
          return false;
        }
        seenUsers[u.username] = true;
        return true;
      });
    };
    var pickGroups = function (test) {
      return groups.filter(function (g) {
        if (seenGroups[g.name] || !test(g)) {
          return false;
        }
        seenGroups[g.name] = true;
        return true;
      });
    };
    var ranked = [].concat(
      pickUsers(function (u) {
        return lower(u.username) === lower(first);
      }),
      pickGroups(function (g) {
        return lower(g.name) === lower(first);
      }),
      emails,
      pickUsers(function (u) {
        return !u.isMetadataMatch && !(first && !lowerCaseIncludes(u.username, first));
      }),
      pickGroups(function (g) {
        return !(first && !lowerCaseIncludes(g.name, first));
      }),
      pickUsers(function (u) {
        return lower(u.name) === lower(first) && !u.isMetadataMatch;
      }),
      pickUsers(function (u) {
        return lowerCaseIncludes(u.name, first) && !u.isMetadataMatch;
      }),
      pickGroups(function (g) {
        return !!g.full_name && lower(g.full_name) === lower(first);
      }),
      pickGroups(function (g) {
        return !!g.full_name && lowerCaseIncludes(g.full_name, first);
      }),
      users.filter(function (u) {
        return u.isMetadataMatch;
      })
    );
    return ranked.slice(0, limit).map(function (item) {
      if (item.isEmail) {
        return { id: item.username, isEmail: true };
      }
      if (item.username) {
        return { id: item.username, name: item.name, isUser: true, avatar_template: item.avatar_template };
      }
      return { id: item.name, name: item.full_name || item.name, full_name: item.full_name, isGroup: true };
    });
  }

  function avatar(template, username) {
    var root = base();
    var src = String(template || "").replace("{size}", "24");
    if (root && src.indexOf(root) !== 0 && src.charAt(0) === "/" && src.charAt(1) !== "/") {
      src = root + src;
    }
    return (
      '<img alt="" width="24" height="24" src="' +
      escapeHtml(src) +
      '" class="avatar" title="' +
      escapeHtml(username) +
      '">'
    );
  }

  // EmailGroupUserChooserRow, in a select-kit row.
  function row(item, index, highlighted) {
    var attrs =
      'aria-checked="false" role="menuitemradio" data-index="' +
      index +
      '"' +
      (item.name ? ' data-name="' + escapeHtml(item.name) + '"' : "") +
      ' data-value="' +
      escapeHtml(item.id) +
      '"' +
      (item.name ? ' title="' + escapeHtml(item.name) + '"' : "") +
      ' tabindex="0" class="select-kit-row email-group-user-chooser-row' +
      ((highlighted === undefined ? index === 0 : highlighted) ? " is-highlighted" : "") +
      '"';
    var inner;
    if (item.isUser) {
      inner =
        avatar(item.avatar_template, item.id) +
        '<div class="email-group-user-chooser__user --username-first"><span class="identifier">' +
        escapeHtml(item.id) +
        "</span>" +
        (item.name ? ' <span class="name">' + escapeHtml(item.name) + "</span>" : "") +
        "</div>";
    } else if (item.isGroup) {
      inner =
        icon("users") +
        '<div class="email-group-user-chooser__group --group-name-first"><span class="identifier">' +
        escapeHtml(item.id) +
        '</span> <span class="name">' +
        escapeHtml(item.full_name || "") +
        "</span></div>";
    } else {
      inner = icon("envelope") + '<span class="identifier">' + escapeHtml(item.id) + "</span>";
    }
    return "<li " + attrs + ">" + inner + "</li>";
  }

  // select-kit's body below its header, as float-kit places it.
  function position(details) {
    var body = details.querySelector(".select-kit-body");
    var summary = details.querySelector("summary");
    var ui = window.FloatingUIDOM;
    if (!ui) {
      return;
    }
    body.style.position = "fixed";
    body.style.minWidth = Math.round(summary.getBoundingClientRect().width) + "px";
    body.style.top = "0px";
    body.style.left = "0px";
    ui.computePosition(summary, body, {
      placement: "bottom-start",
      strategy: "fixed",
      middleware: [ui.flip({ padding: 3 }), ui.shift({ limiter: ui.limitShift() }), ui.offset(3)],
    }).then(function (result) {
      body.style.transform = "translate(" + Math.round(result.x) + "px, " + Math.round(result.y) + "px)";
      body.style.visibility = "visible";
      body.style.pointerEvents = "auto";
    });
  }

  // The multi-select in place: `details` is the chooser's
  // `details.email-group-user-chooser` (header and empty body), `opts` its
  // strings ({ none, placeholder, noContent, resultsOne, resultsOther,
  // deleteItem }), allowEmails and onChange(values). Values are the
  // selected usernames, group names and emails in the order chosen.
  function attach(details, opts) {
    var values = [];
    var groups = {};
    var shown = [];
    var highlighted = 0;
    var choiceHighlighted = false;
    var searchTimer = null;
    var searchSeq = 0;
    var summary = details.querySelector("summary");
    var body = details.querySelector(".select-kit-body");

    function classes() {
      details.classList.toggle("is-expanded", details.open);
      details.classList.toggle("has-selection", values.length > 0);
    }

    function header() {
      var wrapper = summary.querySelector(".select-kit-header-wrapper");
      summary.dataset.name = values.join(",");
      summary.dataset.value = values.join(",");
      wrapper.innerHTML =
        '<span class="formatted-selection">' +
        escapeHtml(values.length ? values.join(", ") : opts.none) +
        "</span>" +
        icon("plus", "angle-icon");
      classes();
    }

    function filterInput() {
      return body.querySelector(".filter-input");
    }

    function renderBody(rows, term) {
      var input = filterInput();
      var value = term !== undefined ? term : input ? input.value : "";
      var html =
        '<div class="select-kit-filter multi-select-filter email-group-user-chooser-filter is-expanded"><input class="filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
        escapeHtml(opts.placeholder) +
        '" spellcheck="false" tabindex="0" type="search"></div>';
      if (values.length) {
        html +=
          '<div class="selected-content">' +
          values
            .map(function (v, i) {
              var last = choiceHighlighted && i === values.length - 1;
              return (
                '<button aria-label="' +
                escapeHtml(String(opts.deleteItem).replace("%{name}", v)) +
                '" class="btn btn-default selected-choice tag-choice ' +
                (last ? "is-highlighted" : "") +
                '" data-name="' +
                escapeHtml(v) +
                '" data-value="' +
                escapeHtml(v) +
                '" type="button">' +
                icon("xmark") +
                '<span class="d-button-label">' +
                escapeHtml(v) +
                "</span></button>"
              );
            })
            .join("") +
          "</div>";
      }
      shown = rows || [];
      if (shown.length) {
        highlighted = Math.min(highlighted, shown.length - 1);
        html +=
          '<ul aria-live="polite" class="select-kit-collection" role="menu">' +
          shown
            .map(function (item, i) {
              return row(item, i, i === highlighted);
            })
            .join("") +
          '</ul><span class="results-count" role="alert">' +
          escapeHtml((shown.length === 1 ? opts.resultsOne : opts.resultsOther).replace("%{count}", shown.length)) +
          "</span>";
      } else if (rows && value) {
        html += '<span class="no-content" role="alert">' + escapeHtml(opts.noContent) + "</span>";
      }
      body.innerHTML = html;
      var fresh = filterInput();
      fresh.value = value;
      fresh.focus();
      position(details);
    }

    // UserChooser#search through lib/user-search, debounced; what is
    // already chosen is left out.
    function search() {
      var input = filterInput();
      var term = (input ? input.value : "").replace(/^@/, "");
      clearTimeout(searchTimer);
      var seq = ++searchSeq;
      choiceHighlighted = false;
      if (term === "" || skipSearch(term, opts.allowEmails)) {
        highlighted = 0;
        renderBody(term === "" ? null : []);
        return;
      }
      searchTimer = setTimeout(function () {
        var params = new URLSearchParams({
          term: term,
          include_groups: "false",
          include_mentionable_groups: "false",
          include_messageable_groups: "true",
          topic_allowed_users: "false",
          limit: "6",
        });
        fetch(base() + "/u/search/users?" + params.toString(), {
          credentials: "same-origin",
          headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
        })
          .then(function (r) {
            return r.ok ? r.json() : {};
          })
          .then(function (json) {
            if (seq !== searchSeq || !details.open) {
              return;
            }
            highlighted = 0;
            renderBody(organize(json, term, values, opts.allowEmails, 6));
          })
          .catch(function () {});
      }, 300);
    }

    function changed() {
      header();
      if (opts.onChange) {
        opts.onChange(values.slice());
      }
    }

    function choose(index) {
      var item = shown[index];
      if (!item) {
        return;
      }
      var value = String(item.id).trim();
      if (values.indexOf(value) === -1) {
        values.push(value);
      }
      if (item.isGroup) {
        groups[value] = true;
      }
      highlighted = 0;
      renderBody(null, "");
      changed();
    }

    function remove(value) {
      values = values.filter(function (v) {
        return v !== value;
      });
      delete groups[value];
      choiceHighlighted = false;
      if (details.open) {
        renderBody(null);
      }
      changed();
    }

    details.addEventListener("toggle", function () {
      classes();
      if (!details.open) {
        clearTimeout(searchTimer);
        searchSeq++;
        body.innerHTML = "";
        choiceHighlighted = false;
        return;
      }
      highlighted = 0;
      renderBody(null);
    });
    details.addEventListener("input", function (event) {
      if (event.target.classList.contains("filter-input")) {
        search();
      }
    });
    // A key typed on the closed header opens the chooser with it.
    summary.addEventListener("keydown", function (event) {
      if (details.open || event.key.length !== 1 || event.key === " " || event.metaKey || event.ctrlKey || event.altKey) {
        return;
      }
      event.preventDefault();
      details.open = true;
      classes();
      highlighted = 0;
      renderBody(null, event.key);
      search();
    });
    details.addEventListener("keydown", function (event) {
      if (!event.target.classList.contains("filter-input")) {
        return;
      }
      var input = event.target;
      if (event.key === "Enter") {
        event.preventDefault();
        if (shown.length) {
          choose(highlighted);
        }
      } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        if (shown.length) {
          highlighted = (highlighted + (event.key === "ArrowDown" ? 1 : shown.length - 1)) % shown.length;
          body.querySelectorAll(".select-kit-row").forEach(function (li, i) {
            li.classList.toggle("is-highlighted", i === highlighted);
          });
        }
      } else if (event.key === "Backspace" && input.value === "" && values.length) {
        // The last choice is highlighted first, then removed.
        event.preventDefault();
        if (choiceHighlighted) {
          remove(values[values.length - 1]);
        } else {
          choiceHighlighted = true;
          renderBody(shown.length ? shown : null);
        }
      } else if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        details.open = false;
        summary.focus();
      }
    });
    details.addEventListener("click", function (event) {
      var li = event.target.closest(".select-kit-row");
      if (li && body.contains(li)) {
        event.preventDefault();
        choose(parseInt(li.dataset.index, 10));
        return;
      }
      var choice = event.target.closest(".selected-choice");
      if (choice && body.contains(choice)) {
        event.preventDefault();
        remove(choice.dataset.value);
      }
    });
    body.addEventListener("mousemove", function (event) {
      var li = event.target.closest(".select-kit-row");
      if (li) {
        highlighted = parseInt(li.dataset.index, 10);
        body.querySelectorAll(".select-kit-row").forEach(function (other) {
          other.classList.toggle("is-highlighted", other === li);
        });
      }
    });
    document.addEventListener("pointerdown", function (event) {
      if (details.open && details.isConnected && !details.contains(event.target)) {
        details.open = false;
      }
    });

    header();
    return {
      values: function () {
        return values.slice();
      },
      // The groups among the values (hasGroups).
      hasGroups: function () {
        return values.some(function (v) {
          return groups[v];
        });
      },
      set: function (list, groupNames) {
        values = (list || []).filter(function (v, i, all) {
          return v && all.indexOf(v) === i;
        });
        groups = {};
        (groupNames || []).forEach(function (g) {
          groups[g] = true;
        });
        details.open = false;
        header();
      },
      focus: function () {
        summary.focus();
      },
      close: function () {
        details.open = false;
      },
    };
  }

  Discourse.emailGroupUserChooser = {
    escapeHtml: escapeHtml,
    icon: icon,
    emailValid: emailValid,
    skipSearch: skipSearch,
    organize: organize,
    avatar: avatar,
    row: row,
    position: position,
    attach: attach,
  };
})();
