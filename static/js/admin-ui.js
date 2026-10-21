// What the advanced and customize admin pages' scripts share
// (src/admin_ui.rs draws their markup), and the review queue's filters:
//
//   Discourse.adminUi.t(key, args)      the page's strings (a JSON block
//                                       read by strings(id)), `%{name}`
//                                       filled from args
//   Discourse.adminUi.strings(id)       loads the strings block with that id
//   Discourse.adminUi.ajax(method, path, data)
//                                       a request as discourse/lib/ajax
//                                       makes it, resolving with the JSON;
//                                       data a plain object (form encoded)
//                                       or FormData, or { json: object }
//   Discourse.adminUi.failed(error)     popupAjaxError
//   Discourse.adminUi.unsupported(what) the page error for what is not
//                                       ported, saying so
//   Discourse.adminUi.icon(name)        dIcon's svg
//   Discourse.adminUi.menus(root)       DMenu triggers (their content in
//                                       the template after them) opening
//                                       their menu on click
//   Discourse.adminUi.subscribe(channel, fn)
//                                       the message bus's channel, from the
//                                       page's position; returns a stop()
//   Discourse.adminUi.floatBody(details)
//                                       a select-kit body floated under its
//                                       header
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.adminUi) {
    return;
  }

  var strings = {};

  function loadStrings(id) {
    var el = document.getElementById(id);
    try {
      var own = el ? JSON.parse(el.textContent) : {};
      Object.keys(own).forEach(function (k) {
        strings[k] = own[k];
      });
    } catch (_) {
      // A broken block leaves the keys showing.
    }
  }

  function t(key, args) {
    var s = strings[key];
    if (s === undefined) {
      return "[en." + key + "]";
    }
    Object.keys(args || {}).forEach(function (name) {
      s = s.split("%{" + name + "}").join(String(args[name]));
    });
    return s;
  }

  function base() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function csrf() {
    var meta = document.querySelector("meta[name='csrf-token']");
    return meta ? meta.content : "";
  }

  function ajax(method, path, data) {
    var options = {
      method: method,
      credentials: "same-origin",
      headers: {
        Accept: "application/json",
        "X-Requested-With": "XMLHttpRequest",
        "X-CSRF-Token": csrf(),
      },
    };
    if (data instanceof FormData) {
      options.body = data;
    } else if (data && data.json) {
      options.headers["Content-Type"] = "application/json";
      options.body = JSON.stringify(data.json);
    } else if (data) {
      options.headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      options.body = encode(data);
    }
    return fetch(base() + path, options).then(function (response) {
      return response.text().then(function (text) {
        var json = null;
        try {
          json = text ? JSON.parse(text) : null;
        } catch (_) {
          // Not JSON: the status decides.
        }
        if (!response.ok || (json && json.failed === "FAILED")) {
          var message =
            (json && json.errors && [].concat(json.errors).join(" ")) ||
            (json && json.message) ||
            (json && json.error) ||
            t("generic_error");
          var error = new Error(message);
          error.status = response.status;
          error.json = json;
          throw error;
        }
        return json;
      });
    });
  }

  // jQuery's param: nested objects and arrays as Rails reads them.
  function encode(data) {
    var parts = [];
    function add(key, value) {
      if (Array.isArray(value)) {
        value.forEach(function (v) {
          add(key + "[]", v);
        });
      } else if (value !== null && typeof value === "object") {
        Object.keys(value).forEach(function (k) {
          add(key + "[" + k + "]", value[k]);
        });
      } else {
        parts.push(
          encodeURIComponent(key) + "=" + encodeURIComponent(value === null || value === undefined ? "" : value)
        );
      }
    }
    Object.keys(data).forEach(function (k) {
      add(k, data[k]);
    });
    return parts.join("&");
  }

  function failed(error) {
    Discourse.dialog.alert(String((error && error.message) || error));
  }

  function unsupported(what) {
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = "Not ported yet: " + what + ".";
      error.hidden = false;
    }
  }

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  // DMenu: the trigger's content is the template beside it; its buttons
  // are handled by the page's own click listener on the menu content.
  // One listener for every page script's callback: two listeners would
  // open a menu and close it again on the same click.
  var menuCallbacks = [];
  function menus(onOpen) {
    if (onOpen) {
      menuCallbacks.push(onOpen);
    }
    if (menuCallbacks.listening) {
      return;
    }
    menuCallbacks.listening = true;
    document.addEventListener("click", function (event) {
      var trigger = event.target.closest(".fk-d-menu__trigger[data-identifier]");
      if (!trigger || !trigger.closest(".admin-contents") || !Discourse.float) {
        return;
      }
      var template = trigger.nextElementSibling;
      if (!template || template.tagName !== "TEMPLATE" || !template.classList.contains("d-menu-content")) {
        return;
      }
      // DMenu's trigger toggles its menu, expanded while it is open.
      if (trigger.getAttribute("aria-expanded") === "true") {
        Discourse.float.close(trigger.dataset.identifier);
        return;
      }
      var content = template.content.firstElementChild.cloneNode(true);
      trigger.setAttribute("aria-expanded", "true");
      trigger.classList.add("-expanded");
      var menu = Discourse.float.menu(trigger, {
        identifier: trigger.dataset.identifier,
        mainClass: trigger.dataset.identifier + "-content",
        content: content,
        onClose: function () {
          trigger.setAttribute("aria-expanded", "false");
          trigger.classList.remove("-expanded");
        },
      });
      menuCallbacks.forEach(function (fn) {
        fn(trigger, content, menu);
      });
    });
  }

  function subscribe(channel, fn) {
    if (!window.EventSource) {
      return function () {};
    }
    var meta = document.querySelector("meta[name='bus-position']");
    var url =
      base() +
      "/bus/events?channels=" +
      encodeURIComponent(channel) +
      (meta && meta.content ? "&position=" + encodeURIComponent(meta.content) : "");
    var source = new EventSource(url, { withCredentials: true });
    source.addEventListener("message", function (event) {
      try {
        fn(JSON.parse(event.data).data);
      } catch (_) {
        // A message the page can't read is skipped.
      }
    });
    return function () {
      source.close();
    };
  }

  // toasts.success: DDefaultToast for three seconds.
  function toast(text) {
    var section = document.querySelector("section.fk-d-toasts");
    if (!section) {
      section = document.createElement("section");
      section.className = "fk-d-toasts";
      document.body.appendChild(section);
    }
    var output = document.createElement("output");
    output.className = "fk-d-toast";
    output.setAttribute("role", "status");
    output.innerHTML =
      '<div class="fk-d-default-toast -success"><div class="fk-d-default-toast__icon-container">' +
      icon("check") +
      '</div><div class="fk-d-default-toast__main-container"><div class="fk-d-default-toast__texts"><div class="fk-d-default-toast__message"></div></div></div><div class="fk-d-default-toast__close-container"><button class="btn no-text btn-icon btn-transparent" type="button">' +
      icon("xmark") +
      '<span aria-hidden="true">&#8203;</span></button></div></div>';
    output.querySelector(".fk-d-default-toast__message").textContent = text;
    section.appendChild(output);
    var close = function () {
      output.remove();
    };
    output.querySelector("button").addEventListener("click", close);
    setTimeout(close, 3000);
  }

  // A select-kit body floated under its header, as select-kit places it.
  function floatBody(details) {
    var body = details.querySelector(".select-kit-body");
    var header = details.querySelector("summary");
    var fui = window.FloatingUIDOM;
    if (!fui || !body || !header) {
      return;
    }
    body.style.position = "fixed";
    body.style.top = "0";
    body.style.left = "0";
    body.style.minWidth = Math.round(header.getBoundingClientRect().width) + "px";
    fui
      .computePosition(header, body, {
        placement: "bottom-start",
        strategy: "fixed",
        middleware: [fui.flip({ padding: 3 }), fui.shift({ limiter: fui.limitShift() }), fui.offset(3)],
      })
      .then(function (result) {
        body.style.transform = "translate(" + Math.round(result.x) + "px," + Math.round(result.y) + "px)";
        body.style.visibility = "visible";
      });
  }

  function escapeHtml(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  // ComboBox (src/admin_ui.rs combo_box): its rows drawn as it opens; a
  // row chosen becomes the header's and fires `change` on the details,
  // its value in data-value.
  document.addEventListener(
    "toggle",
    function (event) {
      var details = event.target;
      if (!details.matches || !details.matches(":is(.admin-contents, .reviewable-filters) details.combo-box")) {
        return;
      }
      var template = details.querySelector("template.select-kit-rows");
      if (!template) {
        return;
      }
      details.classList.toggle("is-expanded", details.open);
      var body = details.querySelector(".select-kit-body");
      if (!details.open) {
        body.innerHTML = "";
        body.removeAttribute("style");
        return;
      }
      body.innerHTML =
        '<ul aria-live="polite" class="select-kit-collection" role="menu">' + template.innerHTML + "</ul>";
      floatBody(details);
    },
    true
  );

  document.addEventListener("click", function (event) {
    var row = event.target.closest(":is(.admin-contents, .reviewable-filters) details.combo-box .select-kit-row");
    if (!row) {
      return;
    }
    var details = row.closest("details.combo-box");
    var name = row.dataset.name;
    var value = row.dataset.value;
    var header = details.querySelector("summary");
    header.dataset.name = name;
    var choice = header.querySelector(".select-kit-selected-name");
    choice.dataset.name = name;
    choice.title = name;
    choice.querySelector(".name").textContent = name;
    details.dataset.value = value;
    details.classList.toggle("has-selection", value !== "");
    details.open = false;
    details.dispatchEvent(new Event("change", { bubbles: true }));
  });

  // EmailGroupUserChooser for one user (details.email-group-user-chooser):
  // the filter as it opens, /u/search/users as it is typed, and the row
  // chosen as the selection, its username in data-value, firing `change`
  // on the details. data-caret names the caret's icon (angle-down, then
  // angle-up while open).
  var searchTimer = null;
  function searchUsers(chooser, term) {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(function () {
      ajax("GET", "/u/search/users.json?include_groups=false&limit=6&term=" + encodeURIComponent(term))
        .then(function (result) {
          var body = chooser.querySelector(".select-kit-body");
          var list = body.querySelector(".select-kit-collection");
          if (!list) {
            list = document.createElement("div");
            list.className = "select-kit-collection";
            list.setAttribute("role", "listbox");
            body.appendChild(list);
          }
          list.innerHTML = (result.users || [])
            .map(function (u) {
              return (
                '<div class="select-kit-row email-group-user-chooser-row" role="option" tabindex="0" data-value="' +
                escapeHtml(u.username) +
                '"><img alt="" class="avatar" height="24" width="24" src="' +
                escapeHtml(String(u.avatar_template || "").replace("{size}", "24")) +
                '"><span class="identifier">' +
                escapeHtml(u.username) +
                '</span><span class="name">' +
                escapeHtml(u.name || "") +
                "</span></div>"
              );
            })
            .join("");
        })
        .catch(function () {});
    }, 300);
  }

  function setCaret(chooser) {
    var caret = chooser.dataset.caret;
    var svg = caret && chooser.querySelector("summary .angle-icon");
    if (svg) {
      var name = chooser.open ? "angle-up" : caret;
      svg.setAttribute("class", "fa d-icon d-icon-" + name + " svg-icon fa-width-auto svg-string angle-icon");
      svg.querySelector("use").setAttribute("href", "#" + name);
    }
  }

  document.addEventListener(
    "toggle",
    function (event) {
      var chooser = event.target;
      if (!chooser.matches || !chooser.matches(":is(.admin-contents, .reviewable-filters) details.email-group-user-chooser")) {
        return;
      }
      chooser.classList.toggle("is-expanded", chooser.open);
      setCaret(chooser);
      var body = chooser.querySelector(".select-kit-body");
      if (!chooser.open) {
        body.innerHTML = "";
        return;
      }
      var value = chooser.dataset.value;
      body.innerHTML =
        '<div class="select-kit-filter multi-select-filter email-group-user-chooser-filter is-expanded"><input autocapitalize="off" autocomplete="off" autocorrect="off" class="filter-input" name="filter-input-search" placeholder="' +
        escapeHtml(chooser.dataset.placeholder || "") +
        '" spellcheck="false" tabindex="0" type="search"></div>' +
        (value
          ? '<div class="selected-content"><button aria-label="' +
            escapeHtml((chooser.dataset.deleteLabel || "%{name}").replace("%{name}", value)) +
            '" class="btn btn-default selected-choice tag-choice" data-name="' +
            escapeHtml(value) +
            '" data-value="' +
            escapeHtml(value) +
            '" type="button">' +
            icon("xmark") +
            '<span class="d-button-label">' +
            escapeHtml(value) +
            "</span></button></div>"
          : "");
      body.querySelector(".filter-input").focus();
    },
    true
  );

  // The selected user's choice, clicked: no one chosen.
  document.addEventListener("click", function (event) {
    var choice = event.target.closest(":is(.admin-contents, .reviewable-filters) .email-group-user-chooser .selected-choice");
    if (!choice) {
      return;
    }
    var chooser = choice.closest(".email-group-user-chooser");
    chooser.dataset.value = "";
    chooser.classList.remove("has-selection");
    chooser.querySelector(".formatted-selection").textContent = chooser.dataset.none || "";
    chooser.querySelector("summary").dataset.name = "";
    choice.parentNode.remove();
    chooser.dispatchEvent(new Event("change", { bubbles: true }));
  });

  document.addEventListener("input", function (event) {
    var input = event.target.closest(":is(.admin-contents, .reviewable-filters) .email-group-user-chooser .filter-input");
    if (input) {
      searchUsers(input.closest(".email-group-user-chooser"), input.value);
    }
  });

  document.addEventListener("click", function (event) {
    var row = event.target.closest(":is(.admin-contents, .reviewable-filters) .email-group-user-chooser .select-kit-row");
    if (!row) {
      return;
    }
    var chooser = row.closest(".email-group-user-chooser");
    chooser.dataset.value = row.dataset.value;
    chooser.classList.add("has-selection");
    chooser.querySelector(".formatted-selection").textContent = row.dataset.value;
    chooser.querySelector("summary").dataset.name = row.dataset.value;
    chooser.open = false;
    chooser.dispatchEvent(new Event("change", { bubbles: true }));
  });

  // DMultiSelect (src/admin_ui.rs multi_select_trigger): the menu with its
  // search and results from the loader the page registers for the
  // trigger's class; a result chosen joins the selection, a selected item
  // clicked leaves it. `change` fires on the trigger.
  var multiLoaders = {};

  function multiSelect(className, load) {
    multiLoaders[className] = load;
  }

  // The selected items, { value, name } each.
  function selectionOf(trigger) {
    return Array.prototype.map.call(
      trigger.querySelectorAll(".d-multi-select-trigger__selected-item"),
      function (b) {
        return { value: b.dataset.value, name: b.title };
      }
    );
  }

  function setSelection(trigger, values) {
    var label = trigger.dataset.label || "";
    var inner = values.length
      ? '<div class="d-multi-select-trigger__selection">' +
        values
          .map(function (v) {
            return (
              '<button class="d-multi-select-trigger__selected-item" title="' +
              escapeHtml(v.name) +
              '" type="button" data-value="' +
              escapeHtml(v.value) +
              '"><span class="d-multi-select-trigger__selection-label">' +
              escapeHtml(v.name) +
              "</span>" +
              icon("xmark").replace("svg-icon", "svg-icon d-multi-select-trigger__remove-selection-icon") +
              "</button>"
            );
          })
          .join("") +
        "</div>"
      : '<span class="d-multi-select-trigger__label">' + escapeHtml(label) + "</span>";
    var expand = trigger.querySelector(".d-multi-select-trigger__expand-btn");
    Array.prototype.slice.call(trigger.children).forEach(function (child) {
      if (child !== expand) {
        child.remove();
      }
    });
    expand.insertAdjacentHTML("beforebegin", inner);
    trigger.classList.toggle("--has-selection", values.length > 0);
    trigger.dispatchEvent(new Event("change", { bubbles: true }));
  }

  function openMultiSelect(trigger) {
    var load = null;
    Object.keys(multiLoaders).forEach(function (cls) {
      if (trigger.classList.contains(cls)) {
        load = multiLoaders[cls];
      }
    });
    if (!load || !Discourse.float) {
      return;
    }
    var content = document.createElement("ul");
    content.className = "dropdown-menu d-multi-select__content";
    content.innerHTML =
      '<li class="dropdown-menu__item d-multi-select__search-container">' +
      icon("magnifying-glass") +
      '<input autocomplete="off" class="d-multi-select__search-input" placeholder="' +
      escapeHtml(t("multi_select.search")) +
      '" type="search"></li><li><hr class="dropdown-menu__divider"></li><div class="d-multi-select__results"></div>';
    trigger.setAttribute("aria-expanded", "true");
    var menu = Discourse.float.menu(trigger, {
      identifier: "d-multi-select",
      mainClass: "d-multi-select-content",
      content: content,
      onClose: function () {
        trigger.setAttribute("aria-expanded", "false");
      },
    });
    var input = content.querySelector("input");
    var results = content.querySelector(".d-multi-select__results");
    var timer = null;
    function search() {
      var chosen = selectionOf(trigger).map(function (s) {
        return s.value;
      });
      Promise.resolve(load(input.value))
        .then(function (items) {
          var available = (items || []).filter(function (i) {
            return chosen.indexOf(i.value) === -1;
          });
          results.innerHTML = available.length
            ? '<ul class="d-multi-select__search-results">' +
              available
                .map(function (i) {
                  return (
                    '<li class="dropdown-menu__item d-multi-select__result" role="button" title="' +
                    escapeHtml(i.name) +
                    '" data-value="' +
                    escapeHtml(i.value) +
                    '"><span class="d-multi-select__result-label">' +
                    // A loader may give the result's own markup (its :result block).
                    (i.resultHtml || escapeHtml(i.name)) +
                    "</span></li>"
                  );
                })
                .join("") +
              "</ul>"
            : '<div class="d-multi-select__search-no-results">' + escapeHtml(t("multi_select.no_results")) + "</div>";
        })
        .catch(function () {});
    }
    input.addEventListener("input", function () {
      clearTimeout(timer);
      timer = setTimeout(search, 250);
    });
    results.addEventListener("click", function (event) {
      var item = event.target.closest(".d-multi-select__result");
      if (item) {
        setSelection(trigger, selectionOf(trigger).concat([{ value: item.dataset.value, name: item.title }]));
        if (menu && menu.close) {
          menu.close();
        }
      }
    });
    input.focus();
    search();
  }

  document.addEventListener("click", function (event) {
    var trigger = event.target.closest(".admin-contents .d-multi-select-trigger");
    if (!trigger) {
      return;
    }
    var selected = event.target.closest(".d-multi-select-trigger__selected-item");
    if (selected) {
      setSelection(
        trigger,
        selectionOf(trigger).filter(function (v) {
          return v.value !== selected.dataset.value;
        })
      );
      return;
    }
    if (trigger.getAttribute("aria-expanded") === "true") {
      Discourse.float.close("d-multi-select");
      return;
    }
    openMultiSelect(trigger);
  });

  // DLoadMore: the sentinel in view loads the next page through the
  // page's loader, with the sentinel's data-load-more URL.
  function loadMore(onLoad) {
    var sentinel = document.querySelector(".admin-contents .load-more-sentinel[data-load-more]");
    if (!sentinel || !window.IntersectionObserver) {
      return function () {};
    }
    var busy = false;
    var observer = new IntersectionObserver(function (entries) {
      if (!entries[0].isIntersecting || busy || !sentinel.dataset.loadMore) {
        return;
      }
      busy = true;
      var url = sentinel.dataset.loadMore;
      var path = url.indexOf(base()) === 0 ? url.slice(base().length) : url;
      ajax("GET", path)
        .then(function (json) {
          var next = onLoad(json);
          if (next) {
            sentinel.dataset.loadMore = next;
          } else {
            delete sentinel.dataset.loadMore;
          }
        })
        .catch(function () {})
        .then(function () {
          busy = false;
        });
    });
    observer.observe(sentinel);
    return function () {
      observer.disconnect();
    };
  }

  // DNativeSelectOption: the chosen option is the --selected one.
  document.addEventListener("change", function (event) {
    var select = event.target.closest(".admin-contents select.d-native-select");
    if (!select) {
      return;
    }
    Array.prototype.forEach.call(select.options, function (option) {
      option.classList.toggle("--selected", option.selected);
    });
  });

  // DButton's @route: the button goes to its route's path.
  document.addEventListener("click", function (event) {
    var button = event.target.closest("button[data-href]");
    if (button && !button.disabled) {
      window.location = base() + button.dataset.href;
    }
  });

  // DTooltip: the icon's content (the template inside the trigger) shown
  // on hover or click.
  function tooltipFor(trigger) {
    var template = trigger.querySelector("template.d-tooltip-content");
    if (!template || !Discourse.float) {
      return;
    }
    Discourse.float.showTooltip(trigger, {
      identifier: trigger.dataset.identifier,
      content: template.innerHTML,
    });
  }
  document.addEventListener("click", function (event) {
    var trigger = event.target.closest(".admin-contents .fk-d-tooltip__trigger");
    if (trigger) {
      event.preventDefault();
      tooltipFor(trigger);
    }
  });

  Discourse.adminUi = {
    t: t,
    strings: loadStrings,
    ajax: ajax,
    base: base,
    failed: failed,
    unsupported: unsupported,
    icon: icon,
    menus: menus,
    subscribe: subscribe,
    toast: toast,
    escapeHtml: escapeHtml,
    floatBody: floatBody,
    multiSelect: multiSelect,
    loadMore: loadMore,
  };
})();
