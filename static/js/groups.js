// The groups pages: what the Ember groups and group routes do in the
// browser. On /groups the name filter loads the matching groups after
// 500ms (GroupsIndexController#onFilterChanged) and the type menu filters
// by type; on /g/:name the group menu goes to the chosen group, the
// username filter reloads the members after 500ms and a column header
// sorts by it (DTableHeaderToggle). Join and Leave call the group's
// join and leave, then draw the page again; Request opens
// RequestGroupMembershipForm (also Discourse.groupMembership.request)
// and Message the composer for a message to the group.
(function () {
  "use strict";

  if (!Discourse.once("groups")) {
    return;
  }

  var MENUS = ".groups-header-filters-type, .group-dropdown";

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function csrfHeaders() {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    headers["X-Requested-With"] = "XMLHttpRequest";
    return headers;
  }

  function go(url) {
    if (!window.htmx) {
      window.location.assign(url);
      return;
    }
    var link = document.createElement("a");
    link.href = url;
    link.hidden = true;
    document.getElementById("main-outlet").appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  // The page's URL with these query params set (null drops one), keys
  // sorted.
  function withParams(changes) {
    var params = new URLSearchParams(location.search);
    Object.keys(changes).forEach(function (key) {
      if (changes[key] === null || changes[key] === "") {
        params.delete(key);
      } else {
        params.set(key, changes[key]);
      }
    });
    var keys = Array.from(params.keys()).sort();
    var query = keys
      .map(function (key) {
        return encodeURIComponent(key) + "=" + encodeURIComponent(params.get(key));
      })
      .join("&");
    return location.pathname + (query ? "?" + query : "");
  }

  function basePath() {
    var link = document.querySelector("a.group-box, .group-nav a");
    if (!link) {
      return "";
    }
    var href = link.getAttribute("href");
    var at = href.indexOf("/g/");
    return at > 0 ? href.slice(0, at) : "";
  }

  Discourse.onPage(function () {
    // GroupCard's descriptions with links, inside the card's link as
    // Ember's DOM has them.
    document.querySelectorAll("template.group-description-html").forEach(function (template) {
      template.replaceWith(template.content.cloneNode(true));
    });
    var cleanups = [];
    [
      [".groups-header-filters-name", "filter"],
      [".group-username-filter", "filter"],
    ].forEach(function (pair) {
      var input = document.querySelector(pair[0]);
      if (!input) {
        return;
      }
      var timer = null;
      var onInput = function () {
        clearTimeout(timer);
        timer = setTimeout(function () {
          var change = {};
          change[pair[1]] = input.value;
          go(withParams(change));
        }, 500);
      };
      input.addEventListener("input", onInput);
      cleanups.push(function () {
        clearTimeout(timer);
        input.removeEventListener("input", onInput);
      });
    });
    return function () {
      cleanups.forEach(function (fn) {
        fn();
      });
    };
  });

  // The menus: details elements whose body floats under them as
  // select-kit's does.
  document.addEventListener(
    "toggle",
    function (event) {
      var menu = event.target;
      if (!menu.classList || !menu.matches(MENUS)) {
        return;
      }
      var open = menu.open;
      menu.classList.toggle("is-expanded", open);
      var caret = menu.querySelector(".angle-icon");
      var closed = menu.matches(".group-dropdown") ? "angle-right" : "angle-down";
      if (caret) {
        caret.outerHTML = icon(open ? "angle-up" : closed).replace(
          "svg-icon fa-width-auto",
          "svg-icon fa-width-auto angle-icon"
        );
      }
      var header = menu.querySelector("summary");
      var body = menu.querySelector(".select-kit-body");
      if (!open || !body || !window.FloatingUIDOM) {
        return;
      }
      var ui = window.FloatingUIDOM;
      body.style.position = "fixed";
      body.style.top = "0";
      body.style.left = "0";
      body.style.minWidth = Math.max(Math.round(header.getBoundingClientRect().width), 220) + "px";
      ui.computePosition(header, body, {
        placement: "bottom-start",
        strategy: "fixed",
        middleware: [ui.flip(), ui.shift({ limiter: ui.limitShift() }), ui.offset(3)],
      }).then(function (result) {
        body.style.transform = "translate(" + Math.round(result.x) + "px," + Math.round(result.y) + "px)";
      });
    },
    true
  );

  function membership(button, method, path) {
    button.disabled = true;
    fetch(basePath() + "/groups/" + button.dataset.groupId + path, {
      method: method,
      credentials: "same-origin",
      headers: csrfHeaders(),
    })
      .then(function (r) {
        if (r.ok) {
          go(location.pathname + location.search);
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

  // The page's #group-membership-strings (groups::view), a member's.
  function membershipStrings() {
    var el = document.getElementById("group-membership-strings");
    try {
      return el ? JSON.parse(el.textContent) : {};
    } catch (e) {
      return {};
    }
  }

  // popupAjaxError: the response's errors in the dialog, which a page
  // without one loads first (as composer.js does).
  var dialogLoading = null;
  function popupError(message, strings) {
    if (Discourse.dialog) {
      Discourse.dialog.popupAjaxError(message);
      return;
    }
    if (!dialogLoading) {
      dialogLoading = new Promise(function (resolve, reject) {
        if (!document.getElementById("dialog-strings")) {
          var block = document.createElement("script");
          block.type = "application/json";
          block.id = "dialog-strings";
          block.textContent = strings.dialogStrings || "{}";
          document.head.appendChild(block);
        }
        var script = document.createElement("script");
        script.src = strings.dialogSrc;
        script.onload = resolve;
        script.onerror = reject;
        document.head.appendChild(script);
      });
    }
    dialogLoading.then(
      function () {
        Discourse.dialog.popupAjaxError(message);
      },
      function () {
        dialogLoading = null;
        window.alert(message);
      }
    );
  }

  function loggedIn() {
    return !!document.body.getAttribute("hx-headers");
  }

  // GroupMembershipButton#showRequestMembershipForm: the login for a
  // visitor, else RequestGroupMembershipForm
  // (modal/request-group-membership-form.gjs) for the group ({ name,
  // membership_request_template }): the reason starts as the group's
  // template, Submit is disabled while it is empty or the request is out,
  // and Group#requestMembership's answer is the message's topic, which it
  // goes to.
  function requestMembership(group) {
    if (!loggedIn()) {
      go(basePath() + "/login");
      return;
    }
    var strings = membershipStrings();
    var reason = group.membership_request_template;
    var loading = false;
    var textarea, submit, spinner;

    function sync() {
      submit.disabled = loading || !reason;
      spinner.className = "loading-container" + (loading ? " visible" : "") + " inline-spinner";
      spinner.innerHTML = loading ? '<div class="spinner small"></div>' : "";
      if (loading) {
        spinner.setAttribute("data-loading", "");
      } else {
        spinner.removeAttribute("data-loading");
      }
    }

    var close = Discourse.modal.show({
      className: "request-group-membership-form",
      title: (strings.title || "").split("%{group_name}").join(group.name),
      body: function (body) {
        var control = document.createElement("div");
        control.className = "control-group";
        var label = document.createElement("label");
        label.textContent = strings.reason || "";
        var wrap = document.createElement("div");
        wrap.className = "expanding-text-area";
        textarea = document.createElement("textarea");
        textarea.className = "--expandable";
        textarea.setAttribute("maxlength", "5000");
        if (reason !== null && reason !== undefined) {
          textarea.value = reason;
          wrap.setAttribute("data-replicated-value", reason);
        }
        textarea.addEventListener("input", function () {
          reason = textarea.value;
          wrap.setAttribute("data-replicated-value", reason);
          sync();
        });
        wrap.appendChild(textarea);
        control.appendChild(label);
        control.appendChild(wrap);
        body.appendChild(control);
      },
      footer: function (footer, closeModal) {
        submit = document.createElement("button");
        submit.type = "button";
        submit.className = "btn btn-text btn-primary";
        submit.innerHTML = '<span class="d-button-label"></span>';
        submit.firstChild.textContent = strings.submit || "";
        var cancel = document.createElement("button");
        cancel.type = "button";
        cancel.className = "btn btn-text btn-flat d-modal-cancel";
        cancel.innerHTML = '<span class="d-button-label"></span>';
        cancel.firstChild.textContent = strings.cancel || "";
        cancel.addEventListener("click", function () {
          closeModal();
        });
        spinner = document.createElement("div");
        submit.addEventListener("click", send);
        footer.appendChild(submit);
        footer.appendChild(cancel);
        footer.appendChild(spinner);
        sync();
      },
    });

    function send() {
      if (submit.disabled) {
        return;
      }
      loading = true;
      sync();
      var headers = csrfHeaders();
      headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      fetch(basePath() + "/groups/" + encodeURIComponent(group.name) + "/request_membership.json", {
        method: "POST",
        credentials: "same-origin",
        headers: headers,
        body: "reason=" + encodeURIComponent(reason),
      })
        .then(function (r) {
          return r
            .json()
            .catch(function () {
              return {};
            })
            .then(function (json) {
              if (r.ok) {
                // DiscourseURL.routeTo(result.relative_url); the modal
                // goes as the route changes.
                close();
                go(json.relative_url);
                return;
              }
              // extractError
              var message =
                json.errors && json.errors.length
                  ? json.errors.join("\n")
                  : json.error || strings.generic_error || r.statusText;
              popupError(message, strings);
            });
        })
        .catch(function () {
          popupError(strings.generic_error || "", strings);
        })
        .finally(function () {
          loading = false;
          sync();
        });
    }
  }

  // For the group card (GroupCardContents has the same button).
  Discourse.groupMembership = { request: requestMembership };

  // GroupController#messageGroup: composer.openNewMessage to the group.
  // Only a member is shown the button, and a member's page has the
  // composer.
  function messageGroup(name) {
    if (!window.composer) {
      return;
    }
    window.composer.open({ mode: "private-message", recipients: [name], hasGroups: true });
  }

  document.addEventListener("click", function (event) {
    var typeRow = event.target.closest(".groups-header-filters-type .select-kit-row");
    if (typeRow) {
      go(withParams({ type: typeRow.dataset.value || null }));
      return;
    }
    var groupRow = event.target.closest(".group-dropdown .select-kit-row");
    if (groupRow) {
      go(basePath() + "/g/" + encodeURIComponent(groupRow.dataset.value));
      return;
    }
    document.querySelectorAll(".groups-header-filters-type[open], .group-dropdown[open]").forEach(function (menu) {
      if (!menu.contains(event.target)) {
        menu.open = false;
      }
    });
    var join = event.target.closest(".group-index-join");
    if (join) {
      event.preventDefault();
      if (!document.body.getAttribute("hx-headers")) {
        go(basePath() + "/login");
        return;
      }
      membership(join, "PUT", "/join.json");
      return;
    }
    var leave = event.target.closest(".group-index-leave");
    if (leave) {
      event.preventDefault();
      membership(leave, "DELETE", "/leave.json");
      return;
    }
    var request = event.target.closest(".group-index-request");
    if (request) {
      event.preventDefault();
      requestMembership({
        name: request.dataset.groupName,
        membership_request_template: request.dataset.membershipRequestTemplate,
      });
      return;
    }
    var message = event.target.closest(".group-details-button .group-message-button");
    if (message) {
      event.preventDefault();
      messageGroup(message.dataset.groupName);
      return;
    }
    // DTableHeaderToggle#toggleProperties: ascending only when it was
    // descending on this field.
    var header = event.target.closest(".group-members .directory-table__column-header.sortable");
    if (header) {
      var params = new URLSearchParams(location.search);
      var field = header.dataset.order;
      var asc = params.get("order") === field && params.get("asc") === "false";
      go(withParams({ order: field, asc: asc ? null : "false" }));
    }
  });

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      document.querySelectorAll(".groups-header-filters-type[open], .group-dropdown[open]").forEach(function (menu) {
        menu.open = false;
      });
    }
    if (event.key === "Enter" && event.target.closest) {
      var target = event.target.closest(".group-members .header-contents, .groups-header-filters-type .select-kit-row, .group-dropdown .select-kit-row");
      if (target) {
        target.click();
      }
    }
  });
})();
